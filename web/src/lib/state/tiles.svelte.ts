import { isContainer, type WorkspaceNode } from '../api'
import { formatDay } from '../util'
import type { Pane, Workspace } from '../store.svelte'

/**
 * What a tile shows: a node of the tree — a document or a file — or one of
 * the views over the boards and the diary, which are tiled beside the
 * documents like any other window.
 */
export type TileKind = 'node' | 'board' | 'agenda' | 'calendar' | 'card'
export type View = Exclude<TileKind, 'node'>

export type Draft = { name: string; body: string; created_at: number }

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
 * The tiles on screen and which has the focus: opened, closed, walked. There
 * are two screens of them, and one is shown at a time: the documents, beside
 * the list, and the plan — the board, the agenda, the calendar, a card's
 * back — which `S` goes to and comes back from.
 */
export class Tiles {
  readonly ws: Workspace

  constructor(ws: Workspace) {
    this.ws = ws
  }

  /*
   * The documents on screen. There is always at least one tile, empty when
   * nothing is open; the workspace's `open`, `draft`, `editing` … are the
   * focused tile's, so most of the app reads one document as it always did.
   */
  all = $state<Tile[]>([new Tile()])
  focused = $state(0)

  get current(): Tile {
    return this.all[this.focused] ?? this.all[0]
  }

  /** The plan screen is on show, and the documents are put away behind it. */
  planning = $state(false)
  /** The screen not on show, as it was left. */
  #away = $state<{ all: Tile[]; focused: number; trail: Tile[]; pane: Pane }>({ all: [], focused: 0, trail: [], pane: 'list' })

