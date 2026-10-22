import { api, ApiError, unreachable, type WorkspaceNode, type NodeSummary } from './api'
import { isTouchDevice } from './util'
import { Tree } from './state/tree.svelte'
import { Tiles, type Tile, type TileKind, type Draft } from './state/tiles.svelte'
import { Drafts } from './state/drafts.svelte'
import { Cards } from './state/cards.svelte'
import { Diary } from './state/diary.svelte'
import { Scratchpad } from './state/scratch.svelte'
import { Sheets } from './state/sheets.svelte'
import { Vault } from './state/vault.svelte'
import { Prompts } from './state/prompts.svelte'

export { Tile, type Draft, type TileKind, type View } from './state/tiles.svelte'
export type { Sheet, InfoSection } from './state/sheets.svelte'

export type Pane = 'list' | 'document'
export type Flash = { text: string; kind: 'info' | 'error' } | null

/**
 * The whole application state, as one object: the keyboard layer needs to
 * see everything at once to decide what a key means. Inside, it is made of
 * parts, one for each thing the app keeps — the tree, the tiles, the drafts,
 * the cards, the diary, the scratchpad, the sheets, the vault and the prompts —
 * which reach each other through it. What is left here is what belongs to
 * no one part: the session, the status line, and what has the focus.
 */
export class Workspace {
  readonly tree: Tree = new Tree(this)
  readonly tiles: Tiles = new Tiles(this)
  readonly drafts: Drafts = new Drafts(this)
  readonly cards: Cards = new Cards(this)
  readonly diary: Diary = new Diary(this)
  readonly scratch: Scratchpad = new Scratchpad(this)
  readonly sheets: Sheets = new Sheets(this)
  readonly vault: Vault = new Vault(this)
  readonly prompts: Prompts = new Prompts(this)

  user = $state<string | null>(null)
  booting = $state(true)

  /** The focused tile; `open`, `draft`, `editing` … below are its. */
  get tile(): Tile {
    return this.tiles.current
  }

  get open() {
    return this.tile.open
  }
  set open(node: WorkspaceNode | null) {
    this.tile.open = node
  }
  get draft() {
    return this.tile.draft
  }
  set draft(draft: Draft) {
    this.tile.draft = draft
  }
  get editing() {
    return this.tile.editing
  }
  set editing(editing: boolean) {
    this.tile.editing = editing
  }
  get enterInsert() {
    return this.tile.enterInsert
  }
  set enterInsert(insert: boolean) {
    this.tile.enterInsert = insert
  }
  get dirty() {
    return this.tile.dirty
  }
  set dirty(dirty: boolean) {
    this.tile.dirty = dirty
  }
  get vimMode() {
    return this.tile.vimMode
  }
  set vimMode(mode: string) {
    this.tile.vimMode = mode
  }

  /** What has the keys: the list, or the kind of the focused tile. */
  get focus(): 'list' | TileKind {
    return this.pane === 'list' || this.tile.empty ? 'list' : this.tile.kind
  }

  pane = $state<Pane>('list')
  flash = $state<Flash>(null)

  /** `f`: the focused pane takes the whole porthole, like a fullscreen window. */
  fullscreen = $state(false)
  /** +1 when the last space switch went right along the bar, -1 when left. */
  switchDirection = $state(1)

  /** Modal editing on a keyboard; a phone's editor is a plain text box. */
  vimEnabled = $state((localStorage.getItem('diary:vim') ?? (isTouchDevice ? 'off' : 'on')) !== 'off')

  /** What the login screen has to say: why it is showing, if not for a first visit. */
  notice = $state('')

  /** Mirror vim yanks and deletes into the system clipboard, like `set clipboard=unnamed`. */
  clipboard = $state(localStorage.getItem('diary:clipboard') !== 'off')

  /**
   * What a verb acts on: the open document while the document pane has focus,
   * the row under the cursor while the list does. Walking the list with a
   * document still open must not make `dd` delete the document.
   */
  get target(): NodeSummary | undefined {
    return this.open && this.pane === 'document' ? this.open : this.tree.selected
  }

  /**
   * A line in the status bar. Information fades after a few seconds, unless
   * `keep`: for what is shown only once, and may have to be copied by hand.
   */
  say(text: string, kind: 'info' | 'error' = 'info', keep = false) {
    this.flash = { text, kind }
    if (kind === 'info' && !keep) {
      const current = this.flash
      setTimeout(() => {
        if (this.flash === current) this.flash = null
      }, 4000)
    }
  }

  #watching = false

  async boot() {
    if (!this.#watching) {
      this.#watching = true
      this.drafts.watchConnection()
    }
    try {
      const { user } = await api.me()
      this.user = user
      this.notice = ''
      await this.tree.loadSpaces(Workspace.addressedPath())
      await this.cards.loadAgenda()
      await this.vault.load()
      await this.cards.openAgendaIfDue()
    } catch (error) {
      this.user = null
      // Down is not the same as logged out: say so, and keep asking.
      if (unreachable(error)) {
        this.notice = 'the server cannot be reached — trying again'
        setTimeout(() => void this.boot(), 5000)
      }
    } finally {
      this.booting = false
    }
  }

  async login(username: string, password: string) {
    const { user } = await api.login(username, password)
    this.user = user
    this.notice = ''
    // The session ran out under an open workspace: everything is still here,
    // the draft included, so pick up where it stopped rather than start over.
    if (this.tree.spaces.length > 0) {
      await this.tree.refresh()
      for (const tile of this.tiles.all) {
        if (tile.open && tile.dirty) await this.guard(() => this.drafts.save({ quiet: true, tile }))
      }
      if (this.drafts.writing?.dirty) await this.guard(() => this.drafts.saveWriting())
      return
    }
    await this.tree.loadSpaces(Workspace.addressedPath())
    await this.cards.loadAgenda()
    await this.vault.load()
    await this.cards.openAgendaIfDue()
  }

  async logout(everywhere = false) {
    await api.logout(everywhere)
    this.user = null
    this.vault.status = null
    this.tree.spaces = []
    this.tree.nodes = []
    for (const tile of this.tiles.all) tile.clear()
    this.tiles.prune()
  }

  /** The slug path in the address bar, if the page was opened on a `/n/...` URL. */
  static addressedPath(): string | null {
    return location.pathname.startsWith('/n/')
      ? decodeURIComponent(location.pathname.slice(3)).replace(/\/$/, '')
      : null
  }

  async copy(text: string) {
    try {
      await navigator.clipboard.writeText(text)
      return true
    } catch {
      return false
    }
  }

  setVim(enabled: boolean) {
    this.vimEnabled = enabled
    localStorage.setItem('diary:vim', enabled ? 'on' : 'off')
  }

  setClipboard(enabled: boolean) {
    this.clipboard = enabled
    localStorage.setItem('diary:clipboard', enabled ? 'on' : 'off')
  }

  /** Surface a failed API call in the status line instead of the console. True when it went through. */
  async guard(action: () => Promise<unknown>): Promise<boolean> {
    try {
      await action()
      return true
    } catch (error) {
      if (error instanceof ApiError && error.status === 401) {
        this.user = null
        this.notice = 'the session ran out — log in again, and carry on where you were'
        this.say('session expired — log in again', 'error')
      } else if (error instanceof ApiError && error.status === 423) {
        // It locked itself while nobody was looking.
        if (this.vault.status) this.vault.status = { ...this.vault.status, unlocked: false, locks_in: null }
        this.say(error.message, 'error')
      } else {
        this.say(error instanceof Error ? error.message : String(error), 'error')
      }
      return false
    }
  }
}

export const workspace = new Workspace()
