import {
  api,
  ApiError,
  isContainer,
  unreachable,
  type Agenda,
  type AgendaCard,
  type Board,
  type Card,
  type Crumb,
  type WorkspaceNode,
  type MediaFile,
  type NodeSummary,
  type SpaceOverview,
  type VaultStatus,
} from './api'
import { formatBytes, isTouchDevice, sealedNote, type Heading } from './util'

export type Pane = 'list' | 'document'
/** The tree of documents, or the board of the space being browsed. */
export type View = 'tree' | 'board'
export type Flash = { text: string; kind: 'info' | 'error' } | null

/**
 * Where a space was left: the folder, the open documents, the row, the pane.
 * `openId` is the focused tile's, all a spot held before there were tiles.
 */
type Spot = {
  parentId: number
  openId: number | null
  cursor: number
  pane: Pane
  open?: number[]
  focused?: number
}

export type Draft = { name: string; body: string; created_at: number }

let tiles = 0

/**
 * One document on screen, and everything that belongs to it rather than to
 * the workspace: what it shows, the draft being written, whether that is
 * saved, and the timer that will save it.
 */
export class Tile {
  readonly id = ++tiles
  open = $state<WorkspaceNode | null>(null)
  draft = $state<Draft>({ name: '', body: '', created_at: 0 })
  editing = $state(false)
  /** Set when the editor should open in insert mode rather than normal mode. */
  enterInsert = $state(false)
  dirty = $state(false)
  /** The editor's vim mode, for the status line while this tile has focus. */
  vimMode = $state('normal')
  /*
   * Set when a card's document was opened from the board, so closing the
   * document goes back to the board — on the same card — rather than leaving
   * you in a tree you never asked for.
   */
  returnTo = $state<'board' | null>(null)
  /*
   * The document `o` just made. If it is left with neither a name nor a word
   * in it, it is removed on the way out rather than left as an empty row.
   */
  fresh: number | null = null
  /** Pending debounced write, if the draft has changed since the last save. */
  autosave: ReturnType<typeof setTimeout> | null = null
  /** Saves that failed for want of a server wait this long before trying again. */
  retryDelay = 0

  stopAutosave() {
    if (this.autosave) clearTimeout(this.autosave)
    this.autosave = null
  }

  /** Put the tile back to showing nothing. Where it goes back to stays. */
  clear() {
    this.stopAutosave()
    this.open = null
    this.editing = false
    this.dirty = false
    this.fresh = null
  }
}

/**
 * The whole application state. It is deliberately one object: the keyboard
 * layer needs to see everything at once to decide what a key means.
 */
class Workspace {
  user = $state<string | null>(null)
  booting = $state(true)

  /** The top level: diary, work, uni, whatever else gets made. */
  spaces = $state<NodeSummary[]>([])
  /** The space being browsed, and the folder inside it the list is showing. */
  spaceId = $state<number | null>(null)
  parentId = $state<number | null>(null)
  /** Ancestors of the current folder, the space first and the folder last. */
  path = $state<Crumb[]>([])
  /** The current folder is sealed: uploads into it go into the vault. */
  parentSealed = $state(false)

  /** Children of the current folder, or the results of a search. */
  nodes = $state<NodeSummary[]>([])
  cursor = $state(0)
  query = $state('')

  /*
   * The documents on screen. There is always at least one tile, empty when
   * nothing is open; the names below (`open`, `draft`, `editing` …) are the
   * focused tile's, so most of the app reads one document as it always did.
   */
  tiles = $state<Tile[]>([new Tile()])
  focused = $state(0)

