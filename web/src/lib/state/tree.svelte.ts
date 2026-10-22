import { api, isContainer, type AgendaCard, type Crumb, type WorkspaceNode, type NodeSummary, type ScratchNote } from '../api'
import { formatBytes, sealedNote } from '../util'
import { Tile, type View } from './tiles.svelte'
import type { Workspace, Pane } from '../store.svelte'

/**
 * Where a space was left: the folder, the tiles, the row, the pane. A tile
 * is the id of the node it shows; the name of a view is what a spot held
 * while views were tiled among the documents, and is passed over now.
 * `openId` is the focused tile's, all a spot held before there were tiles,
 * and `open` the documents alone, all it held before there were views.
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

/**
 * The spaces and the folders in them: where the list is, what it shows,
 * what a search found, what `m` picked up — and what is done to the nodes
 * from there: opening, making, renaming, moving, deleting, uploading.
 */
export class Tree {
  readonly ws: Workspace

  constructor(ws: Workspace) {
    this.ws = ws
  }

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
   * The nodes picked up by `m`, waiting for the `p` that drops them into
   * whatever folder the list is showing then. A move is two keystrokes apart
   * in time because the destination is somewhere else in the tree — there is
   * no way to name it and be somewhere else at once — so the marks stay put
   * while the list walks, and can be gathered from several folders.
   */
  marked = $state.raw<NodeSummary[]>([])

  /** A search across every space rather than the one being browsed — `/` then Tab, or `:search!`. */
  globalSearch = $state(false)

  /** The last thing deleted here, which `u` puts back. */
  lastDeleted = $state<number | null>(null)

  /** Documents whose files `za` has folded open under them in the list. */
  unfolded = $state<number[]>([])

  /*
   * Spaces behave like the desktop's workspaces: each keeps the folder and the
   * document it was left on, and coming back finds them where they were.
   */
  #spots: Record<number, Spot> = JSON.parse(localStorage.getItem('diary:spots') ?? '{}')

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

  get space(): NodeSummary | undefined {
    return this.spaces.find((s) => s.id === this.spaceId)
  }

  /** `work/einarbeiten` — what the sidebar and the status line show. */
  get here(): string {
    return this.path.map((crumb) => crumb.slug).join('/')
  }

  /*
   * Navigation. The list pane always shows one folder's children, so browsing
   * is: pick a space, descend into folders, come back up. A search steps
   * outside that and shows matches from anywhere in the current space.
   */

