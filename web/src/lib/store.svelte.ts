import {
  api,
  ApiError,
  isContainer,
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
import { formatBytes } from './util'

export type Pane = 'list' | 'document'
/** The tree of documents, or the board of the space being browsed. */
export type View = 'tree' | 'board'
export type Flash = { text: string; kind: 'info' | 'error' } | null

/** Where a space was left: the folder, the open document, the row, the pane. */
type Spot = { parentId: number; openId: number | null; cursor: number; pane: Pane }

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

  /** Children of the current folder, or the results of a search. */
  nodes = $state<NodeSummary[]>([])
  cursor = $state(0)
  query = $state('')

  open = $state<WorkspaceNode | null>(null)
  draft = $state({ name: '', body: '', created_at: 0 })
  editing = $state(false)
  /** Set when the editor should open in insert mode rather than normal mode. */
  enterInsert = $state(false)
  dirty = $state(false)

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
   * Set when a card's document was opened from the board, so closing the
   * document goes back to the board — on the same card — rather than leaving
   * you in a tree you never asked for.
   */
  returnTo = $state<'board' | null>(null)

  /*
   * The node picked up by `m`, waiting for the `p` that drops it into whatever
   * folder the list is showing then. A move is two keystrokes apart in time
   * because the destination is somewhere else in the tree — there is no way to
   * name it and be somewhere else at once.
   */
  marked = $state<NodeSummary | null>(null)

  pane = $state<Pane>('list')
  vimMode = $state('normal')
  flash = $state<Flash>(null)

  overlay = $state<'none' | 'help' | 'media' | 'overview' | 'agenda'>('none')

  /*
   * What is due or in hand on every board, kept loaded all the time: the bar
   * counts from it, so what is late is in sight without going to look.
   */
  agenda = $state<Agenda>({ cards: [], diary_today: null })
  agendaCursor = $state(0)

  /** A search across every space rather than the one being browsed — Ctrl-P. */
  globalSearch = $state(false)

  /*
   * The document `o` just made. If it is left with neither a name nor a word
   * in it, it is removed on the way out rather than left as an empty row.
   */
  #fresh: number | null = null

  /*
   * A `[y/N]` question in the command line's place, instead of the browser's
   * confirm(): it names the thing about to go, and it is answered from the
   * keyboard without a modal the rest of the page cannot see past.
   */
  prompt = $state<{ text: string; resolve: (yes: boolean) => void } | null>(null)
  /** A password asked for in the same place, typed into a masked field. */
  secret = $state<{ text: string; resolve: (value: string | null) => void } | null>(null)
  /** The vault, as last heard from the server; null until asked. */
  vault = $state<VaultStatus | null>(null)
  media = $state<MediaFile[]>([])
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

  vimEnabled = $state(localStorage.getItem('diary:vim') !== 'off')

  /** Mirror vim yanks and deletes into the system clipboard, like `set clipboard=unnamed`. */
  clipboard = $state(localStorage.getItem('diary:clipboard') !== 'off')

  /** Pending debounced write, if the draft has changed since the last save. */
  #autosave: ReturnType<typeof setTimeout> | null = null

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

  /** A password, or null when it was waved away with Escape. */
  askSecret(text: string): Promise<string | null> {
    this.secret?.resolve(null)
    return new Promise((resolve) => {
      this.secret = {
        text,
        resolve: (value) => {
          this.secret = null
          resolve(value)
        },
      }
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
    this.vault = await api.unlockVault(password)
    this.say('vault unlocked')
    return true
  }

  /** Seal what is under the cursor, or take it back out, and show the result. */
  async seal(id: number, encrypt: boolean) {
    const { files, unchanged } = await api.seal(id, encrypt)
    const verb = encrypt ? 'encrypted' : 'decrypted'
    const already = unchanged > 0 ? `, ${unchanged} already were` : ''
    this.say(`${verb} ${files} file${files === 1 ? '' : 's'}${already}`)
    await this.refresh()
    if (this.open?.kind === 'file') this.open = await api.getNode(this.open.id)
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
  touch() {
    this.dirty = true
    this.stash()
    if (this.#autosave) clearTimeout(this.#autosave)
    this.#autosave = setTimeout(() => void this.#writeBack(), Workspace.AUTOSAVE_MS)
  }

  stash() {
    if (!this.open) return
    try {
      localStorage.setItem(this.#stashKey(this.open.id), JSON.stringify(this.draft))
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

  #readStash(id: number): typeof this.draft | null {
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

  async #writeBack() {
    this.#autosave = null
    if (!this.open || !this.dirty) return
    await this.guard(() => this.save({ quiet: true }))
  }

  /**
   * Write out a pending draft before doing something that would replace it.
   * Without this, clicking another document in the list discarded unsaved work
   * without a word.
   */
  async flush() {
    if (this.#autosave) {
      clearTimeout(this.#autosave)
      this.#autosave = null
    }
    if (await this.abandonIfBlank()) return
    if (this.open && this.dirty) await this.guard(() => this.save({ quiet: true }))
  }

  /**
   * Drop the document `o` made if nothing was ever put in it. True when it
   * went, so the caller knows there is nothing open any more.
   */
  async abandonIfBlank(): Promise<boolean> {
    const open = this.open
    if (!open || open.id !== this.#fresh) return false
    if (this.draft.name.trim() || this.draft.body.trim()) return false
    this.#fresh = null
    if (this.#autosave) {
      clearTimeout(this.#autosave)
      this.#autosave = null
    }
    this.dropStash(open.id)
    this.open = null
    this.editing = false
    this.dirty = false
    this.pane = 'list'
    try {
      await api.deleteNode(open.id)
    } catch {
      /* already gone, or the server is away; an empty row is no great loss */
    }
    await this.refresh()
    return true
  }

  async boot() {
    try {
      const { user } = await api.me()
      this.user = user
      await this.loadSpaces(Workspace.addressedPath())
      await this.loadAgenda()
      await this.loadVault()
      await this.openAgendaIfDue()
    } catch {
      this.user = null
    } finally {
      this.booting = false
    }
  }

  async login(username: string, password: string) {
    const { user } = await api.login(username, password)
    this.user = user
    await this.loadSpaces(Workspace.addressedPath())
    await this.loadAgenda()
    await this.loadVault()
    await this.openAgendaIfDue()
  }

  async logout() {
    await api.logout()
    this.user = null
    this.vault = null
    this.spaces = []
    this.nodes = []
    this.open = null
    this.editing = false
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
    this.#spots[this.spaceId] = {
      parentId: this.parentId,
      openId: this.open?.id ?? null,
      cursor: this.cursor,
      pane: this.pane,
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
      this.returnTo = null
      this.open = null
      this.editing = false
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
      if (spot.openId !== null && this.nodes.some((n) => n.id === spot.openId)) {
        await this.openNode(spot.openId)
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
    this.query = query
    this.globalSearch = global && query !== ''
    this.cursor = 0
    await this.refresh()
  }

  move(delta: number) {
    if (this.nodes.length === 0) return
    this.cursor = Math.min(Math.max(this.cursor + delta, 0), this.nodes.length - 1)
  }

  async openNode(id: number, edit = false) {
    if (this.open?.id !== id) await this.flush()

    const node = await api.getNode(id)
    if (isContainer(node)) {
      await this.goTo(node.id)
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
    this.#fresh = node.id
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

  async save({ quiet = false } = {}) {
    if (!this.open) return
    // Snapshot what goes over the wire: an autosave races the typing that
    // triggered it, and anything typed since must stay marked unsaved.
    const sent = { ...this.draft, name: this.draft.name.trim() }
    const saved = await api.updateNode(this.open.id, sent)

    this.open = saved
    this.dirty =
      this.draft.body !== sent.body ||
      this.draft.name.trim() !== sent.name ||
      this.draft.created_at !== sent.created_at
    if (this.dirty) this.touch()
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
    if (this.#autosave) {
      clearTimeout(this.#autosave)
      this.#autosave = null
    }
    this.dropStash(id)
    const node = this.open?.id === id ? this.open : this.nodes.find((n) => n.id === id)
    const { removed } = await api.deleteNode(id)
    if (this.open?.id === id) {
      this.open = null
      this.editing = false
      this.dirty = false
      this.pane = 'list'
    }
    if (this.marked?.id === id) this.marked = null
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
    if (this.open?.id === id) {
      this.open = saved
      this.draft.name = saved.name
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
    this.open = null
    this.editing = false
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
    const node = this.marked ?? this.selected
    if (!space) return this.say(`no space ${index + 1}`, 'error')
    if (!node) return this.say('nothing selected', 'error')
    if (node.parent_id === space.id) return this.say(`already in ${space.slug}`)

    if (this.open?.id === node.id) {
      await this.flush()
      this.open = null
      this.editing = false
      this.pane = 'list'
    }
    await api.moveNode(node.id, space.id)
    this.marked = null
    await this.refresh()
    this.spaces = await api.listSpaces()
    this.say(`moved ${node.name.trim() || node.slug} → ${index + 1} ${space.slug}`)
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

  async loadOverview() {
    this.overview = await api.overview()
  }

  /** `m`: pick the node under the cursor up, or put it back down unmoved. */
  mark() {
    const node = this.selected
    if (!node) return this.say('nothing selected', 'error')
    if (this.marked?.id === node.id) {
      this.marked = null
      return this.say('unmarked')
    }
    this.marked = node
    this.say(`marked ${node.name.trim() || node.slug} — p drops it in this folder`)
  }

  /** `p`: drop the marked node into the folder the list is showing. */
  async drop() {
    const node = this.marked
    const parent = this.parentId
    if (!node) return this.say('nothing marked — press m on a row first', 'error')
    if (parent === null) return this.say('no folder open', 'error')
    if (node.parent_id === parent) {
      this.marked = null
      return this.say('already here')
    }

    await api.moveNode(node.id, parent)
    this.marked = null
    await this.refresh(node.id)
    this.say(`moved ${node.name.trim() || node.slug} here`)
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
    await this.openNode(nodeId, card.node_id === null)
    if (card.node_id === null) this.enterInsert = true
    this.returnTo = 'board'
  }

  /** Back to the board a card's document was opened from. */
  async returnToBoard() {
    this.returnTo = null
    this.open = null
    this.editing = false
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
    if (node.shared) {
      await api.unshare(id)
      this.say(`#${id} is private again`)
    } else {
      const url = await this.mintShareLink(id)
      this.say(`shared → ${url} (copied)`)
    }
    if (this.open?.id === id) this.open = await api.getNode(id)
    await this.refresh()
  }

  /*
   * The server keeps only a hash of the key that lives in the link's fragment,
   * so a link exists in full exactly once: here, in the moment it is minted.
   * There is nothing to re-copy later — asking again mints a new link and
   * retires the old one.
   */
  async mintShareLink(id: number) {
    const { path } = await api.share(id)
    const url = `${location.origin}${path}`
    await this.copy(url)
    return url
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
  async importFolder(files: File[]) {
    const parent = this.parentId
    if (parent === null) return
    this.say(`importing ${files.length} file${files.length === 1 ? '' : 's'} …`)

    const report = await api.importFolder(parent, files)
    await this.refresh()

    const parts = [
      `${report.documents} document${report.documents === 1 ? '' : 's'}`,
      `${report.folders} folder${report.folders === 1 ? '' : 's'}`,
      `${report.files} file${report.files === 1 ? '' : 's'}`,
      `${report.links} link${report.links === 1 ? '' : 's'} rewritten`,
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
    const uploaded = await api.upload(files)
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
