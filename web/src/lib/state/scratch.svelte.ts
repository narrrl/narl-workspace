import { api, type ScratchNote } from '../api'
import type { Workspace } from '../store.svelte'

/** The scratchpad: the drawer of notes taken in passing, and the ways out of it. */
export class Scratchpad {
  readonly ws: Workspace

  constructor(ws: Workspace) {
    this.ws = ws
  }

  /*
   * The scratchpad: a drawer that comes down from under the bar over whatever
   * is on screen, as the desktop's scratchpad does, with the notes taken in
   * passing.
   */
  open = $state(false)
  notes = $state<ScratchNote[]>([])
  cursor = $state(0)

  get editing() {
    return this.ws.drafts.writing?.kind === 'note'
  }
  /** The note being written has not been saved yet, so it is in no row. */
  get writingNew() {
    return this.editing && this.ws.drafts.writing?.id === null
  }

  get note(): ScratchNote | undefined {
    return this.notes[this.cursor]
  }

  /** `Q`: the drawer down, read again, or up and away. */
  async toggle() {
    if (this.open) return this.close()
    this.notes = await api.scratch()
    this.cursor = Math.min(this.cursor, Math.max(this.notes.length - 1, 0))
    this.open = true
  }

  async close() {
    if (this.editing && !(await this.ws.drafts.leaveWriting())) return
    this.open = false
  }

  /**
   * Ctrl-Q, from anywhere: the drawer down with the note written last being
   * written on, at its end — or, when it is down, up again.
   */
  async jot() {
    if (this.open) return this.close()
    await this.toggle()
    this.cursor = 0
    this.editNote()
    if (this.ws.drafts.writing) this.ws.drafts.writing.append = true
  }

  /** `o`: a new note at the top, written straight away. */
  newNote() {
    this.#writeNote(null, '')
  }

  /** `e`, `i`, Enter: write the note under the cursor. */
  editNote() {
    const note = this.note
    if (!note) return this.newNote()
    this.#writeNote(note.id, note.body)
  }

  /*
   * A new note is only made once it says something. The note touched last
   * goes to the top of the list, and the cursor with it.
   */
  #writeNote(id: number | null, body: string) {
    this.ws.drafts.write('note', id, body, async (text, writing) => {
      if (writing.id === null) {
        if (!text.trim()) return
        const note = await api.createScratch(text)
        writing.id = note.id
        this.notes = [note, ...this.notes]
      } else {
        const saved = await api.updateScratch(writing.id, text)
        this.notes = [saved, ...this.notes.filter((n) => n.id !== saved.id)]
      }
      this.cursor = 0
    })
  }

  async dropNote(say = true, note = this.note) {
    if (!note) return
    await api.deleteScratch(note.id)
    this.#forgetNote(note.id)
    if (say) this.ws.say('scratch note dropped')
  }

  /** A note gone from the drawer and from the search results, the cursors kept in range. */
  #forgetNote(id: number) {
    this.notes = this.notes.filter((n) => n.id !== id)
    this.cursor = Math.min(this.cursor, Math.max(this.notes.length - 1, 0))
    if (!this.ws.tree.foundNotes.some((n) => n.id === id)) return
    this.ws.tree.foundNotes = this.ws.tree.foundNotes.filter((n) => n.id !== id)
    this.ws.tree.cursor = Math.min(this.ws.tree.cursor, Math.max(this.ws.tree.rows - 1, 0))
  }

  /** A note found by a search: the drawer down, with it under the cursor. */
  async showNote(id: number) {
    if (!this.open) await this.toggle()
    else this.notes = await api.scratch()
    const at = this.notes.findIndex((note) => note.id === id)
    if (at < 0) return this.ws.say('that note is gone', 'error')
    this.cursor = at
  }

  /** The note under the cursor, saved and closed if it was being written, for it to leave. */
  async #leavingNote(): Promise<ScratchNote | undefined> {
    if (this.editing && !(await this.ws.drafts.leaveWriting())) return
    const note = this.note
    if (!note) this.ws.say('no scratch note under the cursor', 'error')
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
  async cardNote() {
    const note = await this.#leavingNote()
    if (!note) return
    const { title, rest } = this.#split(note)
    if (!title) return this.ws.say('the note has nothing to call a card by', 'error')
    // Made first, dropped after: a failure in between leaves two copies, never none.
    await this.ws.cards.createCard(title, null, rest)
    await this.dropNote(false, note)
  }

  /**
   * `:append <document>`: the note under the cursor put at the end of a
   * document, named as a link names it, and taken off the scratchpad. Open
   * drafts are written first, and a tile showing the document reads it again.
   */
  async appendNote(document: string) {
    const note = await this.#leavingNote()
    if (!note) return
    await this.ws.drafts.flush()
    const space = this.ws.tree.space?.role === 'diary' ? undefined : this.ws.tree.space?.slug
    const landed = await api.capture({ text: note.body, to: 'document', document, space, note: note.id })
    this.#forgetNote(note.id)
    await this.ws.drafts.rereadNode(landed.id)
    this.ws.say(`appended to ${landed.where}`)
  }

  /** `:scratch <text>`: a note from the command line, without the drawer. */
  async add(text: string) {
    const note = await api.createScratch(text)
    this.notes = [note, ...this.notes.filter((n) => n.id !== note.id)]
    this.ws.say('scratched — Q shows the scratchpad')
  }

  /**
   * `:keep [title]`: the note under the cursor made a document in the folder
   * being browsed, and taken off the scratchpad. Without a title, the first
   * line is the title — a `# heading` or not — and the rest the body.
   */
  async keepNote(title = '') {
    const note = await this.#leavingNote()
    if (!note) return
    const parent = this.ws.tree.parentId ?? this.ws.tree.spaceId ?? this.ws.tree.spaces.find((space) => space.role !== 'diary')?.id
    if (parent === undefined || parent === null) return this.ws.say('no space to keep it in', 'error')

    const split = this.#split(note)
    const name = title.trim() || split.title
    const body = title.trim() ? note.body : split.rest
    const node = await api.createNode({ parent_id: parent, name, body })
    // Made first, dropped after: a failure in between leaves two copies, never none.
    await this.dropNote(false, note)
    this.open = false
    await this.ws.tree.refresh(node.id)
    await this.ws.tree.openNode(node.id)
    this.ws.say(`kept as ${node.path.map((crumb) => crumb.slug).join('/')}`)
  }
}