  get tile(): Tile {
    return this.tiles[this.focused] ?? this.tiles[0]
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
  get returnTo() {
    return this.tile.returnTo
  }
  set returnTo(to: 'board' | null) {
    this.tile.returnTo = to
  }
  get vimMode() {
    return this.tile.vimMode
  }
  set vimMode(mode: string) {
    this.tile.vimMode = mode
  }

  /** Every tile showing `id`. */
  tilesWith(id: number): Tile[] {
    return this.tiles.filter((tile) => tile.open?.id === id)
  }

  /**
   * Focus a tile, and follow it in the list and the address bar, as opening
   * its document did.
   */
  focusTile(index: number) {
    const tile = this.tiles[index]
    if (!tile) return
    this.focused = index
    if (!tile.open) return
    this.pane = 'document'
    const row = this.nodes.findIndex((n) => n.id === tile.open?.id)
    if (row >= 0) this.cursor = row
    this.#syncUrl(tile.open.path)
  }

  /**
   * Tiles with nothing in them go, as long as one is left: a closed document
   * closes its tile, as `:q` closes a window in vim.
   */
  prune() {
    const current = this.tile
    if (this.tiles.length > 1) {
      const kept = this.tiles.filter((tile) => tile.open)
      this.tiles = kept.length > 0 ? kept : [current]
      const at = this.tiles.indexOf(current)
      this.focused = at >= 0 ? at : Math.min(this.focused, this.tiles.length - 1)
    }
    if (!this.open) this.pane = 'list'
  }

  /** `:close`: the focused tile goes, or with only one, its document. */
  closeTile() {
    this.tile.clear()
    this.prune()
    if (this.open) this.focusTile(this.focused)
  }

  /**
   * `L` / Shift-Enter: the row under the cursor in a tile of its own, next to
   * what is open, rather than in its place.
   */
  async openBeside() {
    const selected = this.selected
    if (!selected) return
    if (isContainer(selected)) return this.say(`${selected.slug} is a ${selected.kind} — l goes into it`, 'error')
    const shown = this.tiles.findIndex((tile) => tile.open?.id === selected.id)
    if (shown >= 0) return this.focusTile(shown)
    if (this.open) {
      this.tiles.push(new Tile())
      this.focused = this.tiles.length - 1
    }
    try {
      await this.openSelected()
    } finally {
      this.prune()
    }
  }

  /** `:only`: every other tile goes, after their drafts are written. */
  async onlyTile() {
    const others = this.tiles.filter((tile) => tile !== this.tile)
    for (const tile of others) await this.flushTile(tile)
    const unsaved = others.filter((tile) => tile.dirty)
    if (unsaved.length > 0) {
      const names = unsaved.map((tile) => tile.open?.slug).join(', ')
      if (!(await this.ask(`${names} could not be written — close anyway? the drafts stay in this browser`))) return
    }
    for (const tile of others) tile.stopAutosave()
    this.tiles = [this.tile]
    this.focused = 0
  }

  /** The board of the current space, loaded only while it is being looked at. */
  view = $state<View>('tree')
  /** The board as the server sent it. */
  rawBoard = $state<Board | null>(null)
  /*
   * The board as it is walked and drawn: the done list keeps only what was
   * finished this past week unless `z` asks for the rest, so the cursor
   * indices and what is on screen are always the same thing.
   */
  board = $derived.by((): Board | null => {
    const raw = this.rawBoard
    if (!raw || this.showOldDone) return raw
    const since = Math.floor(Date.now() / 1000) - Workspace.RECENT_DONE_DAYS * 86400
    const last = raw.lists.length - 1
    return {
      ...raw,
      lists: raw.lists.map((list, index) =>
        index === last && index > 0
          ? { ...list, cards: list.cards.filter((card) => (card.done_at ?? 0) >= since) }
          : list,
      ),
    }
  })
  listCursor = $state(0)
  cardCursor = $state(0)
  /** `z` on the board: show done cards older than a week too. */
  showOldDone = $state(false)
  /*
   * The nodes picked up by `m`, waiting for the `p` that drops them into
   * whatever folder the list is showing then. A move is two keystrokes apart
   * in time because the destination is somewhere else in the tree — there is
   * no way to name it and be somewhere else at once — so the marks stay put
   * while the list walks, and can be gathered from several folders.
   */
  marked = $state.raw<NodeSummary[]>([])

  pane = $state<Pane>('list')
  flash = $state<Flash>(null)

  overlay = $state<'none' | 'help' | 'media' | 'overview' | 'agenda' | 'actions' | 'verify' | 'outline'>('none')
  /** `gO`: the headings of what is on screen, to jump between from the keyboard. */
  outline = $state.raw<Heading[]>([])
  outlineCursor = $state(0)

  /*
   * What is due or in hand on every board, kept loaded all the time: the bar
   * counts from it, so what is late is in sight without going to look.
   */
  agenda = $state<Agenda>({ cards: [], diary_today: null })
  agendaCursor = $state(0)

  /** A search across every space rather than the one being browsed — Ctrl-P. */
  globalSearch = $state(false)

  /*
   * A `[y/N]` question in the command line's place, instead of the browser's
   * confirm(): it names the thing about to go, and it is answered from the
   * keyboard without a modal the rest of the page cannot see past.
   */
  prompt = $state<{ text: string; resolve: (yes: boolean) => void } | null>(null)
  /** A password asked for in the same place, typed into a masked field. */
  secret = $state.raw<{ text: string; resolve: (value: string | null) => void } | null>(null)
  /** The vault, as last heard from the server; null until asked. */
  vault = $state<VaultStatus | null>(null)
  media = $state<MediaFile[]>([])
  mediaCursor = $state(0)
  overview = $state<SpaceOverview[]>([])
  overviewCursor = $state(0)

  /** `f`: the focused pane takes the whole porthole, like a fullscreen window. */
  fullscreen = $state(false)
  /** +1 when the last space switch went right along the bar, -1 when left. */
  switchDirection = $state(1)

  /*
   * Spaces behave like the desktop's workspaces: each keeps the folder and the
   * document it was left on, and coming back finds them where they were.
   */
  #spots: Record<number, Spot> = JSON.parse(localStorage.getItem('diary:spots') ?? '{}')

  /** Modal editing on a keyboard; a phone's editor is a plain text box. */
  vimEnabled = $state((localStorage.getItem('diary:vim') ?? (isTouchDevice ? 'off' : 'on')) !== 'off')

  /** What the login screen has to say: why it is showing, if not for a first visit. */
  notice = $state('')

  /** Mirror vim yanks and deletes into the system clipboard, like `set clipboard=unnamed`. */
  clipboard = $state(localStorage.getItem('diary:clipboard') !== 'off')

  get selected(): NodeSummary | undefined {
    return this.nodes[this.cursor]
  }

  /**
   * What a verb acts on: the open document while the document pane has focus,
   * the row under the cursor while the list does. Walking the list with a
   * document still open must not make `dd` delete the document.
   */
  get target(): NodeSummary | undefined {
    return this.open && this.pane === 'document' ? this.open : this.selected
  }

  get space(): NodeSummary | undefined {
    return this.spaces.find((s) => s.id === this.spaceId)
  }

  /** `work/einarbeiten` — what the sidebar and the status line show. */
  get here(): string {
    return this.path.map((crumb) => crumb.slug).join('/')
  }

  get list() {
    return this.board?.lists[this.listCursor]
  }

  get card(): Card | undefined {
    return this.list?.cards[this.cardCursor]
  }

  /** The agenda in the order it is drawn and walked, bucketed by urgency. */
  get agendaGroups(): { key: 'overdue' | 'today' | 'week' | 'doing'; cards: AgendaCard[] }[] {
    const midnight = new Date()
    midnight.setHours(0, 0, 0, 0)
    const start = Math.floor(midnight.getTime() / 1000)
    const groups = { overdue: [] as AgendaCard[], today: [] as AgendaCard[], week: [] as AgendaCard[], doing: [] as AgendaCard[] }
    for (const card of this.agenda.cards) {
      if (card.due_at !== null && card.due_at < start) groups.overdue.push(card)
      else if (card.due_at !== null && card.due_at < start + 86400) groups.today.push(card)
      else if (card.due_at !== null && card.due_at < start + 8 * 86400) groups.week.push(card)
      else if (card.role === 'doing') groups.doing.push(card)
    }
    return (['overdue', 'today', 'week', 'doing'] as const).map((key) => ({ key, cards: groups[key] }))
  }

  get agendaCards(): AgendaCard[] {
    return this.agendaGroups.flatMap((group) => group.cards)
  }

  /** `late 1 · today 2` — what the bar shows. */
  get dueCounts(): { late: number; today: number } {
    const [overdue, today] = this.agendaGroups
    return { late: overdue.cards.length, today: today.cards.length }
  }

  /** Proton's human check a login waits on, while its sheet is open. */
  verify = $state.raw<{ url: string; resolve: (token: string | null) => void } | null>(null)

  /** The token Proton's check at `url` hands back, or null when abandoned. */
  askVerification(url: string): Promise<string | null> {
    this.verify?.resolve(null)
    return new Promise((resolve) => {
      let answered = false
      this.verify = {
        url,
        resolve: (token) => {
          if (answered) return
          answered = true
          this.verify = null
          resolve(token)
        },
      }
      this.overlay = 'verify'
    })
  }

  /** A password, or null when it was waved away with Escape. */
  askSecret(text: string): Promise<string | null> {
    this.secret?.resolve(null)
    return new Promise((resolve) => {
      let answered = false
      const asked = {
        text,
        resolve: (value: string | null) => {
          if (answered) return
          answered = true
          // Put away a turn later, so a question that follows straight on —
          // the second of two passwords — takes over the same box instead
          // of a new one appearing under the fingers already typing it.
          setTimeout(() => {
            if (this.secret === asked) this.secret = null
          })
          resolve(value)
        },
      }
      this.secret = asked
    })
  }

  async loadVault() {
    this.vault = await api.vaultStatus()
  }

  /**
   * Whatever needs the vault asks for the password itself rather than failing
   * with "locked": opening it is the obvious next step anyway. False when
   * there is no vault, or the question was declined.
   */
  async ensureUnlocked(): Promise<boolean> {
    await this.loadVault()
    if (!this.vault?.configured) {
      this.say('there is no vault yet — :vault sets one up', 'error')
      return false
    }
    if (this.vault.unlocked) return true
    const password = await this.askSecret('vault password')
    if (password === null) return false
    // The key is stretched on purpose, which takes a moment worth saying.
    this.say('unlocking …')
    this.vault = await api.unlockVault(password)
    this.say('vault unlocked')
    return true
  }

  /** Seal what is under the cursor, or take it back out, and show the result. */
  async seal(id: number, encrypt: boolean) {
    const { files, unchanged, failed, shared, sealed } = await api.seal(id, encrypt)
    const verb = encrypt ? 'encrypted' : 'decrypted'
    const already = unchanged > 0 ? `, ${unchanged} already were` : ''
    const broken = failed > 0 ? `, ${failed} could not be` : ''
    const later = sealed ? ' — what lands in the folder from now on is sealed too' : ''
    const readers =
      shared > 0 ? ` — ${shared} shared document${shared === 1 ? '' : 's'} embed it, and their readers no longer see it` : ''
    this.say(
      `${verb} ${files} file${files === 1 ? '' : 's'}${already}${broken}${later}${readers}`,
      failed > 0 || shared > 0 ? 'error' : undefined,
    )
    await this.refresh()
    for (const tile of this.tiles) if (tile.open?.kind === 'file') tile.open = await api.getNode(tile.open.id)
  }

  ask(text: string): Promise<boolean> {
    this.prompt?.resolve(false)
    return new Promise((resolve) => {
      this.prompt = {
        text,
        resolve: (yes) => {
          this.prompt = null
          resolve(yes)
        },
      }
    })
  }

  say(text: string, kind: 'info' | 'error' = 'info') {
    this.flash = { text, kind }
    if (kind === 'info') {
      const current = this.flash
      setTimeout(() => {
        if (this.flash === current) this.flash = null
      }, 4000)
    }
  }

  /*
   * Autosave. Typing is where work gets lost, so a draft goes to localStorage
   * on every change — cheap, synchronous, survives a killed tab — and to the
   * server a couple of seconds after typing stops.
   */

  static readonly AUTOSAVE_MS = 2500
  static readonly RECENT_DONE_DAYS = 7

  #stashKey(id: number) {
    return `diary:draft:${id}`
  }

