import {
  api,
  ApiError,
  isContainer,
  unreachable,
  type Agenda,
  type AgendaCard,
  type Backlink,
  type Board,
  type BoardList,
  type Card,
  type Crumb,
  type DiaryEntry,
  type WorkspaceNode,
  type MediaFile,
  type Moved,
  type NodeSummary,
  type Revision,
  type RevisionSummary,
  type ScratchNote,
  type SpaceOverview,
  type Task,
  type TrashItem,
  type VaultStatus,
} from './api'
import { formatBytes, formatDay, formatStamp, formatWeekday, isTouchDevice, parseDay, sealedNote, type Heading } from './util'
import { AUTOSAVE_MS, Writing, type WritingKind } from './writing.svelte'

export type Pane = 'list' | 'document'
/**
 * What a tile shows: a node of the tree — a document or a file — or one of
 * the views over the boards and the diary, which are tiled beside the
 * documents like any other window.
 */
export type TileKind = 'node' | 'board' | 'agenda' | 'calendar' | 'card'
export type View = Exclude<TileKind, 'node'>
/** What floats over the tiles; one at a time. */
export type Sheet = 'help' | 'media' | 'overview' | 'actions' | 'verify' | 'info' | 'trash'
/** The info sheet's parts: what the open document holds, and what is around it. */
export type InfoSection = 'outline' | 'history' | 'backlinks' | 'cards' | 'share'
export type Flash = { text: string; kind: 'info' | 'error' } | null

/**
 * Where a space was left: the folder, the tiles, the row, the pane. A tile
 * is the id of the node it shows, or the name of the view. `openId` is the
 * focused tile's, all a spot held before there were tiles, and `open` the
 * documents alone, all it held before there were views.
 */
type Spot = {
  parentId: number
  openId: number | null
  cursor: number
  pane: Pane
  open?: number[]
  tiles?: (number | View)[]
  focused?: number
}

export type Draft = { name: string; body: string; created_at: number }

/** `done: x`, and when a repeating card comes back. */
function doneSaid(moved: Moved): string {
  const again = moved.again?.due_at
  return again == null ? `done: ${moved.title}` : `done: ${moved.title} — again ${formatWeekday(again)} ${formatDay(again)}`
}

let tiles = 0

/**
 * One window on screen, and everything that belongs to it rather than to
 * the workspace: what it shows, the draft being written, whether that is
 * saved, and the timer that will save it. A view keeps its state in the
 * workspace — there is one board, one agenda, one calendar — so its tile
 * holds only its kind.
 */
export class Tile {
  readonly id = ++tiles
  kind = $state<TileKind>('node')
  open = $state<WorkspaceNode | null>(null)
  draft = $state<Draft>({ name: '', body: '', created_at: 0 })
  editing = $state(false)
  /** Set when the editor should open in insert mode rather than normal mode. */
  enterInsert = $state(false)
  dirty = $state(false)
  /** The editor's vim mode, for the status line while this tile has focus. */
  vimMode = $state('normal')
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

  constructor(kind: TileKind = 'node') {
    this.kind = kind
  }

  /** Showing nothing: a node tile with no node in it. */
  get empty(): boolean {
    return this.kind === 'node' && !this.open
  }

  /** Put the tile back to showing nothing. */
  clear() {
    this.stopAutosave()
    this.kind = 'node'
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
  /** The cards a search found, listed after the nodes, and the notes after them. */
  foundCards = $state<AgendaCard[]>([])
  foundNotes = $state<ScratchNote[]>([])
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

  /*
   * The tiles focused before the one focused now, the last the latest:
   * closing a tile goes back to the one that had the focus before it — to
   * the board a card's notes were opened from, to the calendar a day was.
   */
  #trail: Tile[] = []

  /** What has the keys: the list, or the kind of the focused tile. */
  get focus(): 'list' | TileKind {
    return this.pane === 'list' || this.tile.empty ? 'list' : this.tile.kind
  }

  /** The tile showing `kind`, if one is. */
  view(kind: View): Tile | undefined {
    return this.tiles.find((tile) => tile.kind === kind)
  }

  /**
   * Focus a tile, and follow it in the list and the address bar, as opening
   * its document did.
   */
  focusTile(index: number) {
    const tile = this.tiles[index]
    if (!tile) return
    if (tile !== this.tile) {
      this.#leaveCard()
      this.#trail = [...this.#trail.filter((t) => t !== tile && t !== this.tile), this.tile].slice(-20)
    }
    this.focused = index
    if (tile.empty) return
    this.pane = 'document'
    if (!tile.open) return
    const row = this.nodes.findIndex((n) => n.id === tile.open?.id)
    if (row >= 0) this.cursor = row
    this.#syncUrl(tile.open.path)
  }

  /** A tile of its own for what opens next, beside what is on screen. */
  #addTile(tile = new Tile()) {
    this.tiles.push(tile)
    this.focusTile(this.tiles.length - 1)
  }

