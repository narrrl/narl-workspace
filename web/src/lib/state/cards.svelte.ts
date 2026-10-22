import { api, ApiError, type Agenda, type AgendaCard, type Board, type BoardList, type Card, type Moved, type NodeSummary, type Task, type WorkspaceNode } from '../api'
import { formatDay, formatWeekday } from '../util'
import type { Workspace } from '../store.svelte'

/** `done: x`, and when a repeating card comes back. */
function doneSaid(moved: Moved): string {
  const again = moved.again?.due_at
  return again == null ? `done: ${moved.title}` : `done: ${moved.title} — again ${formatWeekday(again)} ${formatDay(again)}`
}

/**
 * The boards, the agenda and the back of a card: one board on show at a
 * time, the agenda over every board kept loaded, and the card verbs, which
 * act on the card in focus wherever it is looked at.
 */
export class Cards {
  readonly ws: Workspace

  constructor(ws: Workspace) {
    this.ws = ws
  }

  /** How long a finished card stays in sight on the board before `z` is needed. */
  static readonly RECENT_DONE_DAYS = 7

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
    const since = Math.floor(Date.now() / 1000) - Cards.RECENT_DONE_DAYS * 86400
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

  /** The step of the open card's checklist under the cursor. */
  taskCursor = $state(0)

  get describing() {
    return this.ws.drafts.writing?.kind === 'description'
  }

  /** `z` on the board: show done cards older than a week too. */
  showOldDone = $state(false)

  /**
   * The space whose board the board's tile shows: the one being browsed
   * when it has a board, or else the last one shown, so that coming from the
   * diary does not leave the board's tile empty.
   */
  boardSpaceId = $state<number | null>(null)

  /*
   * What is due or in hand on every board, kept loaded all the time: the bar
   * counts from it, so what is late is in sight without going to look.
   */
  agenda = $state<Agenda>({ cards: [], diary_today: null, on_this_day: [], today: null })
  agendaCursor = $state(0)

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
    const about = this.ws.open?.cards ?? []
    if (this.ws.sheets.shown === 'info' && this.ws.sheets.infoSection === 'cards') return about[this.ws.sheets.cardsCursor]
    switch (this.ws.focus) {
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
    const open = this.ws.focus === 'node' ? (this.ws.open?.cards ?? []).filter((c) => c.done_at === null).length : 0
    if (open > 1) this.ws.say(`${open} open cards about this — gc picks one`, 'error')
    else if (this.ws.focus === 'node' && this.ws.open?.kind === 'document') this.ws.say('no open card about this document — c makes one', 'error')
    else this.ws.say('no card in focus — :board', 'error')
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
    await this.ws.tiles.show('agenda')
  }

  /** Enter in the agenda: to the card, on its own board, beside the agenda. */
  async goToCard(card: AgendaCard) {
    // From the agenda's tile, which comes along into the card's space.
    if (this.ws.tree.spaceId !== card.space_id) await this.ws.tree.enterSpace(card.space_id)
    await this.reloadBoard(card.space_id, card.id)
    await this.ws.tiles.show('board')
  }

  /**
   * Enter on a card a search found, or one picked in a document's info
   * sheet: to its board, turned over beside it.
   */
  async turnTo(card: AgendaCard) {
    await this.goToCard(card)
    await this.turnCard()
  }

  /*
   * Boards. One per space, which is what makes "what am I working on" a
   * question with an answer: work and uni are asked separately, and the diary
   * is not asked at all. The board is reloaded whole after every change — it
   * is a few dozen rows, and a board that disagrees with the server about
   * where a card is would be worse than a round trip.
   */

  /**
   * The board of the space being browsed, or else the one shown last, or
   * else the first space's that has one; null when no space has a board.
   */
  async loadBoard() {
    const space = this.ws.tree.space
    const shown = this.boardSpace?.has_board ? this.boardSpace : this.ws.tree.spaces.find((s) => s.has_board)
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
    return this.ws.tree.spaces.find((space) => space.id === this.boardSpaceId)
  }