  /** Call whenever the draft changes. Replaces a bare `dirty = true`. */
  touch(tile = this.tile) {
    tile.dirty = true
    this.stash(tile)
    tile.stopAutosave()
    tile.autosave = setTimeout(() => void this.#writeBack(tile), Workspace.AUTOSAVE_MS)
  }

  stash(tile = this.tile) {
    if (!tile.open) return
    try {
      localStorage.setItem(this.#stashKey(tile.open.id), JSON.stringify(tile.draft))
    } catch {
      /* a full or disabled localStorage must not stop the typing */
    }
  }

  dropStash(id: number) {
    try {
      localStorage.removeItem(this.#stashKey(id))
    } catch {
      /* nothing to do */
    }
  }

  #readStash(id: number): Draft | null {
    try {
      const raw = localStorage.getItem(this.#stashKey(id))
      if (!raw) return null
      const stashed = JSON.parse(raw)
      // Drafts stashed before documents had names carry a title instead.
      return { name: stashed.name ?? stashed.title ?? '', body: stashed.body, created_at: stashed.created_at }
    } catch {
      return null
    }
  }

  async #writeBack(tile: Tile) {
    tile.autosave = null
    if (!tile.open || !tile.dirty) return
    try {
      await this.save({ quiet: true, tile })
      if (tile.retryDelay > 0) this.say('saved — the server is back')
      tile.retryDelay = 0
    } catch (error) {
      if (!unreachable(error)) return this.guard(() => Promise.reject(error))
      // Nothing is lost — the draft is in localStorage — but it is not on the
      // server either, so it is tried again, less often the longer it fails.
      tile.retryDelay = Math.min(Math.max(tile.retryDelay * 2, 2000), 60_000)
      this.say(`not saved: the server cannot be reached — kept here, trying again in ${tile.retryDelay / 1000}s`, 'error')
      tile.autosave = setTimeout(() => void this.#writeBack(tile), tile.retryDelay)
    }
  }

  /** Back online: what is waiting goes now rather than at the next retry. */
  #watchConnection() {
    window.addEventListener('online', () => {
      for (const tile of this.tiles) {
        if (!tile.dirty) continue
        tile.stopAutosave()
        void this.#writeBack(tile)
      }
    })
    window.addEventListener('offline', () => {
      this.say('offline — what is typed is kept here, and saved once the server answers', 'error')
    })
  }

  /**
   * Write out every pending draft before doing something that would replace
   * one. Without this, clicking another document in the list discarded
   * unsaved work without a word.
   */
  async flush() {
    for (const tile of this.tiles) await this.flushTile(tile)
  }

  /** The same for one tile only, as it is about to show something else. */
  async flushTile(tile = this.tile) {
    tile.stopAutosave()
    if (await this.abandonIfBlank(tile)) return
    if (tile.open && tile.dirty) await this.guard(() => this.save({ quiet: true, tile }))
  }

  /**
   * Drop the document `o` made if nothing was ever put in it. True when it
   * went, so the caller knows there is nothing open any more.
   */
  async abandonIfBlank(tile = this.tile): Promise<boolean> {
    const open = tile.open
    if (!open || open.id !== tile.fresh) return false
    if (tile.draft.name.trim() || tile.draft.body.trim()) return false
    tile.clear()
    this.dropStash(open.id)
    this.prune()
    try {
      await api.deleteNode(open.id)
    } catch {
      /* already gone, or the server is away; an empty row is no great loss */
    }
    await this.refresh()
    return true
  }

  #watching = false

  async boot() {
    if (!this.#watching) {
      this.#watching = true
      this.#watchConnection()
    }
    try {
      const { user } = await api.me()
      this.user = user
      this.notice = ''
      await this.loadSpaces(Workspace.addressedPath())
      await this.loadAgenda()
      await this.loadVault()
      await this.openAgendaIfDue()
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
    if (this.spaces.length > 0) {
      await this.refresh()
      for (const tile of this.tiles) {
        if (tile.open && tile.dirty) await this.guard(() => this.save({ quiet: true, tile }))
      }
      return
    }
    await this.loadSpaces(Workspace.addressedPath())
    await this.loadAgenda()
    await this.loadVault()
    await this.openAgendaIfDue()
  }

  async logout(everywhere = false) {
    await api.logout(everywhere)
    this.user = null
    this.vault = null
    this.spaces = []
    this.nodes = []
    for (const tile of this.tiles) tile.clear()
    this.prune()
  }

  /*
   * Navigation. The list pane always shows one folder's children, so browsing
   * is: pick a space, descend into folders, come back up. A search steps
   * outside that and shows matches from anywhere in the current space.
   */

  /** The slug path in the address bar, if the page was opened on a `/n/...` URL. */
  static addressedPath(): string | null {
    return location.pathname.startsWith('/n/')
      ? decodeURIComponent(location.pathname.slice(3)).replace(/\/$/, '')
      : null
  }

  /** Keep the address bar on the node being looked at, so it can be linked to. */
  #syncUrl(path: Crumb[]) {
    const slugs = path.map((crumb) => crumb.slug).join('/')
    if (slugs) history.replaceState(null, '', `/n/${slugs}`)
  }

  /** Open whatever `work/einarbeiten/01-was-ist-eebus` names. */
  async openPath(path: string) {
    const node = await api.resolvePath(path)
    if (!isContainer(node)) {
      if (node.parent_id !== null) await this.goTo(node.parent_id)
      await this.openNode(node.id)
    } else {
      await this.goTo(node.id)
    }
  }

  async loadSpaces(startPath: string | null = null) {
    this.spaces = await api.listSpaces()
    if (startPath) {
      try {
        await this.openPath(startPath)
        return
      } catch {
        // A stale or hand-typed address should not leave the app on nothing.
        this.say(`no such path: ${startPath}`, 'error')
      }
    }
    const remembered = Number(localStorage.getItem('diary:space'))
    const start =
      this.spaces.find((s) => s.id === remembered) ??
      this.spaces.find((s) => s.role === 'diary') ??
      this.spaces[0]
    if (start) await this.enterSpace(start.id)
  }

  #rememberSpot() {
    if (this.spaceId === null || this.parentId === null) return
    const open = this.tiles.flatMap((tile) => (tile.open ? [tile.open.id] : []))
    this.#spots[this.spaceId] = {
      parentId: this.parentId,
      openId: this.open?.id ?? null,
      cursor: this.cursor,
      pane: this.pane,
      open,
      focused: Math.max(open.indexOf(this.open?.id ?? -1), 0),
    }
    localStorage.setItem('diary:spots', JSON.stringify(this.#spots))
  }

  async enterSpace(id: number) {
    const leaving = this.spaceId !== null && this.spaceId !== id
    if (leaving) {
      await this.flush()
      this.#rememberSpot()
      const from = this.spaces.findIndex((s) => s.id === this.spaceId)
      const to = this.spaces.findIndex((s) => s.id === id)
      this.switchDirection = to >= from ? 1 : -1
      // Each space has tiles of its own; these come back with this one.
      for (const tile of this.tiles) tile.stopAutosave()
      this.tiles = [new Tile()]
      this.focused = 0
      this.pane = 'list'
    }
    this.spaceId = id
    localStorage.setItem('diary:space', String(id))
    this.query = ''

    // A remembered folder may have been deleted or moved to another space
    // since; then the space's root is where it opens.
    const spot = this.#spots[id]
    try {
      await this.goTo(spot?.parentId ?? id)
      if (this.spaceId !== id) await this.goTo(id)
    } catch {
      await this.goTo(id)
    }
    if (spot && this.parentId === spot.parentId) {
      const ids = spot.open ?? (spot.openId !== null && this.nodes.some((n) => n.id === spot.openId) ? [spot.openId] : [])
      let opened = 0
      for (const open of ids) {
        if (opened > 0) {
          this.tiles.push(new Tile())
          this.focused = this.tiles.length - 1
        }
        try {
          // Reopened under the board, if that is what is on screen.
          await this.openNode(open, false, true)
          opened++
        } catch {
          /* deleted or moved away since: one tile fewer */
        }
      }
      this.prune()
      if (opened > 0) {
        this.focusTile(Math.min(spot.focused ?? 0, this.tiles.length - 1))
        this.pane = spot.pane
      } else {
        this.cursor = Math.min(spot.cursor, Math.max(this.nodes.length - 1, 0))
      }
    }
    // The board belongs to the space, so switching space while looking at one
    // shows the new space's board — or drops back to the tree for a space that
    // has none, rather than leaving the previous board on screen.
    if (this.view === 'board') {
      this.listCursor = 0
      this.cardCursor = 0
      if (this.space?.has_board) await this.reloadBoard(id, undefined)
      else this.closeBoard()
    }
  }

  /** Show the children of `id`, which must be a space or a folder. */
  async goTo(id: number, keep?: number) {
    await this.flush()
    const folder = await api.getNode(id)
    // Walking into another space by a link leaves this one where it was.
    if (this.spaceId !== null && folder.path[0]?.id !== this.spaceId) this.#rememberSpot()
    this.parentId = id
    this.parentSealed = folder.sealed
    this.query = ''
    this.globalSearch = false
    this.path = folder.path
    this.spaceId = folder.path[0]?.id ?? id
    this.nodes = await api.listNodes({ parent: id })
    const index = this.nodes.findIndex((n) => n.id === keep)
    this.cursor = index >= 0 ? index : 0
    this.#syncUrl(folder.path)
  }

  /** Up one level, selecting the folder just left so `h` then `l` is a no-op. */
  async up() {
    const from = this.path.at(-1)
    const parent = this.path.at(-2)
    if (!parent || !from) return
    await this.goTo(parent.id, from.id)
  }

  /** `keep` follows a node that just arrived, instead of holding the cursor. */
  async refresh(keep?: number) {
    const keepId = keep ?? this.selected?.id
    this.nodes = this.query
      ? await api.listNodes({ q: this.query, space: this.globalSearch ? undefined : (this.spaceId ?? undefined) })
      : await api.listNodes({ parent: this.parentId ?? undefined })
    const index = this.nodes.findIndex((n) => n.id === keepId)
    this.cursor = index >= 0 ? index : Math.min(this.cursor, Math.max(this.nodes.length - 1, 0))
  }

  async search(query: string, global = this.globalSearch) {
    // The results are rows in the list, which the board would hide.
    if (query && this.view === 'board') this.closeBoard()
    this.query = query
    this.globalSearch = global && query !== ''
    this.cursor = 0
    await this.refresh()
  }

  /**
   * `n` / `N`: the next or previous search result after the cursor, opened
   * beside the results so the next `n` goes on from there. A folder is only
   * highlighted: going into it would end the search. Past the last result it
   * starts again at the first, as vim's `wrapscan` does.
   */
  async nextResult(delta: 1 | -1) {
    if (!this.query) return this.say('no search — / starts one', 'error')
    const count = this.nodes.length
    if (count === 0) return this.say(`no results for "${this.query}"`, 'error')
    const next = this.cursor + delta
    if (next >= count) this.say('search hit BOTTOM, continuing at TOP')
    else if (next < 0) this.say('search hit TOP, continuing at BOTTOM')
    this.cursor = (next + count) % count
    const node = this.nodes[this.cursor]
    if (!isContainer(node)) await this.openNode(node.id)
  }

  move(delta: number) {
    if (this.nodes.length === 0) return
    this.cursor = Math.min(Math.max(this.cursor + delta, 0), this.nodes.length - 1)
  }

  async openNode(id: number, edit = false, underBoard = false) {
    if (this.open?.id !== id) await this.flushTile()

    const node = await api.getNode(id)
    // What is asked for from the board — `:12`, `T`, a search — is shown, not
    // opened behind it; closing a document goes back to the board.
    if (this.view === 'board' && !underBoard) {
      if (isContainer(node)) this.closeBoard()
      else {
        this.view = 'tree'
        this.returnTo = 'board'
      }
    }
    if (isContainer(node)) {
      await this.goTo(node.id)
      return
    }
    // A document already open in another tile is gone to, not opened twice:
    // two drafts of one document would only overwrite each other.
    const shown = this.tiles.findIndex((tile) => tile !== this.tile && tile.open?.id === id)
    if (shown >= 0) {
      this.focusTile(shown)
      if (edit && node.kind === 'document') this.editing = true
      return
    }

    this.open = node
    this.draft = { name: node.name, body: node.body, created_at: node.created_at }
    this.dirty = false
    // A file has a name and nothing to write in.
    this.editing = edit && node.kind === 'document'
    this.pane = 'document'
    this.#syncUrl(node.path)
    const index = this.nodes.findIndex((n) => n.id === id)
    if (index >= 0) this.cursor = index

    // A stash that differs from what the server has is work the last session
    // did not manage to write. Vim would call this a swap file.
    const stashed = this.#readStash(id)
    if (stashed && (stashed.body !== node.body || stashed.name !== node.name)) {
      this.draft = stashed
      this.dirty = true
      this.say('recovered an unsaved draft — :w to keep it, :e! to discard', 'error')
    } else {
      this.dropStash(id)
    }
  }

  /** What Enter does: descend into a folder, open a document or a file. */
  async openSelected(edit = false) {
    const selected = this.selected
    if (!selected) return
    // A match from anywhere is gone to, not opened beside a list it is not in.
    if (this.globalSearch) {
      const node = await api.getNode(selected.id)
      return this.openPath(node.path.map((crumb) => crumb.slug).join('/'))
    }
    if (isContainer(selected)) await this.goTo(selected.id)
    else await this.openNode(selected.id, edit)
  }

  /** A new document is created immediately so uploads have something to attach to. */
  async createDocument(name = '') {
    const parent = this.parentId
    if (parent === null) return
    const node = await api.createNode({ parent_id: parent, name, body: '' })
    this.query = ''
    this.globalSearch = false
    await this.refresh()
    await this.openNode(node.id, true)
    this.tile.fresh = node.id
    this.enterInsert = true
    this.say(`new document ${this.here}/${node.slug}`)
  }

  async createFolder(name: string) {
    const parent = this.parentId
    if (parent === null) return
    const node = await api.createNode({ parent_id: parent, kind: 'folder', name })
    await this.refresh()
    const index = this.nodes.findIndex((n) => n.id === node.id)
    if (index >= 0) this.cursor = index
    this.say(`new folder ${this.here}/${node.slug}`)
  }

  async createSpace(name: string) {
    const space = await api.createSpace(name)
    await this.loadSpaces()
    await this.enterSpace(space.id)
    this.say(`new space ${space.slug}`)
  }

  /**
   * Write the draft. Refused when the document was saved somewhere else
   * since it was opened here — another tab, the phone, the SFTP mount —
   * unless `force`, which is `:w!`.
   */
  async save({ quiet = false, force = false, tile = this.tile } = {}) {
    if (!tile.open) return
    // Snapshot what goes over the wire: an autosave races the typing that
    // triggered it, and anything typed since must stay marked unsaved.
    const sent = { ...tile.draft, name: tile.draft.name.trim() }
    const saved = await api.updateNode(tile.open.id, {
      ...sent,
      base_updated_at: force ? undefined : tile.open.updated_at,
    })

    tile.open = saved
    tile.dirty =
      tile.draft.body !== sent.body ||
      tile.draft.name.trim() !== sent.name ||
      tile.draft.created_at !== sent.created_at
    if (tile.dirty) this.touch(tile)
    else this.dropStash(saved.id)
    // The response carries the same excerpt the list query builds, so the row
    // can usually be patched in place rather than re-reading the whole list.
    if (!this.patch(saved)) await this.refresh()
    if (!quiet) this.say(`"${saved.slug}" ${saved.body.split('\n').length}L written`)
  }

  /**
   * Fold a freshly saved document back into its list row. Returns false when
   * the row cannot be patched in place — a new document, or one whose date or
   * slug moved it somewhere else — and the caller should re-read the list.
   */
  patch(node: WorkspaceNode): boolean {
    const index = this.nodes.findIndex((n) => n.id === node.id)
    if (index < 0) return false
    const row = this.nodes[index]
    if (row.created_at !== node.created_at || row.slug !== node.slug) return false
    const { body: _body, path: _path, ...summary } = node
    this.nodes[index] = summary
    return true
  }

  async deleteNode(id: number) {
    const showing = this.tilesWith(id)
    for (const tile of showing) tile.stopAutosave()
    this.dropStash(id)
    const node = showing[0]?.open ?? this.nodes.find((n) => n.id === id)
    const { removed } = await api.deleteNode(id)
    for (const tile of showing) tile.clear()
    this.prune()
    this.marked = this.marked.filter((n) => n.id !== id)
    await this.refresh()
    const name = node ? `"${node.name.trim() || node.slug}"` : `#${id}`
    this.say(removed > 1 ? `deleted ${name} and ${removed - 1} inside it` : `deleted ${name}`)
  }

  /**
   * Rename anything in the tree — a document, a folder or a space. The body
   * has to ride along because the update is a whole-node write; for the node
   * being edited that is the draft, which would otherwise be rolled back to
   * whatever the server still holds.
   */
  async renameNode(id: number, name: string) {
    const current =
      this.open?.id === id ? { ...this.open, body: this.draft.body } : await api.getNode(id)
    const saved = await api.updateNode(id, {
      name: name.trim(),
      body: current.body,
      created_at: current.created_at,
    })
    for (const tile of this.tilesWith(id)) {
      tile.open = saved
      tile.draft.name = saved.name
    }
    if (this.path.some((crumb) => crumb.id === id)) this.path = saved.path
    if (saved.kind === 'space') this.spaces = await api.listSpaces()
    await this.refresh()
    this.say(`renamed to ${saved.slug}`)
  }

  /**
   * Delete a space and everything in it. The app has to land somewhere
   * afterwards, so it falls into the next space — or, if that was the last
   * one, into an empty workspace that `:space! <name>` fills again.
   */
  async deleteSpace(id: number) {
    const { removed } = await api.deleteNode(id)
    for (const tile of this.tiles) tile.clear()
    this.prune()
    this.rawBoard = null
    this.view = 'tree'
    this.spaces = await api.listSpaces()
    const next = this.spaces.find((space) => space.role === 'diary') ?? this.spaces[0]
    if (next) await this.enterSpace(next.id)
    else {
      localStorage.removeItem('diary:space')
      this.spaceId = null
      this.parentId = null
      this.path = []
      this.nodes = []
      this.cursor = 0
    }
    this.say(`deleted the space and ${removed - 1} node(s) in it`)
  }

  /** Reparent the node under the cursor, for `:mv`. */
  async moveNode(id: number, parentId: number) {
    await api.moveNode(id, parentId)
    await this.refresh()
    this.say(`moved #${id}`)
  }

  isMarked(id: number): boolean {
    return this.marked.some((n) => n.id === id)
  }

  /**
   * Move `nodes` into `parent`, all or none, and say how it went. An open one
   * is saved and closed first: it is about to be somewhere else.
   */
  async moveNodes(nodes: NodeSummary[], parent: number, where = 'here') {
    if (nodes.length === 0) return this.say('nothing marked — press m on a row first', 'error')
    const leaving = this.tiles.filter((tile) => nodes.some((n) => n.id === tile.open?.id))
    for (const tile of leaving) {
      await this.flushTile(tile)
      tile.clear()
    }
    this.prune()
    const { moved, unchanged } = await api.moveNodes(
      nodes.map((n) => n.id),
      parent,
    )
    const gone = new Set(nodes.map((n) => n.id))
    this.marked = this.marked.filter((n) => !gone.has(n.id))
    await this.refresh(nodes.length === 1 ? nodes[0].id : undefined)
    const what = nodes.length === 1 ? nodes[0].name.trim() || nodes[0].slug : `${moved}`
    const already = unchanged > 0 ? ` — ${unchanged} already ${unchanged === 1 ? 'was' : 'were'}` : ''
    this.say(moved === 0 ? `already ${where}` : `moved ${what} ${where}${already}`)
  }

  /** `1`..`9`, `0`: the space at that place on the bar. */
  async switchSpace(index: number) {
    const space = this.spaces[index]
    if (!space) return this.say(`no space ${index + 1}`, 'error')
    if (space.id !== this.spaceId) await this.enterSpace(space.id)
  }

  /** Tab / Shift-Tab: the next or previous space, wrapping round. */
  async cycleSpace(delta: number) {
    const count = this.spaces.length
    if (count < 2) return
    const at = this.spaces.findIndex((s) => s.id === this.spaceId)
    await this.enterSpace(this.spaces[(at + delta + count) % count].id)
  }

  /**
   * Shift+digit: send the marked node, or else the one under the cursor, to
   * the root of the space at that place — the window moved to workspace N.
   */
  async sendToSpace(index: number) {
    const space = this.spaces[index]
    const nodes = this.marked.length ? this.marked : this.selected ? [this.selected] : []
    if (!space) return this.say(`no space ${index + 1}`, 'error')
    if (nodes.length === 0) return this.say('nothing selected', 'error')
    await this.moveNodes(nodes, space.id, `→ ${index + 1} ${space.slug}`)
    this.spaces = await api.listSpaces()
  }

  async loadAgenda() {
    try {
      this.agenda = await api.agenda()
    } catch {
      /* the counter is a convenience; a failed refresh leaves the last one up */
    }
    this.agendaCursor = Math.min(this.agendaCursor, Math.max(this.agendaCards.length - 1, 0))
  }

  /**
   * The agenda opens itself once a day when something is late or due — the
   * point is to be told, not to have to remember to ask.
   */
  async openAgendaIfDue() {
    const today = new Date().toDateString()
    if (localStorage.getItem('diary:agenda-seen') === today) return
    const { late, today: due } = this.dueCounts
    if (late + due === 0) return
    localStorage.setItem('diary:agenda-seen', today)
    this.agendaCursor = 0
    this.overlay = 'agenda'
  }

  /** Enter in the agenda: to the card, on its own board. */
  async goToCard(card: AgendaCard) {
    this.overlay = 'none'
    if (this.spaceId !== card.space_id) await this.enterSpace(card.space_id)
    await this.reloadBoard(card.space_id, card.id)
    this.view = 'board'
  }

  /** Space in the agenda: into that board's done list, from wherever it is. */
  async finishCard(card: AgendaCard) {
    const done = (await api.board(card.space_id)).lists.at(-1)
    if (!done) return
    await api.moveCard(card.id, done.id, done.cards.length)
    await this.loadAgenda()
    if (this.view === 'board' && this.spaceId === card.space_id) await this.reloadBoard()
    this.say(`done: ${card.title}`)
  }

  /** `:due` with the agenda up: the highlighted card's date, on its own board. */
  async dueAgendaCard(due_at: number | null) {
    const card = this.agendaCards[this.agendaCursor]
    if (!card) return this.say('no card highlighted', 'error')
    await api.updateCard(card.id, { due_at })
    await this.loadAgenda()
    // It may have changed group; the cursor goes with it.
    const at = this.agendaCards.findIndex((c) => c.id === card.id)
    if (at >= 0) this.agendaCursor = at
    if (this.view === 'board' && this.spaceId === card.space_id) await this.reloadBoard()
  }

  /**
   * `H` / `L` in the agenda: the card to the list left or right of the one it
   * is in, without going to the board — `L` on the last list but one is done.
   */
  async sendAgendaCard(card: AgendaCard, delta: number) {
    const lists = (await api.board(card.space_id)).lists
    const from = lists.findIndex((list) => list.id === card.list_id)
    const target = lists[from + delta]
    if (from < 0 || !target) return this.say(`${card.title} is already in the ${delta < 0 ? 'first' : 'last'} list`, 'error')
    await api.moveCard(card.id, target.id, target.cards.length)
    await this.loadAgenda()
    const at = this.agendaCards.findIndex((c) => c.id === card.id)
    if (at >= 0) this.agendaCursor = at
    if (this.view === 'board' && this.spaceId === card.space_id) await this.reloadBoard()
    this.say(from + delta === lists.length - 1 ? `done: ${card.title}` : `"${card.title}" → ${target.name}`)
  }

  async loadOverview() {
    this.overview = await api.overview()
  }

  /**
   * `m`: pick the node under the cursor up, or put it back down unmoved, and
   * step to the next row — `mmm` picks up three in a row.
   */
  mark(node = this.selected, step = true) {
    if (!node) return this.say('nothing selected', 'error')
    this.marked = this.isMarked(node.id) ? this.marked.filter((n) => n.id !== node.id) : [...this.marked, node]
    if (step && node === this.selected) this.move(1)
    this.#sayMarked()
  }

  /** Shift-click: mark every row from the cursor to `index`. */
  markRange(index: number) {
    const [from, to] = [Math.min(this.cursor, index), Math.max(this.cursor, index)]
    const rows = this.nodes.slice(from, to + 1).filter((n) => !this.isMarked(n.id))
    this.marked = [...this.marked, ...rows]
    this.cursor = index
    this.#sayMarked()
  }

  /** `M`: mark the whole folder, or, when it all is, none of it. */
  markAll() {
    const all = this.nodes.length > 0 && this.nodes.every((n) => this.isMarked(n.id))
    const here = new Set(this.nodes.map((n) => n.id))
    this.marked = all
      ? this.marked.filter((n) => !here.has(n.id))
      : [...this.marked, ...this.nodes.filter((n) => !this.isMarked(n.id))]
    this.#sayMarked()
  }

  unmarkAll() {
    if (this.marked.length === 0) return
    this.marked = []
    this.say('unmarked')
  }

  #sayMarked() {
    const count = this.marked.length
    if (count === 0) return this.say('unmarked')
    const first = this.marked[0]
    const what = count === 1 ? first.name.trim() || first.slug : `${count} nodes`
    this.say(`marked ${what} — p drops ${count === 1 ? 'it' : 'them'} in the folder you are in`)
  }

  /** `p`: drop the marked nodes into the folder the list is showing. */
  async drop() {
    const parent = this.parentId
    if (this.marked.length === 0) return this.say('nothing marked — press m on a row first', 'error')
    if (parent === null) return this.say('no folder open', 'error')
    await this.moveNodes(this.marked, parent)
  }

  /*
   * Boards. One per space, which is what makes "what am I working on" a
   * question with an answer: work and uni are asked separately, and the diary
   * is not asked at all. The board is reloaded whole after every change — it
   * is a few dozen rows, and a board that disagrees with the server about
   * where a card is would be worse than a round trip.
   */

  async openBoard() {
    const space = this.space
    if (!space) return
    if (!space.has_board) {
      return this.say(`${space.slug} has no board — :set board turns one on`, 'error')
    }
    await this.reloadBoard(space.id)
    this.view = 'board'
  }

  closeBoard() {
    this.view = 'tree'
    this.returnTo = null
  }

  async toggleBoardView() {
    if (this.view === 'board') this.closeBoard()
    else await this.openBoard()
  }

  async reloadBoard(spaceId = this.spaceId, keep = this.card?.id) {
    if (spaceId === null) return
    this.rawBoard = await api.board(spaceId)
    void this.loadAgenda()
    // Follow the card that was just moved rather than whatever now sits under
    // the cursor, so J J J walks one card down instead of shuffling three.
    if (keep !== undefined) {
      const at = this.locate(keep)
      if (at) {
        this.listCursor = at.list
        this.cardCursor = at.card
      }
    }
    this.clampBoard()
  }

  /** Where a card sits now, as list and card indices. */
  locate(cardId: number): { list: number; card: number } | null {
    const lists = this.board?.lists ?? []
    for (let list = 0; list < lists.length; list++) {
      const card = lists[list].cards.findIndex((c) => c.id === cardId)
      if (card >= 0) return { list, card }
    }
    return null
  }

  clampBoard() {
    const lists = this.board?.lists ?? []
    this.listCursor = Math.min(Math.max(this.listCursor, 0), Math.max(lists.length - 1, 0))
    const cards = lists[this.listCursor]?.cards ?? []
    this.cardCursor = Math.min(Math.max(this.cardCursor, 0), Math.max(cards.length - 1, 0))
  }

  /** `h`/`l`: to the list next door, keeping the cursor as near as it can. */
  moveList(delta: number) {
    const lists = this.board?.lists ?? []
    if (lists.length === 0) return
    this.listCursor = Math.min(Math.max(this.listCursor + delta, 0), lists.length - 1)
    this.clampBoard()
  }

  /** `j`/`k`: down and up the cards of the current list. */
  moveCard(delta: number) {
    const cards = this.list?.cards ?? []
    if (cards.length === 0) return
    this.cardCursor = Math.min(Math.max(this.cardCursor + delta, 0), cards.length - 1)
  }

  /** `J`/`K`: take the card with you instead of leaving it behind. */
  async nudgeCard(delta: number) {
    const card = this.card
    const list = this.list
    if (!card || !list) return
    const to = this.cardCursor + delta
    if (to < 0 || to >= list.cards.length) return
    // The neighbour's own position, not its index: older done cards may be
    // hidden above it.
    await api.moveCard(card.id, list.id, list.cards[to].position)
    await this.reloadBoard(this.spaceId, card.id)
  }

  /** `H`/`L`: the same card, one list over — the move a board is really for. */
  async sendCard(delta: number) {
    await this.sendCardTo(this.listCursor + delta)
  }

  /*
   * The last list is "done": the server stamps a card finished as it arrives
   * there and clears the stamp as it leaves, so moving is all there is to it.
   */
  async sendCardTo(index: number) {
    const card = this.card
    const lists = this.board?.lists ?? []
    const target = lists[index]
    if (!card || !target || index === this.listCursor) return
    await api.moveCard(card.id, target.id, target.cards.length)
    await this.reloadBoard(this.spaceId, card.id)
    this.say(index === lists.length - 1 ? `done: ${card.title}` : `"${card.title}" → ${target.name}`)
  }

  /**
   * A card lands in the first list — the inbox — of this space's board, or of
   * the first space that has one, without leaving the view you are in: writing
   * a thought down must not cost the place you were at. On the board itself
   * it goes on the list under the cursor instead.
   */
  async createCard(title: string, nodeId: number | null = null) {
    if (this.view === 'board') {
      const list = this.list
      if (!list) return this.say('no list to put it on', 'error')
      const card = await api.createCard(list.id, { title, node_id: nodeId })
      await this.reloadBoard(this.spaceId, card.id)
      return this.say(nodeId === null ? `carded "${card.title}"` : `carded "${card.title}" → #${nodeId}`)
    }

    const space = this.space?.has_board ? this.space : this.spaces.find((s) => s.has_board)
    if (!space) return this.say('no space has a board — :set board turns one on', 'error')
    const inbox = (await api.board(space.id)).lists[0]
    if (!inbox) return this.say(`${space.slug} has no lists — :board, then :list <name>`, 'error')
    const card = await api.createCard(inbox.id, { title, node_id: nodeId })
    await this.loadAgenda()
    this.say(`carded "${card.title}" → ${space.slug}/${inbox.name}`)
  }

  async patchCard(patch: Parameters<typeof api.updateCard>[1]) {
    const card = this.card
    if (!card) return this.say('no card selected', 'error')
    await api.updateCard(card.id, patch)
    await this.reloadBoard(this.spaceId, card.id)
  }

  /** Space: into the done list, or back out of it to the inbox. */
  async toggleDone() {
    const card = this.card
    const lists = this.board?.lists ?? []
    if (!card) return this.say('no card selected', 'error')
    if (lists.length < 2) return this.say('a board needs two lists for one to mean done', 'error')
    const last = lists.length - 1
    if (this.listCursor === last) {
      await this.sendCardTo(0)
      this.say(`reopened: ${card.title}`)
    } else {
      await this.sendCardTo(last)
    }
  }

  async deleteCard() {
    const card = this.card
    if (!card) return this.say('no card selected', 'error')
    await api.deleteCard(card.id)
    await this.reloadBoard(this.spaceId, undefined)
    this.say(`deleted "${card.title}"`)
  }

  async createList(name: string) {
    if (this.spaceId === null) return
    this.rawBoard = await api.createList(this.spaceId, name)
    // It goes in front of the done list, so it is not simply the last one.
    const names = this.rawBoard.lists.map((list) => list.name)
    this.listCursor = Math.max(names.lastIndexOf(name), 0)
    this.cardCursor = 0
    this.say(`new list ${name}`)
  }

  async renameList(name: string) {
    const list = this.list
    if (!list) return this.say('no list selected', 'error')
    this.rawBoard = await api.renameList(list.id, name)
    this.clampBoard()
    this.say(`list renamed to ${name}`)
  }

  async deleteList() {
    const list = this.list
    if (!list) return this.say('no list selected', 'error')
    this.rawBoard = await api.deleteList(list.id)
    this.clampBoard()
    this.say(`deleted list ${list.name}`)
  }

  /**
   * Open the document a card points at. A card without one gets one, named
   * after it, in the space's root — so Enter always leads to a place to write
   * the notes down. `q` on that document comes back here.
   */
  async openCard() {
    const card = this.card
    const space = this.spaceId
    if (!card || space === null) return
    let nodeId = card.node_id
    if (nodeId === null) {
      const node = await api.createNode({ parent_id: space, name: card.title, body: '' })
      await api.updateCard(card.id, { node_id: node.id })
      nodeId = node.id
    }
    this.view = 'tree'
    if (this.parentId !== space) await this.goTo(space)
    else await this.refresh()
    // The notes open beside whatever was being read, and `q` takes their
    // tile away again on the way back to the board.
    if (this.open && this.open.id !== nodeId) {
      this.tiles.push(new Tile())
      this.focused = this.tiles.length - 1
    }
    await this.openNode(nodeId, card.node_id === null)
    this.prune()
    if (card.node_id === null) this.enterInsert = true
    this.returnTo = 'board'
  }

  /** Back to the board a card's document was opened from. */
  async returnToBoard() {
    this.returnTo = null
    this.closeTile()
    this.pane = 'list'
    await this.reloadBoard()
    this.view = 'board'
  }

  /** Turn a space's board on or off. The diary ships with it off. */
  async setBoard(on: boolean) {
    const space = this.space
    if (!space) return
    await api.updateNode(space.id, { name: space.name, body: '', has_board: on })
    this.spaces = await api.listSpaces()
    if (on) await this.openBoard()
    else {
      this.closeBoard()
      this.rawBoard = null
    }
    this.say(on ? `${space.slug} has a board` : `${space.slug} has no board any more`)
  }

  async toggleShare(id: number) {
    const node = this.open?.id === id ? this.open : await api.getNode(id)
    if (node.kind !== 'document') return this.say('only a document can be shared', 'error')
    if (node.shared) {
      await api.unshare(id)
      this.say(`#${id} is private again`)
    } else {
      const { url, sealed } = await this.mintShareLink(id)
      this.say(`shared → ${url} (copied)${sealedNote(sealed)}`, sealed > 0 ? 'error' : undefined)
    }
    for (const tile of this.tilesWith(id)) tile.open = await api.getNode(id)
    await this.refresh()
  }

  /*
   * The server keeps only a hash of the key that lives in the link's fragment,
   * so a link exists in full exactly once: here, in the moment it is minted.
   * There is nothing to re-copy later — asking again mints a new link and
   * retires the old one.
   */
  async mintShareLink(id: number): Promise<{ url: string; sealed: number }> {
    const { path, sealed } = await api.share(id)
    const url = `${location.origin}${path}`
    await this.copy(url)
    return { url, sealed }
  }

  async copy(text: string) {
    try {
      await navigator.clipboard.writeText(text)
      return true
    } catch {
      return false
    }
  }

  /**
   * Import a picked folder (or a zip of one) into the folder being browsed.
   * The report is worth a line of its own: an import that quietly skipped half
   * its files would only be noticed much later.
   */
  async importFolder(files: File[], root = false) {
    const parent = this.parentId
    if (parent === null && !root) return
    if (!root && this.parentSealed && !(await this.ensureUnlocked())) return
    this.say(`importing ${files.length} file${files.length === 1 ? '' : 's'} …`)

    const report = root ? await api.importRoot(files) : await api.importFolder(parent!, files)
    if (root) this.spaces = await api.listSpaces()
    await this.refresh()

    const parts = [
      ...(report.spaces > 0 ? [`${report.spaces} space${report.spaces === 1 ? '' : 's'}`] : []),
      `${report.documents} document${report.documents === 1 ? '' : 's'}`,
      `${report.folders} folder${report.folders === 1 ? '' : 's'}`,
      `${report.files} file${report.files === 1 ? '' : 's'}`,
      `${report.links} link${report.links === 1 ? '' : 's'} rewritten`,
      ...(report.cards > 0 ? [`${report.cards} card${report.cards === 1 ? '' : 's'}`] : []),
    ]
    if (report.skipped.length > 0) parts.push(`${report.skipped.length} skipped`)
    this.say(`imported ${parts.join(', ')}`)
  }

  /**
   * Upload files into the folder being browsed, as nodes of their own. The
   * status line counts up while they go, because a large one takes a while.
   */
  async putFiles(files: File[], encrypt = false) {
    const parent = this.parentId
    if (parent === null) return
    // A sealed folder seals them anyway, and needs the vault open for it.
    if (this.parentSealed) {
      if (!(await this.ensureUnlocked())) return
      encrypt = true
    }
    const total = files.reduce((sum, file) => sum + file.size, 0)
    const what = files.length === 1 ? files[0].name : `${files.length} files`
    this.say(`uploading ${what} (${formatBytes(total)})${encrypt ? ' into the vault' : ''} …`)

    const created = await api.putFiles(parent, files, encrypt, (fraction) =>
      this.say(`uploading ${what} — ${Math.floor(fraction * 100)}%`),
    )
    this.query = ''
    this.globalSearch = false
    await this.refresh(created[0]?.id)
    this.say(created.length === 1 ? `uploaded ${this.here}/${created[0].slug}` : `uploaded ${created.length} files`)
  }

  async upload(files: File[]): Promise<MediaFile[]> {
    const what = files.length === 1 ? files[0].name : `${files.length} files`
    const total = files.reduce((sum, file) => sum + file.size, 0)
    this.say(`uploading ${what} (${formatBytes(total)}) …`)
    const uploaded = await api.upload(files, (fraction) =>
      this.say(`uploading ${what} — ${Math.floor(fraction * 100)}%`),
    )
    this.say(`uploaded ${uploaded.length} file${uploaded.length === 1 ? '' : 's'}`)
    return uploaded
  }

  async loadMedia() {
    this.media = await api.listMedia()
  }

  setVim(enabled: boolean) {
    this.vimEnabled = enabled
    localStorage.setItem('diary:vim', enabled ? 'on' : 'off')
  }

  setClipboard(enabled: boolean) {
    this.clipboard = enabled
    localStorage.setItem('diary:clipboard', enabled ? 'on' : 'off')
  }

  /** Surface a failed API call in the status line instead of the console. */
  async guard(action: () => Promise<unknown>) {
    try {
      await action()
    } catch (error) {
      if (error instanceof ApiError && error.status === 401) {
        this.user = null
        this.notice = 'the session ran out — log in again, and carry on where you were'
        this.say('session expired — log in again', 'error')
      } else if (error instanceof ApiError && error.status === 423) {
        // It locked itself while nobody was looking.
        if (this.vault) this.vault = { ...this.vault, unlocked: false, locks_in: null }
        this.say(error.message, 'error')
      } else {
        this.say(error instanceof Error ? error.message : String(error), 'error')
      }
    }
  }
}

export const workspace = new Workspace()