  /**
   * Tiles with nothing in them go, as long as one is left: a closed document
   * closes its tile, as `:q` closes a window in vim. When the focused one
   * went, the focus goes back along the trail.
   */
  prune() {
    const current = this.tile
    if (this.tiles.length > 1) {
      const kept = this.tiles.filter((tile) => !tile.empty)
      this.tiles = kept.length > 0 ? kept : [current]
      const back = this.tiles.includes(current) ? current : [...this.#trail].reverse().find((tile) => this.tiles.includes(tile))
      this.focused = back ? this.tiles.indexOf(back) : Math.min(this.focused, this.tiles.length - 1)
    }
    this.#trail = this.#trail.filter((tile) => this.tiles.includes(tile) && tile !== this.tile)
    if (this.tile.empty) this.pane = 'list'
  }

  /** `:close`: the focused tile goes, or with only one, what it shows. */
  closeTile() {
    this.#leaveCard()
    this.tile.clear()
    this.prune()
    if (!this.tile.empty) this.focusTile(this.focused)
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
    if (!this.tile.empty) this.#addTile()
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
    this.#trail = []
  }

  /** The board as the server sent it, loaded only while it is being looked at. */
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
  /*
   * The scratchpad: a drawer that comes down from under the bar over whatever
   * is on screen, as the desktop's scratchpad does, with the notes taken in
   * passing.
   */
  scratchOpen = $state(false)
  scratchNotes = $state<ScratchNote[]>([])
  scratchCursor = $state(0)

  /** The step of the open card's checklist under the cursor. */
  taskCursor = $state(0)
  /*
   * The short text being written in place, if one is: the description of the
   * card turned over, or the note under the drawer's cursor. One at a time,
   * as there is one place on screen each is written in.
   */
  writing = $state<Writing | null>(null)

  get describing() {
    return this.writing?.kind === 'description'
  }
  get scratchEditing() {
    return this.writing?.kind === 'note'
  }
  /** The note being written has not been saved yet, so it is in no row. */
  get scratchNew() {
    return this.scratchEditing && this.writing?.id === null
  }

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

  /** The sheet over everything, if any; `openSheet` and `closeSheet` move it. */
  overlay = $state<'none' | Sheet>('none')
  /** The part of the info sheet on show; it stays for the next time it opens. */
  infoSection = $state<InfoSection>('outline')
  /**
   * The space whose board the board's tile shows: the one being browsed
   * when it has a board, or else the last one shown, so that coming from the
   * diary does not leave the board's tile empty.
   */
  boardSpaceId = $state<number | null>(null)
  /** `gO`: the headings of what is on screen, to jump between from the keyboard. */
  outline = $state.raw<Heading[]>([])
  outlineCursor = $state(0)

  /*
   * What is due or in hand on every board, kept loaded all the time: the bar
   * counts from it, so what is late is in sight without going to look.
   */
  agenda = $state<Agenda>({ cards: [], diary_today: null, on_this_day: [], today: null })
  agendaCursor = $state(0)

  /** `C`: the day the calendar's cursor is on, `2026-10-20`, and its month's entries. */
  calendarDay = $state('')
  calendarEntries = $state.raw<DiaryEntry[]>([])

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
  /** What `:trash` lists: everything deleted, the last first. */
  trash = $state<TrashItem[]>([])
  trashCursor = $state(0)
  /** The last thing deleted here, which `u` puts back. */
  lastDeleted = $state<number | null>(null)
  /** `:history`: the open document's earlier versions, and the one shown. */
  history = $state<RevisionSummary[]>([])
  historyCursor = $state(0)
  revision = $state.raw<Revision | null>(null)
  /** `:backlinks`: the documents that link to the open one. */
  backlinks = $state<Backlink[]>([])
  backlinksCursor = $state(0)
  /** The info sheet's cards: where its cursor is in the open document's. */
  cardsCursor = $state(0)
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

  /** How many rows the list has to walk: the nodes, then the cards and the notes a search found. */
  get rows(): number {
    return this.nodes.length + this.foundCards.length + this.foundNotes.length
  }

  /** The card a search found under the cursor, past the nodes it found. */
  get foundCard(): AgendaCard | undefined {
    return this.cursor >= this.nodes.length ? this.foundCards[this.cursor - this.nodes.length] : undefined
  }

  /** The note a search found under the cursor, past the cards. */
  get foundNote(): ScratchNote | undefined {
    const at = this.cursor - this.nodes.length - this.foundCards.length
    return at >= 0 ? this.foundNotes[at] : undefined
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

  /**
   * The card the card verbs act on, wherever it is looked at: picked in the
   * info sheet, under the board's cursor or turned over beside it,
   * highlighted in the agenda — or, reading a document, the one open card
   * about it, when there is only one.
   */
  get focusedCard(): Card | AgendaCard | undefined {
    const about = this.open?.cards ?? []
    if (this.overlay === 'info' && this.infoSection === 'cards') return about[this.cardsCursor]
    switch (this.focus) {
      case 'board':
      case 'card':
        return this.card
      case 'agenda':
        return this.agendaCards[this.agendaCursor]
      case 'node': {
        const open = about.filter((card) => card.done_at === null)
        return open.length === 1 ? open[0] : undefined
      }
    }
  }

  /** The focused card, or `undefined` once it has been said why there is none. */
  #needCard(): Card | AgendaCard | undefined {
    const card = this.focusedCard
    if (card) return card
    const open = this.focus === 'node' ? (this.open?.cards ?? []).filter((c) => c.done_at === null).length : 0
    if (open > 1) this.say(`${open} open cards about this — gc picks one`, 'error')
    else if (this.focus === 'node' && this.open?.kind === 'document') this.say('no open card about this document — c makes one', 'error')
    else this.say('no card in focus — :board', 'error')
  }

  /**
   * The agenda in the order it is drawn and walked, bucketed by urgency. The
   * day is the server's, in the workspace's timezone, so a browser on a trip
   * groups the cards the way the morning mail does.
   */
  get agendaGroups(): { key: 'overdue' | 'today' | 'week' | 'doing'; cards: AgendaCard[] }[] {
    const { start, end, week_end } = this.agenda.today ?? { start: 0, end: 0, week_end: 0 }
    const groups = { overdue: [] as AgendaCard[], today: [] as AgendaCard[], week: [] as AgendaCard[], doing: [] as AgendaCard[] }
    for (const card of this.agenda.cards) {
      if (card.due_at !== null && card.due_at < start) groups.overdue.push(card)
      else if (card.due_at !== null && card.due_at < end) groups.today.push(card)
      else if (card.due_at !== null && card.due_at < week_end) groups.week.push(card)
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
      this.openSheet('verify')
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

  /*
   * Autosave. Typing is where work gets lost, so a draft goes to localStorage
   * on every change — cheap, synchronous, survives a killed tab — and to the
   * server a couple of seconds after typing stops.
   */

  static readonly RECENT_DONE_DAYS = 7

  #stashKey(id: number) {
    return `diary:draft:${id}`
  }

  /** Call whenever the draft changes. Replaces a bare `dirty = true`. */
  touch(tile = this.tile) {
    tile.dirty = true
    this.stash(tile)
    tile.stopAutosave()
    tile.autosave = setTimeout(() => void this.#writeBack(tile), AUTOSAVE_MS)
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
      if (this.writing?.dirty) void this.guard(() => this.saveWriting())
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
      if (this.writing?.dirty) await this.guard(() => this.saveWriting())
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

  /** `views` false: the views are leaving with the focus, and are not left behind. */
  #rememberSpot(views = true) {
    if (this.spaceId === null || this.parentId === null) return
    // A card's back is the card under the board's cursor, which is not kept.
    const kept = this.tiles.filter((tile) => !tile.empty && (tile.kind === 'node' || (views && tile.kind !== 'card')))
    this.#spots[this.spaceId] = {
      parentId: this.parentId,
      openId: this.open?.id ?? null,
      cursor: this.cursor,
      pane: this.pane,
      tiles: kept.map((tile) => (tile.kind === 'node' ? tile.open!.id : tile.kind)),
      focused: Math.max(kept.indexOf(this.tile), 0),
    }
    localStorage.setItem('diary:spots', JSON.stringify(this.#spots))
  }

  async enterSpace(id: number) {
    const leaving = this.spaceId !== null && this.spaceId !== id
    /*
     * The views look at every board and the whole diary, so with one of them
     * focused they come along — moved, not copied — as the special workspace
     * once did: a digit on the board shows that space's board, and the
     * agenda stays beside it.
     */
    let carried: Tile[] = []
    let carriedFocus: Tile | undefined
    if (leaving) {
      await this.flush()
      const carry = this.focus !== 'list' && this.focus !== 'node'
      this.#rememberSpot(!carry)
      const from = this.spaces.findIndex((s) => s.id === this.spaceId)
      const to = this.spaces.findIndex((s) => s.id === id)
      this.switchDirection = to >= from ? 1 : -1
      if (carry) {
        carried = this.tiles.filter((tile) => tile.kind !== 'node' && tile.kind !== 'card')
        carriedFocus = this.tile.kind === 'card' ? (this.view('board') ?? carried[0]) : this.tile
      }
      // Each space has tiles of its own; these come back with this one.
      for (const tile of this.tiles) tile.stopAutosave()
      this.tiles = carried.length > 0 ? carried : [new Tile()]
      this.focused = carriedFocus ? this.tiles.indexOf(carriedFocus) : 0
      this.#trail = []
      this.pane = carried.length > 0 ? 'document' : 'list'
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
      const entries =
        spot.tiles ?? spot.open ?? (spot.openId !== null && this.nodes.some((n) => n.id === spot.openId) ? [spot.openId] : [])
      const restored: Tile[] = []
      for (const entry of entries) {
        if (typeof entry === 'string') {
          let tile = this.view(entry)
          if (!tile) this.tiles.push((tile = new Tile(entry)))
          restored.push(tile)
          continue
        }
        const empty = this.tiles.findIndex((tile) => tile.empty)
        if (empty >= 0) this.focused = empty
        else {
          this.tiles.push(new Tile())
          this.focused = this.tiles.length - 1
        }
        try {
          await this.openNode(entry)
          restored.push(this.tile)
        } catch {
          /* deleted or moved away since: one tile fewer */
        }
      }
      this.prune()
      const back = carriedFocus ?? restored[Math.min(spot.focused ?? 0, restored.length - 1)]
      if (back && this.tiles.includes(back)) {
        this.focusTile(this.tiles.indexOf(back))
        if (!carriedFocus) this.pane = spot.pane
      } else {
        this.cursor = Math.min(spot.cursor, Math.max(this.nodes.length - 1, 0))
      }
    }
    this.#trail = []
    await this.#loadViews()
  }

  /** Show the children of `id`, which must be a space or a folder. */
  async goTo(id: number, keep?: number) {
    await this.flush()
    const folder = await api.getNode(id)
    // Walking into another space by a link leaves this one where it was; the
    // views stay on screen, so they go along rather than stay behind too.
    if (this.spaceId !== null && folder.path[0]?.id !== this.spaceId) this.#rememberSpot(false)
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
    if (this.query) {
      const space = this.#searchSpace
      // A note is in no space, so every search looks through them all.
      ;[this.nodes, this.foundCards, this.foundNotes] = await Promise.all([
        api.listNodes({ q: this.query, space }),
        api.searchCards(this.query, space),
        api.scratch(this.query),
      ])
    } else {
      this.nodes = await api.listNodes({ parent: this.parentId ?? undefined })
      this.foundCards = []
      this.foundNotes = []
    }
    const index = this.nodes.findIndex((n) => n.id === keepId)
    this.cursor = index >= 0 ? index : Math.min(this.cursor, Math.max(this.rows - 1, 0))
  }

  /** The space a search keeps to; none for one across every space. */
  get #searchSpace(): number | undefined {
    return this.globalSearch ? undefined : (this.spaceId ?? undefined)
  }

  async search(query: string, global = this.globalSearch) {
    this.query = query
    this.globalSearch = global && query !== ''
    this.cursor = 0
    await this.refresh()
  }

  /**
   * `n` / `N`: the next or previous search result after the cursor, opened
   * beside the results so the next `n` goes on from there. A folder or a card
   * is only highlighted: going to it would end the search. Past the last
   * result it starts again at the first, as vim's `wrapscan` does.
   */
  async nextResult(delta: 1 | -1) {
    if (!this.query) return this.say('no search — / starts one', 'error')
    const count = this.rows
    if (count === 0) return this.say(`no results for "${this.query}"`, 'error')
    const next = this.cursor + delta
    if (next >= count) this.say('search hit BOTTOM, continuing at TOP')
    else if (next < 0) this.say('search hit TOP, continuing at BOTTOM')
    this.cursor = (next + count) % count
    const node = this.selected
    if (node && !isContainer(node)) await this.openNode(node.id)
  }

  move(delta: number) {
    if (this.rows === 0) return
    this.cursor = Math.min(Math.max(this.cursor + delta, 0), this.rows - 1)
  }

  async openNode(id: number, edit = false) {
    if (this.open?.id !== id) await this.flushTile()

    const node = await api.getNode(id)
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
    // A view is never put away by what is opened from it — a card's notes,
    // a day of the calendar, a search from the board: that opens beside it,
    // and closing it goes back there.
    if (this.tile.kind !== 'node') this.#addTile()

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
    const found = this.foundCard
    if (found) return this.turnTo(found)
    const note = this.foundNote
    if (note) return this.showNote(note.id)
    const selected = this.selected
    if (!selected) return
    // A match from anywhere opens beside the results, as one from this space
    // does, so `n` goes on from it; only a folder ends the search, by going in.
    if (isContainer(selected)) await this.goTo(selected.id)
    else await this.openNode(selected.id, edit)
  }

  /**
   * A new document is created immediately so uploads have something to attach
   * to. In the diary it starts from `templates/day` when there is one; the
   * server sees to that, so capture does the same.
   */
  async createDocument(name = '', from: { created_at?: number; template_id?: number } = {}) {
    const parent = this.parentId
    if (parent === null) return
    const node = await api.createNode({ parent_id: parent, name, body: '', ...from })
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
  async save({ quiet = false, force = false, restore = false, tile = this.tile } = {}) {
    if (!tile.open) return
    // Snapshot what goes over the wire: an autosave races the typing that
    // triggered it, and anything typed since must stay marked unsaved.
    const sent = { ...tile.draft, name: tile.draft.name.trim() }
    const saved = await api.updateNode(tile.open.id, {
      ...sent,
      base_updated_at: force ? undefined : tile.open.updated_at,
      restore,
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
    const { removed, purged } = await api.deleteNode(id)
    for (const tile of showing) tile.clear()
    this.prune()
    this.marked = this.marked.filter((n) => n.id !== id)
    await this.refresh()
    const name = node ? `"${node.name.trim() || node.slug}"` : `#${id}`
    if (purged) return this.say(`deleted ${name} for good`)
    this.lastDeleted = id
    const what = removed > 1 ? `${name} and ${removed - 1} inside it` : name
    this.say(`${what} went to the trash — u brings it back`)
  }

  /** `u`: the last delete, undone. */
  async undoDelete() {
    const id = this.lastDeleted
    if (id === null) return this.say('nothing deleted here to undo — :trash has the rest', 'error')
    await this.restore(id)
  }

  /**
   * Out of the trash, back where it was. A space is entered again; anything
   * else in the space being browsed gets the cursor, so `u` lands on what it
   * brought back.
   */
  async restore(id: number) {
    const { node, elsewhere } = await api.restore(id)
    if (this.lastDeleted === id) this.lastDeleted = null
    const name = `"${node.name.trim() || node.slug}"`
    if (node.kind === 'space') {
      this.spaces = await api.listSpaces()
      if (this.overlay !== 'trash') await this.enterSpace(node.id)
      this.say(`restored the space ${name}`)
    } else {
      const parent = node.path.at(-2)
      if (parent && node.path[0]?.id === this.spaceId) {
        if (parent.id === this.parentId && !this.query) await this.refresh(node.id)
        else if (this.overlay !== 'trash') await this.goTo(parent.id, node.id)
      }
      const where = node.path.slice(0, -1).map((crumb) => crumb.slug).join('/')
      this.say(elsewhere ? `restored ${name} to ${where} — the folder it was in is gone` : `restored ${name} to ${where}`)
    }
    if (this.overlay === 'trash') await this.loadTrash()
  }

  /** `:history`: the open document's earlier versions, the newest first. */
  async loadHistory() {
    const open = this.open
    if (!open) return
    // What is typed and not yet written is part of what a revision is
    // compared with, and of what a restore must not lose.
    await this.flushTile()
    this.history = await api.revisions(open.id)
    this.historyCursor = 0
    this.revision = null
    await this.showRevision(0)
  }

  /** Read the revision at `index`, for the overlay to compare with the draft. */
  async showRevision(index: number) {
    const open = this.open
    const summary = this.history[index]
    this.historyCursor = index
    if (!open || !summary) return
    const revision = await api.revision(open.id, summary.id)
    // A later j may have moved on while this one was being read.
    if (this.history[this.historyCursor]?.id === revision.id) this.revision = revision
  }

  /**
   * Bring a revision back: it becomes the document again, and what it
   * replaces is kept as a revision of its own, so a restore is undone the
   * same way. The editor reads the draft when it opens, so this leaves it.
   */
  async restoreRevision(revision: Revision) {
    const tile = this.tile
    if (!tile.open) return
    await this.flushTile(tile)
    tile.editing = false
    tile.draft = { ...tile.draft, name: revision.name, body: revision.body }
    tile.dirty = true
    await this.save({ quiet: true, restore: true, tile })
    this.closeSheet()
    this.say(`restored the version from ${formatStamp(revision.saved_at)} — :history has the one it replaced`)
  }

  /**
   * Follow a `[[title]]`: to the document it names, or — when there is none
   * — to a new one called that, here, after asking. Which document a title
   * names the server says when the text is written, so it is written first.
   */
  async followTitle(title: string) {
    await this.flushTile()
    const path = this.open?.wikilinks[title]
    if (path) return this.openPath(path)
    if (!(await this.ask(`no document called "${title}" yet — write it in ${this.here}?`))) return
    await this.createDocument(title)
  }

  /** `:backlinks`: what links to the open document, the last written first. */
  async loadBacklinks() {
    const open = this.open
    if (!open) return
    // A link typed and not yet written is not one the server knows about.
    await this.flush()
    this.backlinks = await api.backlinks(open.id)
    this.backlinksCursor = 0
  }

  async loadTrash() {
    this.trash = await api.trash()
    this.trashCursor = Math.min(this.trashCursor, Math.max(this.trash.length - 1, 0))
  }

  /** Delete one thing in the trash for good, files and all. */
  async purge(id: number) {
    await api.purge(id)
    if (this.lastDeleted === id) this.lastDeleted = null
    await this.loadTrash()
  }

  async emptyTrash() {
    const { removed } = await api.emptyTrash()
    this.lastDeleted = null
    if (this.overlay === 'trash') await this.loadTrash()
    this.say(removed === 1 ? 'the trash is empty — 1 thing deleted for good' : `the trash is empty — ${removed} things deleted for good`)
  }

  /**
   * Rename anything in the tree — a document, a folder or a space. Only the
   * name is sent, so the text on the server stays as it is and a draft still
   * being typed is saved by autosave as before. When the document is open
   * here, the rename carries the version this tab last saw: one saved
   * somewhere else since is not stamped over, which would let the next
   * autosave write the draft over that save without a word.
   */
  async renameNode(id: number, name: string) {
    const open = this.tilesWith(id)[0]?.open
    const saved = await api.updateNode(id, { name: name.trim(), base_updated_at: open?.updated_at })
    for (const tile of this.tilesWith(id)) {
      tile.open = saved
      tile.draft.name = saved.name
    }
    if (this.path.some((crumb) => crumb.id === id)) this.path = saved.path
    if (saved.kind === 'space') this.spaces = await api.listSpaces()
    await this.reread()
    await this.refresh()
    this.say(`renamed to ${saved.slug}`)
  }

  /**
   * Read the open documents again after a rename or a move: their paths may
   * have changed with it, and their text too, where the server rewrote a
   * link to what moved. One being written is left as it is — saving it is
   * then refused, and `:e!` takes the rewritten text.
   */
  async reread() {
    for (const tile of this.tiles) {
      if (!tile.open || tile.editing || tile.dirty) continue
      const node = await api.getNode(tile.open.id).catch(() => null)
      if (!node) continue
      tile.open = node
      tile.draft = { name: node.name, body: node.body, created_at: node.created_at }
    }
    if (this.open) this.#syncUrl(this.open.path)
  }

  /**
   * Delete a space and everything in it. The app has to land somewhere
   * afterwards, so it falls into the next space — or, if that was the last
   * one, into an empty workspace that `:space! <name>` fills again.
   */
  async deleteSpace(id: number) {
    const { removed } = await api.deleteNode(id)
    this.lastDeleted = id
    for (const tile of this.tiles) tile.clear()
    this.prune()
    this.rawBoard = null
    this.boardSpaceId = null
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
    this.say(`the space and ${removed - 1} node(s) in it went to the trash — u brings it back`)
  }

  /** Reparent the node under the cursor, for `:mv`. */
  async moveNode(id: number, parentId: number) {
    await api.moveNode(id, parentId)
    await this.reread()
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
    if (moved > 0) await this.reread()
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
    this.agendaCursor = Math.min(this.agendaCursor, Math.max(this.agendaCards.length + this.agenda.on_this_day.length - 1, 0))
  }

  /**
   * The agenda opens itself once a day when something is late or due — the
   * point is to be told, not to have to remember to ask.
   */
  async openAgendaIfDue() {
    const today = this.agenda.today?.date
    if (!today || localStorage.getItem('diary:agenda-seen') === today) return
    const { late, today: due } = this.dueCounts
    if (late + due === 0) return
    localStorage.setItem('diary:agenda-seen', today)
    this.agendaCursor = 0
    await this.showView('agenda')
  }

  /** `C`: the calendar, on the day of the entry being read or else today. */
  async openCalendar() {
    const open = this.open
    const reading = open && this.spaces.find((space) => space.id === open.path[0]?.id)?.role === 'diary'
    this.calendarDay = formatDay(reading ? open.created_at : Math.floor(Date.now() / 1000))
    await this.showView('calendar')
  }

  async loadCalendar() {
    this.calendarEntries = await api.diaryDays(this.calendarDay.slice(0, 7))
  }

  /** The calendar's cursor, `days` along or `months` over; a month keeps the day where it can. */
  async moveCalendar(days: number, months = 0) {
    const [y, m, d] = this.calendarDay.split('-').map(Number)
    const last = new Date(y, m - 1 + months + 1, 0).getDate()
    const date = new Date(y, m - 1 + months, Math.min(d, last) + days, 12)
    const before = this.calendarDay.slice(0, 7)
    this.calendarDay = formatDay(Math.floor(date.getTime() / 1000))
    if (this.calendarDay.slice(0, 7) !== before) await this.loadCalendar()
  }

  /** Enter in the calendar: that day's entry, or a new one dated that day. */
  async openCalendarDay() {
    const day = this.calendarDay
    const entry = this.calendarEntries.find((e) => e.day === day)
    // Beside the calendar, so `q` on the entry comes back to it.
    if (entry) return this.openPath(entry.path)
    await this.openDay(day)
  }

  /**
   * `T`, and a day in the calendar without an entry: the server finds the
   * day's entry or starts it, by its own clock and at the root of the diary,
   * as a capture does, so the three never write two entries for one day.
   */
  async openDay(day?: string) {
    const home = this.spaces.find((space) => space.role === 'diary')
    if (!home) return this.say('there is no diary to write in', 'error')
    const entry = await api.diaryDay(day)
    if (this.spaceId !== home.id) await this.enterSpace(home.id)
    else if (this.query) await this.search('')
    if (entry.created) await this.refresh()
    await this.openNode(entry.id, true)
    if (entry.created) {
      // Left as it was started, it goes again, as a new document from `o` does.
      this.tile.fresh = entry.id
      this.enterInsert = true
    }
  }

  /** `[d` / `]d`: the diary entry before or after the one being read. */
  async stepEntry(by: -1 | 1) {
    const open = this.open
    if (!open) return this.say('nothing open', 'error')
    const { prev, next } = await api.diaryAround(open.id)
    const entry = by < 0 ? prev : next
    if (!entry) return this.say(by < 0 ? 'no entry before this one' : 'no entry after this one')
    await this.openPath(entry.path)
    this.say(entry.day)
  }

  /** Enter in the agenda: to the card, on its own board, beside the agenda. */
  async goToCard(card: AgendaCard) {
    // From the agenda's tile, which comes along into the card's space.
    if (this.spaceId !== card.space_id) await this.enterSpace(card.space_id)
    await this.reloadBoard(card.space_id, card.id)
    await this.showView('board')
  }

  /**
   * Enter on a card a search found, or one picked in a document's info
   * sheet: to its board, turned over beside it.
   */
  async turnTo(card: AgendaCard) {
    await this.goToCard(card)
    await this.turnCard()
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

  /*
   * The views — the board, the agenda, the calendar, a card's back — are
   * tiles, opened beside what is on screen. Each is read again whenever it is
   * gone to: it is looked at in passing, and what it says should be what is
   * true now.
   */

  /** `b`, `t`, `C`, Enter on a card: to that view's tile, opening it first when it is not up. */
  async showView(kind: View) {
    await this.#loadView(kind)
    if (!this.view(kind)) {
      // The one empty tile there can be takes it; otherwise it goes beside.
      if (this.tile.empty) this.tile.kind = kind
      else this.tiles.push(new Tile(kind))
    }
    this.focusTile(this.tiles.indexOf(this.view(kind)!))
    if (kind === 'board' && this.boardSpaceId === null) this.say('no space has a board — :set board turns one on', 'error')
  }

  /** The views `S` puts up together, in the order they are tiled. */
  static readonly PLAN: View[] = ['board', 'agenda', 'calendar']

  /**
   * `S`: the board, the agenda and the calendar, tiled ahead of the
   * documents with the board focused — or, when all three are up, every view
   * put away again.
   */
  async togglePlan() {
    if (Workspace.PLAN.every((kind) => this.view(kind))) {
      for (const tile of this.tiles) if (tile.kind !== 'node') tile.clear()
      this.prune()
      if (!this.tile.empty) this.focusTile(this.focused)
      return
    }
    await Promise.all(Workspace.PLAN.map((kind) => this.#loadView(kind)))
    const current = this.tile
    const missing = Workspace.PLAN.filter((kind) => !this.view(kind)).map((kind) => new Tile(kind))
    this.tiles = [...missing, ...this.tiles]
    this.focused = this.tiles.indexOf(current)
    this.prune()
    this.focusTile(this.tiles.indexOf(this.view('board')!))
    if (this.boardSpaceId === null) this.say('no space has a board — :set board turns one on', 'error')
  }

  /** Read again what a view shows. */
  async #loadView(kind: View) {
    if (kind === 'board' || kind === 'card') await this.#loadBoard()
    else if (kind === 'agenda') await this.loadAgenda()
    else {
      if (!this.calendarDay) this.calendarDay = formatDay(Math.floor(Date.now() / 1000))
      await this.loadCalendar()
    }
  }

  /** Every view on screen read again, after a space switch brought them. */
  async #loadViews() {
    if (this.view('board') || this.view('card')) await this.#loadBoard()
    if (this.view('calendar')) await this.#loadView('calendar')
  }

  /**
   * The board of the space being browsed, or else the one shown last, or
   * else the first space's that has one; null when no space has a board.
   */
  async #loadBoard() {
    const space = this.space
    const shown = this.boardSpace?.has_board ? this.boardSpace : this.spaces.find((s) => s.has_board)
    const board = space?.has_board ? space : shown
    if (!board) {
      this.rawBoard = null
      this.boardSpaceId = null
      return
    }
    // Another space's board starts at its top left, as a space switch did.
    if (board.id !== this.boardSpaceId) {
      this.listCursor = 0
      this.cardCursor = 0
    }
    await this.reloadBoard(board.id, board.id === this.boardSpaceId ? this.card?.id : undefined)
  }

  get boardSpace(): NodeSummary | undefined {
    return this.spaces.find((space) => space.id === this.boardSpaceId)
  }

  async reloadBoard(spaceId = this.boardSpaceId ?? this.spaceId, keep = this.card?.id) {
    if (spaceId === null) return
    this.rawBoard = await api.board(spaceId)
    this.boardSpaceId = spaceId
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
    await this.reloadBoard(this.boardSpaceId, card.id)
  }

  /**
   * `H`/`L`: the same card, one list over — the move a board is really for.
   * From the agenda or a document too, without going to the board.
   */
  async sendCard(delta: number, card = this.#needCard()) {
    if (!card) return
    const lists = await this.#listsOf(card)
    await this.#sendTo(card, lists, lists.findIndex((list) => list.id === card.list_id) + delta)
  }

  /** The digits on the board: the card under the cursor to that list. */
  async sendCardTo(index: number) {
    const card = this.card
    if (!card || index === this.listCursor) return
    await this.#sendTo(card, this.board?.lists ?? [], index)
  }

  /** The lists of a card's board: the one shown, or read for it. */
  async #listsOf(card: Card | AgendaCard): Promise<BoardList[]> {
    const shown = this.board?.lists ?? []
    if (shown.some((list) => list.id === card.list_id)) return shown
    return 'space_id' in card ? (await api.board(card.space_id)).lists : []
  }

  /*
   * The last list is "done": the server stamps a card finished as it arrives
   * there and clears the stamp as it leaves, so moving is all there is to it.
   */
  async #sendTo(card: Card, lists: BoardList[], index: number) {
    const target = lists[index]
    if (!target) return this.say(`"${card.title}" is already in the ${index < 0 ? 'first' : 'last'} list`, 'error')
    const moved = await api.moveCard(card.id, target.id)
    await this.#cardChanged(card)
    this.say(index === lists.length - 1 ? doneSaid(moved) : `"${card.title}" → ${target.name}`)
  }

  /**
   * After a card changed, wherever it was changed from: the board, the
   * agenda and the open documents read it again, and the cursor that was on
   * it stays on it.
   */
  async #cardChanged(card: Card) {
    const shown = this.rawBoard?.lists.some((list) => list.cards.some((c) => c.id === card.id)) ?? false
    const here = this.focus === 'board' || this.focus === 'card'
    if (shown) await this.reloadBoard(this.boardSpaceId, here ? card.id : this.card?.id)
    if (!shown || this.focus === 'agenda') {
      await this.loadAgenda()
      // It may have changed group; the cursor goes with it.
      const at = this.agendaCards.findIndex((c) => c.id === card.id)
      if (at >= 0 && this.focus === 'agenda') this.agendaCursor = at
    }
    await this.#reloadCardsAbout()
    if (this.foundCards.some((c) => c.id === card.id)) this.foundCards = await api.searchCards(this.query, this.#searchSpace)
    const at = this.open?.cards.findIndex((c) => c.id === card.id) ?? -1
    if (at >= 0) this.cardsCursor = at
  }

  /** The cards about each document open in a tile, read again. */
  async #reloadCardsAbout() {
    await Promise.all(
      this.tiles.map(async (tile) => {
        const id = tile.open?.kind === 'document' ? tile.open.id : null
        if (id === null) return
        const cards = await api.nodeCards(id).catch(() => null)
        if (cards && tile.open?.id === id) tile.open.cards = cards
      }),
    )
  }

  /**
   * A card lands in the first list — the inbox — of this space's board, or of
   * the first space that has one, without leaving the view you are in: writing
   * a thought down must not cost the place you were at. On the board itself
   * it goes on the list under the cursor instead.
   */
  async createCard(title: string, nodeId: number | null = null, body = '') {
    if (this.focus === 'board' || this.focus === 'card') {
      const list = this.list
      if (!list) return this.say('no list to put it on', 'error')
      const card = await api.createCard(list.id, { title, node_id: nodeId, body })
      await this.reloadBoard(this.boardSpaceId, card.id)
      await this.#reloadCardsAbout()
      return this.say(nodeId === null ? `carded "${card.title}"` : `carded "${card.title}" → #${nodeId}`)
    }

    const space = this.space?.has_board ? this.space : this.spaces.find((s) => s.has_board)
    if (!space) return this.say('no space has a board — :set board turns one on', 'error')
    const inbox = (await api.board(space.id)).lists[0]
    if (!inbox) return this.say(`${space.slug} has no lists — :board, then :list <name>`, 'error')
    const card = await api.createCard(inbox.id, { title, node_id: nodeId, body })
    await this.loadAgenda()
    await this.#reloadCardsAbout()
    this.say(`carded "${card.title}" → ${space.slug}/${inbox.name}`)
  }

  /** A change to the focused card — or to `card`, when it is known which. */
  async patchCard(patch: Parameters<typeof api.updateCard>[1], card = this.#needCard()) {
    if (!card) return
    const updated = await api.updateCard(card.id, patch)
    await this.#cardChanged(card)
    return updated
  }

  /** Space: into the done list, or back out of it to the inbox. */
  async toggleDone(card = this.#needCard()) {
    if (!card) return
    const done = card.done_at === null
    const moved = await api.updateCard(card.id, { done })
    await this.#cardChanged(card)
    this.say(done ? doneSaid(moved) : `reopened: ${card.title}`)
  }

  async deleteCard() {
    const card = this.card
    if (!card) return this.say('no card selected', 'error')
    await api.deleteCard(card.id)
    await this.reloadBoard(this.boardSpaceId, undefined)
    this.say(`deleted "${card.title}"`)
  }

  /*
   * The card sheet: one card opened up over its board, the way a card on a
   * paper board is turned over to read its back — the description, the
   * checklist, the labels. It is the card under the board's cursor, so the
   * board keys that act on a card (`D`, `H`/`L`, `:repeat`) act on it here
   * too, and the cursor follows it wherever a move puts it.
   */

  /** Enter on a card: its back, in a tile beside the board. */
  async turnCard() {
    if (!this.card) return this.say('no card selected', 'error')
    this.taskCursor = 0
    await this.showView('card')
  }

  /*
   * Writing in place. A card's description and a note are written the way a
   * document is: saved by themselves a moment after the typing stops, `:w`
   * sooner, and `:q` or Esc leaves with what is still waiting written first.
   */

  /** Begin writing `kind`: what it says on the server, unless this browser kept newer. */
  #write(kind: WritingKind, id: number | null, text: string, write: (text: string, writing: Writing) => Promise<void>) {
    const writing: Writing = new Writing(
      kind,
      id,
      text,
      (text) => write(text, writing),
      (error, retry) =>
        unreachable(error)
          ? this.say(`not saved: the server cannot be reached — kept here, trying again in ${retry / 1000}s`, 'error')
          : void this.guard(() => Promise.reject(error)),
    )
    this.writing = writing
    if (writing.recovered) this.say(`recovered unsaved ${kind} — :w keeps it, :q! drops it`, 'error')
  }

  /** `:w`, Ctrl-S: what is written goes now rather than in a moment. */
  async saveWriting() {
    const writing = this.writing
    if (!writing) return
    await writing.save()
    if (writing.kind === 'description') this.say('description written')
  }

  /**
   * `:q`, Esc in normal mode: leave, with what is still waiting written
   * first; `:q!` lets that go instead. False, and a word why, when writing
   * failed, so the text stays on screen. A note left empty is not kept.
   */
  async leaveWriting(discard = false): Promise<boolean> {
    const writing = this.writing
    if (!writing) return true
    if (discard) writing.discard()
    else if (!(await this.guard(() => writing.save()))) return false
    if (this.writing === writing) this.writing = null
    const note = this.scratchNotes.find((n) => n.id === writing.id)
    if (writing.kind === 'note' && note && !note.body.trim()) await this.guard(() => this.dropScratchNote(false, note))
    return true
  }

  /** Leaving a card's back, or the drawer, while something is written there keeps it, as leaving a document's tile does. */
  #leaveCard() {
    if (this.tile.kind === 'card' && this.describing) void this.leaveWriting()
  }

  get scratchNote(): ScratchNote | undefined {
    return this.scratchNotes[this.scratchCursor]
  }

  /** `Q`: the drawer down, read again, or up and away. */
  async toggleScratch() {
    if (this.scratchOpen) return this.closeScratch()
    this.scratchNotes = await api.scratch()
    this.scratchCursor = Math.min(this.scratchCursor, Math.max(this.scratchNotes.length - 1, 0))
    this.scratchOpen = true
  }

  async closeScratch() {
    if (this.scratchEditing && !(await this.leaveWriting())) return
    this.scratchOpen = false
  }

  /** `o`: a new note at the top, written straight away. */
  newScratchNote() {
    this.#writeNote(null, '')
  }

  /** `e`, `i`, Enter: write the note under the cursor. */
  editScratchNote() {
    const note = this.scratchNote
    if (!note) return this.newScratchNote()
    this.#writeNote(note.id, note.body)
  }

  /*
   * A new note is only made once it says something. The note touched last
   * goes to the top of the list, and the cursor with it.
   */
  #writeNote(id: number | null, body: string) {
    this.#write('note', id, body, async (text, writing) => {
      if (writing.id === null) {
        if (!text.trim()) return
        const note = await api.createScratch(text)
        writing.id = note.id
        this.scratchNotes = [note, ...this.scratchNotes]
      } else {
        const saved = await api.updateScratch(writing.id, text)
        this.scratchNotes = [saved, ...this.scratchNotes.filter((n) => n.id !== saved.id)]
      }
      this.scratchCursor = 0
    })
  }

  async dropScratchNote(say = true, note = this.scratchNote) {
    if (!note) return
    await api.deleteScratch(note.id)
    this.#forgetNote(note.id)
    if (say) this.say('scratch note dropped')
  }

  /** A note gone from the drawer and from the search results, the cursors kept in range. */
  #forgetNote(id: number) {
    this.scratchNotes = this.scratchNotes.filter((n) => n.id !== id)
    this.scratchCursor = Math.min(this.scratchCursor, Math.max(this.scratchNotes.length - 1, 0))
    if (!this.foundNotes.some((n) => n.id === id)) return
    this.foundNotes = this.foundNotes.filter((n) => n.id !== id)
    this.cursor = Math.min(this.cursor, Math.max(this.rows - 1, 0))
  }

  /** A note found by a search: the drawer down, with it under the cursor. */
  async showNote(id: number) {
    if (!this.scratchOpen) await this.toggleScratch()
    else this.scratchNotes = await api.scratch()
    const at = this.scratchNotes.findIndex((note) => note.id === id)
    if (at < 0) return this.say('that note is gone', 'error')
    this.scratchCursor = at
  }

  /** The note under the cursor, saved and closed if it was being written, for it to leave. */
  async #leavingNote(): Promise<ScratchNote | undefined> {
    if (this.scratchEditing && !(await this.leaveWriting())) return
    const note = this.scratchNote
    if (!note) this.say('no scratch note under the cursor', 'error')
    return note
  }

  /** A note split as a card or a document takes it: the first line, a `# heading` or not, and the rest. */
  #split(note: ScratchNote): { title: string; rest: string } {
    const [first, ...rest] = note.body.replace(/^\s+/, '').split('\n')
    return { title: first.replace(/^#+\s*/, '').trim().slice(0, 200), rest: rest.join('\n').replace(/^\n+/, '') }
  }

  /**
   * `C`: the note under the cursor made a card, where `:card` would put one
   * — the list under the board's cursor, or this space's inbox. The first
   * line is its title, and the rest its description.
   */
  async cardScratchNote() {
    const note = await this.#leavingNote()
    if (!note) return
    const { title, rest } = this.#split(note)
    if (!title) return this.say('the note has nothing to call a card by', 'error')
    // Made first, dropped after: a failure in between leaves two copies, never none.
    await this.createCard(title, null, rest)
    await this.dropScratchNote(false, note)
  }

  /**
   * `:append <document>`: the note under the cursor put at the end of a
   * document, named as a link names it, and taken off the scratchpad. Open
   * drafts are written first, and a tile showing the document reads it again.
   */
  async appendScratchNote(document: string) {
    const note = await this.#leavingNote()
    if (!note) return
    await this.flush()
    const space = this.space?.role === 'diary' ? undefined : this.space?.slug
    const landed = await api.capture({ text: note.body, to: 'document', document, space, note: note.id })
    this.#forgetNote(note.id)
    await this.#reread(landed.id)
    this.say(`appended to ${landed.where}`)
  }

  /** `:today <text>`: a line at the end of today's diary entry, without going there. */
  async logToday(text: string) {
    await this.flush()
    const landed = await api.capture({ text, to: 'diary' })
    await this.#reread(landed.id)
    this.say(`logged in ${landed.where}`)
  }

  /** Every tile showing `id` reads it again, after it was written somewhere else. */
  async #reread(id: number) {
    for (const tile of this.tiles) {
      if (tile.open?.id !== id || tile.dirty) continue
      const node = await api.getNode(id)
      tile.open = node
      tile.draft = { name: node.name, body: node.body, created_at: node.created_at }
    }
  }

  /** `:scratch <text>`: a note from the command line, without the drawer. */
  async scratch(text: string) {
    const note = await api.createScratch(text)
    this.scratchNotes = [note, ...this.scratchNotes.filter((n) => n.id !== note.id)]
    this.say('scratched — Q shows the scratchpad')
  }

  /**
   * `:keep [title]`: the note under the cursor made a document in the folder
   * being browsed, and taken off the scratchpad. Without a title, the first
   * line is the title — a `# heading` or not — and the rest the body.
   */
  async keepScratchNote(title = '') {
    const note = await this.#leavingNote()
    if (!note) return
    const parent = this.parentId ?? this.spaceId ?? this.spaces.find((space) => space.role !== 'diary')?.id
    if (parent === undefined || parent === null) return this.say('no space to keep it in', 'error')

    const split = this.#split(note)
    const name = title.trim() || split.title
    const body = title.trim() ? note.body : split.rest
    const node = await api.createNode({ parent_id: parent, name, body })
    // Made first, dropped after: a failure in between leaves two copies, never none.
    await this.dropScratchNote(false, note)
    this.scratchOpen = false
    await this.refresh(node.id)
    await this.openNode(node.id)
    this.say(`kept as ${node.path.map((crumb) => crumb.slug).join('/')}`)
  }

  openSheet(sheet: Sheet) {
    this.overlay = sheet
  }

  /** Esc, q, a click beside it: the sheet away. */
  closeSheet() {
    this.overlay = 'none'
  }

  /** `e`: write the description of the card turned over, from what it says now. */
  describe() {
    const card = this.card
    if (!card) return
    // Named, as the board's cursor may be on another card by the time it saves.
    this.#write('description', card.id, card.body, (body) => this.patchCard({ body }, card).then(() => {}))
  }

  /*
   * The checklist and the labels are written back whole, so two keys pressed
   * faster than the board reloads would each start from the card as it was,
   * and the second would undo the first. Each change waits for the one
   * before it, and reads the card as that one left it.
   */
  private cardWrites: Promise<unknown> = Promise.resolve()
  private inTurn(change: () => Promise<unknown>): Promise<void> {
    const next = this.cardWrites.then(change)
    this.cardWrites = next.catch(() => {})
    return next.then(() => {})
  }

  /** The checklist written back whole, with the cursor on step `at`. */
  async writeChecklist(checklist: Task[], at = this.taskCursor) {
    await this.patchCard({ checklist }, this.card)
    this.taskCursor = Math.min(Math.max(at, 0), Math.max(checklist.length - 1, 0))
  }

  /** `o` / `O`: a new step below or above the cursor. */
  addTask(text: string, above = false) {
    return this.inTurn(async () => {
      const card = this.card
      if (!card) return this.say('no card selected', 'error')
      const tasks = [...card.checklist]
      const at = tasks.length === 0 ? 0 : above ? this.taskCursor : this.taskCursor + 1
      tasks.splice(at, 0, { text, done: false })
      await this.writeChecklist(tasks, at)
    })
  }

  /** Space: tick the step under the cursor, or untick it. */
  toggleTask() {
    return this.inTurn(async () => {
      const tasks = this.card?.checklist ?? []
      const task = tasks[this.taskCursor]
      if (!task) return this.say('no step here — o adds one', 'error')
      const next = tasks.map((t, i) => (i === this.taskCursor ? { ...t, done: !t.done } : t))
      await this.writeChecklist(next)
      this.say(`${next.filter((t) => t.done).length}/${next.length} done`)
    })
  }

  renameTask(text: string) {
    return this.inTurn(async () => {
      const tasks = this.card?.checklist ?? []
      if (!tasks[this.taskCursor]) return this.say('no step here — o adds one', 'error')
      await this.writeChecklist(tasks.map((t, i) => (i === this.taskCursor ? { ...t, text } : t)))
    })
  }

  deleteTask() {
    return this.inTurn(async () => {
      const tasks = this.card?.checklist ?? []
      const task = tasks[this.taskCursor]
      if (!task) return this.say('no step here', 'error')
      await this.writeChecklist(tasks.filter((_, i) => i !== this.taskCursor))
      this.say(`dropped "${task.text}"`)
    })
  }

  /** `J` / `K`: the step takes the cursor with it. */
  nudgeTask(delta: number) {
    return this.inTurn(async () => {
      const tasks = [...(this.card?.checklist ?? [])]
      const to = this.taskCursor + delta
      if (!tasks[this.taskCursor] || to < 0 || to >= tasks.length) return
      ;[tasks[this.taskCursor], tasks[to]] = [tasks[to], tasks[this.taskCursor]]
      await this.writeChecklist(tasks, to)
    })
  }

  /** `j`/`k`, and `gg`/`G` as ±Infinity — in turn too, so a move lands on the steps as written. */
  moveTask(delta: number) {
    return this.inTurn(async () => {
      const count = this.card?.checklist.length ?? 0
      this.taskCursor = Math.min(Math.max(this.taskCursor + delta, 0), Math.max(count - 1, 0))
    })
  }

  /** `:label work`: on the card, or off it when it is on already. */
  toggleLabel(name: string) {
    return this.inTurn(async () => {
      const card = this.#needCard()
      if (!card) return
      const label = name.trim().toLowerCase()
      const on = card.labels.includes(label)
      await this.patchCard({ labels: on ? card.labels.filter((l) => l !== label) : [...card.labels, label] }, card)
      this.say(on ? `${label} off` : `${label} on`)
    })
  }

  /** Every label on the board, for Tab on `:label`. */
  get boardLabels(): string[] {
    const all = (this.rawBoard?.lists ?? []).flatMap((list) => list.cards.flatMap((card) => card.labels))
    return [...new Set(all)].sort()
  }

  async createList(name: string) {
    if (this.boardSpaceId === null) return
    this.rawBoard = await api.createList(this.boardSpaceId, name)
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
    const space = this.boardSpaceId
    if (!card || space === null) return
    let nodeId = card.node_id
    if (nodeId !== null) {
      // Deleted notes read as gone, but the card still points at them, so
      // the way back to them is named rather than a bare "not found".
      try {
        await api.getNode(nodeId)
      } catch (error) {
        if (error instanceof ApiError && error.status === 404) {
          return this.say(`the notes of "${card.title}" are gone — :trash may still have them`, 'error')
        }
        throw error
      }
    }
    if (nodeId === null) {
      const node = await api.createNode({ parent_id: space, name: card.title, body: '' })
      await api.updateCard(card.id, { node_id: node.id })
      nodeId = node.id
    }
    if (this.parentId !== space) await this.goTo(space)
    else await this.refresh()
    // The notes open beside the board, and `q` takes their tile away again
    // on the way back to it.
    await this.openNode(nodeId, card.node_id === null)
    if (card.node_id === null) this.enterInsert = true
  }

  /** Turn a space's board on or off. The diary ships with it off. */
  async setBoard(on: boolean) {
    const space = this.space
    if (!space) return
    await api.updateNode(space.id, { has_board: on })
    this.spaces = await api.listSpaces()
    if (on) await this.showView('board')
    else if (this.boardSpaceId === space.id) {
      this.rawBoard = null
      this.boardSpaceId = null
      if (this.view('board') || this.view('card')) await this.#loadBoard()
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

  /** `:link`: a new link for a shared document, copied; the old one stops working. */
  async renewShareLink(id: number) {
    const { url, sealed } = await this.mintShareLink(id)
    this.say(`new link → ${url} (copied) — the previous one is dead${sealedNote(sealed)}`, sealed > 0 ? 'error' : undefined)
    for (const tile of this.tilesWith(id)) tile.open = await api.getNode(id)
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
        if (this.vault) this.vault = { ...this.vault, unlocked: false, locks_in: null }
        this.say(error.message, 'error')
      } else {
        this.say(error instanceof Error ? error.message : String(error), 'error')
      }
      return false
    }
  }
}

export const workspace = new Workspace()