  /** Keep the address bar on the node being looked at, so it can be linked to — the history entry, back's guard among them, kept as it is. */
  syncUrl(path: Crumb[]) {
    const slugs = path.map((crumb) => crumb.slug).join('/')
    if (slugs) history.replaceState(history.state, '', `/n/${slugs}`)
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
        this.ws.say(`no such path: ${startPath}`, 'error')
      }
    }
    const remembered = Number(localStorage.getItem('diary:space'))
    const start =
      this.spaces.find((s) => s.id === remembered) ??
      this.spaces.find((s) => s.role === 'diary') ??
      this.spaces[0]
    if (start) await this.enterSpace(start.id)
  }

  /** Where this space is left, its documents on screen. */
  #rememberSpot() {
    if (this.spaceId === null || this.parentId === null) return
    const kept = this.ws.tiles.all.filter((tile) => tile.open)
    this.#spots[this.spaceId] = {
      parentId: this.parentId,
      openId: this.ws.open?.id ?? null,
      cursor: this.cursor,
      pane: this.ws.pane,
      tiles: kept.map((tile) => tile.open!.id),
      focused: Math.max(kept.indexOf(this.ws.tile), 0),
    }
    localStorage.setItem('diary:spots', JSON.stringify(this.#spots))
  }

  async enterSpace(id: number) {
    /*
     * The views look at every board and the whole diary, so the plan screen
     * stays as it is — a digit on the board shows that space's board — and
     * it is the documents behind it that are this space's.
     */
    const planning = this.ws.tiles.planning
    if (planning) this.ws.tiles.swap()
    try {
      await this.#enterSpace(id)
    } finally {
      if (planning) this.ws.tiles.swap()
    }
    await this.ws.tiles.loadViews()
  }

  async #enterSpace(id: number) {
    const leaving = this.spaceId !== null && this.spaceId !== id
    if (leaving) {
      await this.ws.drafts.flush()
      this.#rememberSpot()
      const from = this.spaces.findIndex((s) => s.id === this.spaceId)
      const to = this.spaces.findIndex((s) => s.id === id)
      this.ws.switchDirection = to >= from ? 1 : -1
      // Each space has tiles of its own; these come back with this one.
      for (const tile of this.ws.tiles.all) tile.stopAutosave()
      this.ws.tiles.replace([])
      this.ws.pane = 'list'
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
        if (typeof entry === 'string') continue
        const empty = this.ws.tiles.all.findIndex((tile) => tile.empty)
        if (empty >= 0) this.ws.tiles.focused = empty
        else {
          this.ws.tiles.all.push(new Tile())
          this.ws.tiles.focused = this.ws.tiles.all.length - 1
        }
        try {
          await this.openNode(entry)
          restored.push(this.ws.tile)
        } catch {
          /* deleted or moved away since: one tile fewer */
        }
      }
      this.ws.tiles.prune()
      const back = restored[Math.min(spot.focused ?? 0, restored.length - 1)]
      if (back && this.ws.tiles.all.includes(back)) {
        this.ws.tiles.focusOn(this.ws.tiles.all.indexOf(back))
        this.ws.pane = spot.pane
      } else {
        this.cursor = Math.min(spot.cursor, Math.max(this.nodes.length - 1, 0))
      }
    }
    this.ws.tiles.trail = []
  }

  /** Show the children of `id`, which must be a space or a folder. */
  async goTo(id: number, keep?: number) {
    await this.ws.drafts.flush()
    const folder = await api.getNode(id)
    // Walking into another space by a link leaves this one where it was.
    if (this.spaceId !== null && folder.path[0]?.id !== this.spaceId) this.#rememberSpot()
    this.parentId = id
    this.parentSealed = folder.sealed
    this.query = ''
    this.globalSearch = false
    this.path = folder.path
    this.spaceId = folder.path[0]?.id ?? id
    this.nodes = await this.#withFiles(await api.listNodes({ parent: id }))
    const index = this.nodes.findIndex((n) => n.id === keep)
    this.cursor = index >= 0 ? index : 0
    this.syncUrl(folder.path)
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
      const space = this.searchSpace
      // A note is in no space, so every search looks through them all.
      ;[this.nodes, this.foundCards, this.foundNotes] = await Promise.all([
        api.listNodes({ q: this.query, space }),
        api.searchCards(this.query, space),
        api.scratch(this.query),
      ])
    } else {
      this.nodes = await this.#withFiles(await api.listNodes({ parent: this.parentId ?? undefined }))
      this.foundCards = []
      this.foundNotes = []
    }
    const index = this.nodes.findIndex((n) => n.id === keepId)
    this.cursor = index >= 0 ? index : Math.min(this.cursor, Math.max(this.rows - 1, 0))
  }

  /** The list, with the files of each document `za` folded open under it. */
  async #withFiles(list: NodeSummary[]): Promise<NodeSummary[]> {
    const open = list.filter((n) => n.kind === 'document' && n.child_count > 0 && this.unfolded.includes(n.id))
    if (open.length === 0) return list
    const files = new Map(await Promise.all(open.map(async (doc) => [doc.id, await api.listNodes({ parent: doc.id })] as const)))
    return list.flatMap((n) => [n, ...(files.get(n.id) ?? [])])
  }

  /** `za`: a document's own files, folded open under it in the list, or away again. */
  async toggleFiles() {
    const node = this.selected
    if (!node || this.query) return
    const doc = node.kind === 'document' ? node : this.nodes.find((n) => n.id === node.parent_id && n.kind === 'document')
    if (!doc) return this.ws.say('za folds open the files of a document', 'error')
    if (this.unfolded.includes(doc.id)) {
      this.unfolded = this.unfolded.filter((id) => id !== doc.id)
    } else {
      if (doc.child_count === 0) return this.ws.say('no files — :upload while reading it gives it some', 'error')
      this.unfolded = [...this.unfolded, doc.id]
    }
    await this.refresh(doc.id)
  }

  /** A row the list shows under its document, not in the folder itself. */
  isNested(node: NodeSummary): boolean {
    return !this.query && node.parent_id !== this.parentId
  }

  /** The space a search keeps to; none for one across every space. */
  get searchSpace(): number | undefined {
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
    if (!this.query) return this.ws.say('no search — / starts one', 'error')
    const count = this.rows
    if (count === 0) return this.ws.say(`no results for "${this.query}"`, 'error')
    const next = this.cursor + delta
    if (next >= count) this.ws.say('search hit BOTTOM, continuing at TOP')
    else if (next < 0) this.ws.say('search hit TOP, continuing at BOTTOM')
    this.cursor = (next + count) % count
    const node = this.selected
    if (node && !isContainer(node)) await this.openNode(node.id)
  }

  move(delta: number) {
    if (this.rows === 0) return
    this.cursor = Math.min(Math.max(this.cursor + delta, 0), this.rows - 1)
  }

  async openNode(id: number, edit = false) {
    /*
     * The plan screen holds the views alone: what is opened from it — a
     * card's notes, a day of the calendar, a search — opens on the documents'
     * screen, beside what is there, and `S` goes back.
     */
    const fromPlan = this.ws.tiles.planning
    if (fromPlan) this.ws.tiles.leavePlan()
    else if (this.ws.open?.id !== id) await this.ws.drafts.flushTile()

    const node = await api.getNode(id)
    if (isContainer(node)) {
      await this.goTo(node.id)
      return
    }
    // A document already open in another tile is gone to, not opened twice:
    // two drafts of one document would only overwrite each other.
    const shown = this.ws.tiles.all.findIndex((tile) => tile !== this.ws.tile && tile.open?.id === id)
    if (shown >= 0) {
      this.ws.tiles.focusOn(shown)
      if (edit && node.kind === 'document') this.ws.editing = true
      return
    }
    if (fromPlan && !this.ws.tile.empty) this.ws.tiles.add()

    this.ws.open = node
    this.ws.draft = { name: node.name, body: node.body, created_at: node.created_at }
    this.ws.dirty = false
    // A file has a name and nothing to write in.
    this.ws.editing = edit && node.kind === 'document'
    this.ws.pane = 'document'
    this.syncUrl(node.path)
    const index = this.nodes.findIndex((n) => n.id === id)
    if (index >= 0) this.cursor = index

    // A stash that differs from what the server has is work the last session
    // did not manage to write. Vim would call this a swap file.
    const stashed = this.ws.drafts.readStash(id)
    if (stashed && (stashed.body !== node.body || stashed.name !== node.name)) {
      this.ws.draft = stashed
      this.ws.dirty = true
      this.ws.say('recovered an unsaved draft — :w to keep it, :e! to discard', 'error')
    } else {
      this.ws.drafts.dropStash(id)
    }
  }

  /** What Enter does: descend into a folder, open a document or a file. */
  async openSelected(edit = false) {
    const found = this.foundCard
    if (found) return this.ws.cards.turnTo(found)
    const note = this.foundNote
    if (note) return this.ws.scratch.showNote(note.id)
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
    this.ws.tile.fresh = node.id
    this.ws.enterInsert = true
    this.ws.say(`new document ${this.here}/${node.slug}`)
  }

  async createFolder(name: string) {
    const parent = this.parentId
    if (parent === null) return
    const node = await api.createNode({ parent_id: parent, kind: 'folder', name })
    await this.refresh()
    const index = this.nodes.findIndex((n) => n.id === node.id)
    if (index >= 0) this.cursor = index
    this.ws.say(`new folder ${this.here}/${node.slug}`)
  }

  async createSpace(name: string) {
    const space = await api.createSpace(name)
    await this.loadSpaces()
    await this.enterSpace(space.id)
    this.ws.say(`new space ${space.slug}`)
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
    const showing = this.ws.tiles.showing(id)
    for (const tile of showing) tile.stopAutosave()
    this.ws.drafts.dropStash(id)
    const node = showing[0]?.open ?? this.nodes.find((n) => n.id === id)
    const { removed, purged } = await api.deleteNode(id)
    for (const tile of showing) tile.clear()
    this.ws.tiles.prune()
    this.marked = this.marked.filter((n) => n.id !== id)
    await this.refresh()
    const name = node ? `"${node.name.trim() || node.slug}"` : `#${id}`
    if (purged) return this.ws.say(`deleted ${name} for good`)
    this.lastDeleted = id
    const what = removed > 1 ? `${name} and ${removed - 1} inside it` : name
    this.ws.say(`${what} went to the trash — u brings it back`)
  }

  /** `u`: the last delete, undone. */
  async undoDelete() {
    const id = this.lastDeleted
    if (id === null) return this.ws.say('nothing deleted here to undo — :trash has the rest', 'error')
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
      if (this.ws.sheets.shown !== 'trash') await this.enterSpace(node.id)
      this.ws.say(`restored the space ${name}`)
    } else {
      // A document's file comes back to the document, shown folded open in
      // the list of the folder the document is in.
      const owner = node.path.at(-2)?.kind === 'document' ? node.path.at(-2) : undefined
      if (owner && !this.unfolded.includes(owner.id)) this.unfolded = [...this.unfolded, owner.id]
      const parent = node.path.at(owner ? -3 : -2)
      if (parent && node.path[0]?.id === this.spaceId) {
        if (parent.id === this.parentId && !this.query) await this.refresh(node.id)
        else if (this.ws.sheets.shown !== 'trash') await this.goTo(parent.id, node.id)
      }
      const where = node.path.slice(0, -1).map((crumb) => crumb.slug).join('/')
      this.ws.say(elsewhere ? `restored ${name} to ${where} — the folder it was in is gone` : `restored ${name} to ${where}`)
    }
    if (this.ws.sheets.shown === 'trash') await this.ws.sheets.loadTrash()
  }

  /**
   * Follow a `[[title]]`: to the document it names, or — when there is none
   * — to a new one called that, beside the document linking to it, after
   * asking. Which document a title names the server says when the text is
   * written, so it is written first.
   */
  async followTitle(title: string) {
    await this.ws.drafts.flushTile()
    const open = this.ws.open
    const path = open?.wikilinks[title]
    if (path) return this.openPath(path)
    const crumbs = open?.path.slice(0, -1) ?? []
    const folder = crumbs.at(-1)
    const where = folder ? crumbs.map((crumb) => crumb.slug).join('/') : this.here
    if (!(await this.ws.prompts.ask(`no document called "${title}" yet — write it in ${where}?`))) return
    if (folder && folder.id !== this.parentId) await this.goTo(folder.id)
    await this.createDocument(title)
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
    const open = this.ws.tiles.showing(id)[0]?.open
    const saved = await api.updateNode(id, { name: name.trim(), base_updated_at: open?.updated_at })
    for (const tile of this.ws.tiles.showing(id)) {
      tile.open = saved
      tile.draft.name = saved.name
    }
    if (this.path.some((crumb) => crumb.id === id)) this.path = saved.path
    if (saved.kind === 'space') this.spaces = await api.listSpaces()
    await this.ws.drafts.reread()
    await this.refresh()
    this.ws.say(`renamed to ${saved.slug}`)
  }

  /**
   * Delete a space and everything in it. The app has to land somewhere
   * afterwards, so it falls into the next space — or, if that was the last
   * one, into an empty workspace that `:space new <name>` fills again.
   */
  async deleteSpace(id: number) {
    const { removed } = await api.deleteNode(id)
    this.lastDeleted = id
    for (const tile of this.ws.tiles.every) if (tile.kind === 'node') tile.clear()
    this.ws.tiles.prune()
    this.ws.cards.rawBoard = null
    this.ws.cards.boardSpaceId = null
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
    this.ws.say(`the space and ${removed - 1} node(s) in it went to the trash — u brings it back`)
  }

  /** Reparent the node under the cursor, for `:mv`. */
  async moveNode(id: number, parentId: number) {
    await api.moveNode(id, parentId)
    await this.ws.drafts.reread()
    await this.refresh()
    this.ws.say(`moved #${id}`)
  }

  isMarked(id: number): boolean {
    return this.marked.some((n) => n.id === id)
  }

  /**
   * Move `nodes` into `parent`, all or none, and say how it went. An open one
   * is saved and closed first: it is about to be somewhere else.
   */
  async moveNodes(nodes: NodeSummary[], parent: number, where = 'here') {
    if (nodes.length === 0) return this.ws.say('nothing marked — press m on a row first', 'error')
    const leaving = this.ws.tiles.every.filter((tile) => nodes.some((n) => n.id === tile.open?.id))
    for (const tile of leaving) {
      await this.ws.drafts.flushTile(tile)
      tile.clear()
    }
    this.ws.tiles.prune()
    const { moved, unchanged } = await api.moveNodes(
      nodes.map((n) => n.id),
      parent,
    )
    const gone = new Set(nodes.map((n) => n.id))
    this.marked = this.marked.filter((n) => !gone.has(n.id))
    if (moved > 0) await this.ws.drafts.reread()
    await this.refresh(nodes.length === 1 ? nodes[0].id : undefined)
    const what = nodes.length === 1 ? nodes[0].name.trim() || nodes[0].slug : `${moved}`
    const already = unchanged > 0 ? ` — ${unchanged} already ${unchanged === 1 ? 'was' : 'were'}` : ''
    this.ws.say(moved === 0 ? `already ${where}` : `moved ${what} ${where}${already}`)
  }

  /** `1`..`9`, `0`: the space at that place on the bar. */
  async switchSpace(index: number) {
    const space = this.spaces[index]
    if (!space) return this.ws.say(`no space ${index + 1}`, 'error')
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
    if (!space) return this.ws.say(`no space ${index + 1}`, 'error')
    if (nodes.length === 0) return this.ws.say('nothing selected', 'error')
    await this.moveNodes(nodes, space.id, `→ ${index + 1} ${space.slug}`)
    this.spaces = await api.listSpaces()
  }

  /**
   * `m`: pick the node under the cursor up, or put it back down unmoved, and
   * step to the next row — `mmm` picks up three in a row.
   */
  mark(node = this.selected, step = true) {
    if (!node) return this.ws.say('nothing selected', 'error')
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
    this.ws.say('unmarked')
  }

  #sayMarked() {
    const count = this.marked.length
    if (count === 0) return this.ws.say('unmarked')
    const first = this.marked[0]
    const what = count === 1 ? first.name.trim() || first.slug : `${count} nodes`
    this.ws.say(`marked ${what} — p drops ${count === 1 ? 'it' : 'them'} in the folder you are in`)
  }

  /**
   * `p`: drop the marked nodes into the folder the list is showing — or,
   * when they are files and the cursor is in a document's files folded open,
   * into that document.
   */
  async drop() {
    const parent = this.parentId
    if (this.marked.length === 0) return this.ws.say('nothing marked — press m on a row first', 'error')
    if (parent === null) return this.ws.say('no folder open', 'error')
    const at = this.selected
    const doc = at && this.isNested(at) ? at.parent_id : at?.id
    if (doc && this.unfolded.includes(doc) && this.marked.every((n) => n.kind === 'file')) {
      const name = this.nodes.find((n) => n.id === doc)?.name.trim()
      return this.moveNodes(this.marked, doc, `to ${name || 'the document'}`)
    }
    await this.moveNodes(this.marked, parent)
  }

  async toggleShare(id: number) {
    const node = this.ws.open?.id === id ? this.ws.open : await api.getNode(id)
    if (node.kind !== 'document') return this.ws.say('only a document can be shared', 'error')
    if (node.shared) {
      await api.unshare(id)
      this.ws.say(`#${id} is private again`)
    } else {
      const { url, sealed } = await this.mintShareLink(id)
      this.ws.say(`shared → ${url} (copied)${sealedNote(sealed)}`, sealed > 0 ? 'error' : undefined)
    }
    for (const tile of this.ws.tiles.showing(id)) tile.open = await api.getNode(id)
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
    await this.ws.copy(url)
    return { url, sealed }
  }

  /** `:share new`: a new link for a shared document, copied; the old one stops working. */
  async renewShareLink(id: number) {
    const { url, sealed } = await this.mintShareLink(id)
    this.ws.say(`new link → ${url} (copied) — the previous one is dead${sealedNote(sealed)}`, sealed > 0 ? 'error' : undefined)
    for (const tile of this.ws.tiles.showing(id)) tile.open = await api.getNode(id)
  }

  /**
   * Import a picked folder (or a zip of one) into the folder being browsed.
   * The report is worth a line of its own: an import that quietly skipped half
   * its files would only be noticed much later.
   */
  async importFolder(files: File[], root = false) {
    const parent = this.parentId
    if (parent === null && !root) return
    if (!root && this.parentSealed && !(await this.ws.vault.ensureUnlocked())) return
    this.ws.say(`importing ${files.length} file${files.length === 1 ? '' : 's'} …`)

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
    this.ws.say(`imported ${parts.join(', ')}`)
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
      if (!(await this.ws.vault.ensureUnlocked())) return
      encrypt = true
    }
    const total = files.reduce((sum, file) => sum + file.size, 0)
    const what = files.length === 1 ? files[0].name : `${files.length} files`
    this.ws.say(`uploading ${what} (${formatBytes(total)})${encrypt ? ' into the vault' : ''} …`)

    const created = await api.putFiles(parent, files, encrypt, (fraction) =>
      this.ws.say(`uploading ${what} — ${Math.floor(fraction * 100)}%`),
    )
    this.query = ''
    this.globalSearch = false
    await this.refresh(created[0]?.id)
    this.ws.say(created.length === 1 ? `uploaded ${this.here}/${created[0].slug}` : `uploaded ${created.length} files`)
  }

  /**
   * Give the document in `tile` files of its own — what a picture pasted
   * while writing is too. Answers the file nodes, for the editor to embed.
   */
  async attach(files: File[], tile = this.ws.tile): Promise<NodeSummary[]> {
    const doc = tile.open
    if (doc?.kind !== 'document') {
      this.ws.say('files go to a document — open one first', 'error')
      return []
    }
    // In a sealed folder they are sealed on the way, which needs the vault.
    if (doc.sealed && !(await this.ws.vault.ensureUnlocked())) return []
    const what = files.length === 1 ? files[0].name : `${files.length} files`
    const total = files.reduce((sum, file) => sum + file.size, 0)
    const name = doc.name.trim() || doc.slug
    this.ws.say(`uploading ${what} (${formatBytes(total)}) to ${name} …`)
    const created = await api.putFiles(doc.id, files, doc.sealed, (fraction) =>
      this.ws.say(`uploading ${what} — ${Math.floor(fraction * 100)}%`),
    )
    // Its list of files, without touching what is being written in it.
    if (tile.open?.id === doc.id) {
      tile.open = { ...tile.open, files: [...tile.open.files, ...created], child_count: tile.open.child_count + created.length }
    }
    await this.refresh()
    this.ws.say(`${created.length === 1 ? created[0].name : `${created.length} files`} now belong${created.length === 1 ? 's' : ''} to ${name}`)
    return created
  }
}