  async reloadBoard(spaceId = this.boardSpaceId ?? this.ws.tree.spaceId, keep = this.card?.id) {
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
    if (!target) return this.ws.say(`"${card.title}" is already in the ${index < 0 ? 'first' : 'last'} list`, 'error')
    const moved = await api.moveCard(card.id, target.id)
    await this.#cardChanged(card)
    this.ws.say(index === lists.length - 1 ? doneSaid(moved) : `"${card.title}" → ${target.name}`)
  }

  /**
   * After a card changed, wherever it was changed from: the board, the
   * agenda and the open documents read it again, and the cursor that was on
   * it stays on it.
   */
  async #cardChanged(card: Card) {
    const shown = this.rawBoard?.lists.some((list) => list.cards.some((c) => c.id === card.id)) ?? false
    const here = this.ws.focus === 'board' || this.ws.focus === 'card'
    if (shown) await this.reloadBoard(this.boardSpaceId, here ? card.id : this.card?.id)
    if (!shown || this.ws.focus === 'agenda') {
      await this.loadAgenda()
      // It may have changed group; the cursor goes with it.
      const at = this.agendaCards.findIndex((c) => c.id === card.id)
      if (at >= 0 && this.ws.focus === 'agenda') this.agendaCursor = at
    }
    await this.#reloadCardsAbout()
    if (this.ws.tree.foundCards.some((c) => c.id === card.id)) this.ws.tree.foundCards = await api.searchCards(this.ws.tree.query, this.ws.tree.searchSpace)
    const at = this.ws.open?.cards.findIndex((c) => c.id === card.id) ?? -1
    if (at >= 0) this.ws.sheets.cardsCursor = at
  }

  /** The cards about each document open in a tile, read again. */
  async #reloadCardsAbout() {
    await Promise.all(
      this.ws.tiles.every.map(async (tile) => {
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
    if (this.ws.focus === 'board' || this.ws.focus === 'card') {
      const list = this.list
      if (!list) return this.ws.say('no list to put it on', 'error')
      const card = await api.createCard(list.id, { title, node_id: nodeId, body })
      await this.reloadBoard(this.boardSpaceId, card.id)
      await this.#reloadCardsAbout()
      return this.ws.say(nodeId === null ? `carded "${card.title}"` : `carded "${card.title}" → #${nodeId}`)
    }

    const space = this.ws.tree.space?.has_board ? this.ws.tree.space : this.ws.tree.spaces.find((s) => s.has_board)
    if (!space) return this.ws.say('no space has a board — :set board turns one on', 'error')
    const inbox = (await api.board(space.id)).lists[0]
    if (!inbox) return this.ws.say(`${space.slug} has no lists — :board, then :list <name>`, 'error')
    const card = await api.createCard(inbox.id, { title, node_id: nodeId, body })
    await this.loadAgenda()
    await this.#reloadCardsAbout()
    this.ws.say(`carded "${card.title}" → ${space.slug}/${inbox.name}`)
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
    this.ws.say(done ? doneSaid(moved) : `reopened: ${card.title}`)
  }

  async deleteCard() {
    const card = this.card
    if (!card) return this.ws.say('no card selected', 'error')
    await api.deleteCard(card.id)
    await this.reloadBoard(this.boardSpaceId, undefined)
    this.ws.say(`deleted "${card.title}"`)
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
    if (!this.card) return this.ws.say('no card selected', 'error')
    this.taskCursor = 0
    await this.ws.tiles.show('card')
  }

  /** `e`: write the description of the card turned over, from what it says now. */
  describe() {
    const card = this.card
    if (!card) return
    // Named, as the board's cursor may be on another card by the time it saves.
    this.ws.drafts.write('description', card.id, card.body, (body) => this.patchCard({ body }, card).then(() => {}))
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
      if (!card) return this.ws.say('no card selected', 'error')
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
      if (!task) return this.ws.say('no step here — o adds one', 'error')
      const next = tasks.map((t, i) => (i === this.taskCursor ? { ...t, done: !t.done } : t))
      await this.writeChecklist(next)
      this.ws.say(`${next.filter((t) => t.done).length}/${next.length} done`)
    })
  }

  renameTask(text: string) {
    return this.inTurn(async () => {
      const tasks = this.card?.checklist ?? []
      if (!tasks[this.taskCursor]) return this.ws.say('no step here — o adds one', 'error')
      await this.writeChecklist(tasks.map((t, i) => (i === this.taskCursor ? { ...t, text } : t)))
    })
  }

  deleteTask() {
    return this.inTurn(async () => {
      const tasks = this.card?.checklist ?? []
      const task = tasks[this.taskCursor]
      if (!task) return this.ws.say('no step here', 'error')
      await this.writeChecklist(tasks.filter((_, i) => i !== this.taskCursor))
      this.ws.say(`dropped "${task.text}"`)
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
      this.ws.say(on ? `${label} off` : `${label} on`)
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
    this.ws.say(`new list ${name}`)
  }

  async renameList(name: string) {
    const list = this.list
    if (!list) return this.ws.say('no list selected', 'error')
    this.rawBoard = await api.renameList(list.id, name)
    this.clampBoard()
    this.ws.say(`list renamed to ${name}`)
  }

  async deleteList() {
    const list = this.list
    if (!list) return this.ws.say('no list selected', 'error')
    this.rawBoard = await api.deleteList(list.id)
    this.clampBoard()
    this.ws.say(`deleted list ${list.name}`)
  }

  /**
   * Open the document a card points at. A card without one gets one, named
   * after it, in the folder being browsed when that is in the card's space,
   * and in the space's root when it is not — so Enter always leads to a
   * place to write the notes down. `q` on that document comes back here.
   */
  async openCard() {
    const card = this.card
    const space = this.boardSpaceId
    if (!card || space === null) return
    let notes: WorkspaceNode | null = null
    if (card.node_id !== null) {
      // Deleted notes read as gone, but the card still points at them, so
      // the way back to them is named rather than a bare "not found".
      try {
        notes = await api.getNode(card.node_id)
      } catch (error) {
        if (error instanceof ApiError && error.status === 404) {
          return this.ws.say(`the notes of "${card.title}" are gone — :trash may still have them`, 'error')
        }
        throw error
      }
    }
    if (!notes) {
      const parent = this.ws.tree.spaceId === space ? (this.ws.tree.parentId ?? space) : space
      notes = await api.createNode({ parent_id: parent, name: card.title, body: '' })
      await api.updateCard(card.id, { node_id: notes.id })
    }
    const folder = notes.parent_id ?? space
    if (this.ws.tree.parentId !== folder) await this.ws.tree.goTo(folder)
    else await this.ws.tree.refresh()
    // The notes open beside the board, and `q` takes their tile away again
    // on the way back to it.
    await this.ws.tree.openNode(notes.id, card.node_id === null)
    if (card.node_id === null) this.ws.enterInsert = true
  }

  /** Turn a space's board on or off. The diary ships with it off. */
  async setBoard(on: boolean) {
    const space = this.ws.tree.space
    if (!space) return
    await api.updateNode(space.id, { has_board: on })
    this.ws.tree.spaces = await api.listSpaces()
    if (on) await this.ws.tiles.show('board')
    else if (this.boardSpaceId === space.id) {
      this.rawBoard = null
      this.boardSpaceId = null
      if (this.ws.tiles.view('board') || this.ws.tiles.view('card')) await this.loadBoard()
    }
    this.ws.say(on ? `${space.slug} has a board` : `${space.slug} has no board any more`)
  }
}