  /** The tiles of both screens: a draft behind the plan is still written. */
  get every(): Tile[] {
    return [...this.all, ...this.#away.all]
  }

  /** Every tile showing `id`, on either screen. */
  showing(id: number): Tile[] {
    return this.every.filter((tile) => tile.open?.id === id)
  }

  /*
   * The tiles focused before the one focused now, the last the latest:
   * closing a tile goes back to the one that had the focus before it — to
   * the board a card's notes were opened from, to the calendar a day was.
   */
  trail: Tile[] = []

  /** The tile showing `kind`, if one is. */
  view(kind: View): Tile | undefined {
    return this.all.find((tile) => tile.kind === kind)
  }

  /**
   * Focus a tile, and follow it in the list and the address bar, as opening
   * its document did.
   */
  focusOn(index: number) {
    const tile = this.all[index]
    if (!tile) return
    if (tile !== this.current) {
      this.ws.drafts.leaveCard()
      this.trail = [...this.trail.filter((t) => t !== tile && t !== this.current), this.current].slice(-20)
    }
    this.focused = index
    if (tile.empty) return
    this.ws.pane = 'document'
    if (!tile.open) return
    const row = this.ws.tree.nodes.findIndex((n) => n.id === tile.open?.id)
    if (row >= 0) this.ws.tree.cursor = row
    this.ws.tree.syncUrl(tile.open.path)
  }

  /** `tiles` on screen in place of the ones there, `focus` focused, with no trail behind. */
  replace(tiles: Tile[], focus?: Tile) {
    this.all = tiles.length > 0 ? tiles : [new Tile()]
    this.focused = focus ? this.all.indexOf(focus) : 0
    this.trail = []
  }

  /** A tile of its own for what opens next, beside what is on screen. */
  add(tile = new Tile()) {
    this.all.push(tile)
    this.focusOn(this.all.length - 1)
  }

  /**
   * Tiles with nothing in them go, as long as one is left: a closed document
   * closes its tile, as `:q` closes a window in vim. When the focused one
   * went, the focus goes back along the trail.
   */
  prune() {
    const current = this.current
    if (this.all.length > 1) {
      const kept = this.all.filter((tile) => !tile.empty)
      this.all = kept.length > 0 ? kept : [current]
      const back = this.all.includes(current) ? current : [...this.trail].reverse().find((tile) => this.all.includes(tile))
      this.focused = back ? this.all.indexOf(back) : Math.min(this.focused, this.all.length - 1)
    }
    this.trail = this.trail.filter((tile) => this.all.includes(tile) && tile !== this.current)
    if (this.current.empty) this.ws.pane = 'list'
  }

  /**
   * `:close`: the focused tile goes, or with only one, what it shows. On the
   * plan only a card's back goes; closing the board, the agenda or the
   * calendar goes back to the documents.
   */
  close() {
    if (this.planning && this.current.kind !== 'card') return this.leavePlan()
    this.ws.drafts.leaveCard()
    this.current.clear()
    this.prune()
    if (!this.current.empty) this.focusOn(this.focused)
    // Nothing open: the address is the folder's again.
    else this.ws.tree.syncUrl(this.ws.tree.path)
  }

  /**
   * `L` / Shift-Enter: the row under the cursor in a tile of its own, next to
   * what is open, rather than in its place.
   */
  async openBeside() {
    const selected = this.ws.tree.selected
    if (!selected) return
    if (isContainer(selected)) return this.ws.say(`${selected.slug} is a ${selected.kind} — l goes into it`, 'error')
    const shown = this.all.findIndex((tile) => tile.open?.id === selected.id)
    if (shown >= 0) return this.focusOn(shown)
    if (!this.current.empty) this.add()
    try {
      await this.ws.tree.openSelected()
    } finally {
      this.prune()
    }
  }

  /** `:only`: every other tile goes, after their drafts are written. */
  async only() {
    const others = this.all.filter((tile) => tile !== this.current)
    for (const tile of others) await this.ws.drafts.flushTile(tile)
    const unsaved = others.filter((tile) => tile.dirty)
    if (unsaved.length > 0) {
      const names = unsaved.map((tile) => tile.open?.slug).join(', ')
      if (!(await this.ws.prompts.ask(`${names} could not be written — close anyway? the drafts stay in this browser`))) return
    }
    for (const tile of others) tile.stopAutosave()
    this.all = [this.current]
    this.focused = 0
    this.trail = []
  }

  /*
   * The views — the board, the agenda, the calendar, a card's back — are
   * the tiles of the plan screen. Each is read again whenever it is gone to:
   * it is looked at in passing, and what it says should be what is true now.
   */

  /** The views the plan screen holds, in the order they are tiled. */
  static readonly PLAN: View[] = ['board', 'agenda', 'calendar']

  /** The other screen on show, and this one put away as it is. */
  swap() {
    const away = this.#away
    this.#away = { all: this.all, focused: this.focused, trail: this.trail, pane: this.ws.pane }
    this.all = away.all.length > 0 ? away.all : [new Tile()]
    this.focused = Math.min(away.focused, this.all.length - 1)
    this.trail = away.trail
    this.ws.pane = away.pane
    this.planning = !this.planning
  }

  /** To the plan screen, with every view on it, from wherever the documents were. */
  enterPlan() {
    if (this.planning) return
    this.ws.drafts.leaveCard()
    this.swap()
    const cards = this.all.filter((tile) => tile.kind === 'card')
    this.all = [...Tiles.PLAN.map((kind) => this.view(kind) ?? new Tile(kind)), ...cards]
    this.focused = Math.min(this.focused, this.all.length - 1)
    this.ws.pane = 'document'
  }

  /** Back to the documents, as they were left. */
  leavePlan() {
    if (!this.planning) return
    this.ws.drafts.leaveCard()
    this.swap()
    // What was closed behind the plan — deleted, moved away — is gone from here too.
    this.prune()
    if (!this.current.empty) this.focusOn(this.focused)
  }

  /** `b`, `t`, `C`, Enter on a card: to the plan screen, and that view on it focused. */
  async show(kind: View) {
    // Coming to the plan, every view on it is read again, not only this one.
    const kinds = this.planning ? [kind] : Tiles.PLAN
    await Promise.all(kinds.map((each) => this.loadView(each)))
    this.enterPlan()
    if (!this.view(kind)) this.all.push(new Tile(kind))
    this.focusOn(this.all.indexOf(this.view(kind)!))
    if (kind === 'board' && this.ws.cards.boardSpaceId === null) this.ws.say('no space has a board — :set board turns one on', 'error')
  }

  /** `S`: the plan screen with the board focused, or back to the documents. */
  async togglePlan() {
    if (this.planning) return this.leavePlan()
    await this.show('board')
  }

  /** Read again what a view shows. */
  async loadView(kind: View) {
    if (kind === 'board' || kind === 'card') await this.ws.cards.loadBoard()
    else if (kind === 'agenda') await this.ws.cards.loadAgenda()
    else {
      if (!this.ws.diary.calendarDay) this.ws.diary.calendarDay = formatDay(Math.floor(Date.now() / 1000))
      await this.ws.diary.loadCalendar()
    }
  }

  /** Every view on the plan read again, after a space switch. */
  async loadViews() {
    if (this.view('board') || this.view('card')) await this.ws.cards.loadBoard()
    if (this.view('calendar')) await this.loadView('calendar')
  }
}
