import { api, unreachable } from '../api'
import { AUTOSAVE_MS, Writing, type WritingKind } from '../writing.svelte'
import { type Tile, type Draft } from './tiles.svelte'
import type { Workspace } from '../store.svelte'

/**
 * What is being written, and getting it to the server: the documents' drafts
 * in their tiles, and the short text written in place — a card's
 * description, a note.
 */
export class Drafts {
  readonly ws: Workspace

  constructor(ws: Workspace) {
    this.ws = ws
  }

  /*
   * The short text being written in place, if one is: the description of the
   * card turned over, or the note under the drawer's cursor. One at a time,
   * as there is one place on screen each is written in.
   */
  writing = $state<Writing | null>(null)

  /*
   * Autosave. Typing is where work gets lost, so a draft goes to localStorage
   * on every change — cheap, synchronous, survives a killed tab — and to the
   * server a couple of seconds after typing stops.
   */

  #stashKey(id: number) {
    return `diary:draft:${id}`
  }

  /** Call whenever the draft changes. Replaces a bare `dirty = true`. */
  touch(tile = this.ws.tile) {
    tile.dirty = true
    this.stash(tile)
    tile.stopAutosave()
    tile.autosave = setTimeout(() => void this.#writeBack(tile), AUTOSAVE_MS)
  }

  stash(tile = this.ws.tile) {
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

  readStash(id: number): Draft | null {
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
      if (tile.retryDelay > 0) this.ws.say('saved — the server is back')
      tile.retryDelay = 0
    } catch (error) {
      if (!unreachable(error)) return this.ws.guard(() => Promise.reject(error))
      // Nothing is lost — the draft is in localStorage — but it is not on the
      // server either, so it is tried again, less often the longer it fails.
      tile.retryDelay = Math.min(Math.max(tile.retryDelay * 2, 2000), 60_000)
      this.ws.say(`not saved: the server cannot be reached — kept here, trying again in ${tile.retryDelay / 1000}s`, 'error')
      tile.autosave = setTimeout(() => void this.#writeBack(tile), tile.retryDelay)
    }
  }

  /** Back online: what is waiting goes now rather than at the next retry. */
  watchConnection() {
    window.addEventListener('online', () => {
      for (const tile of this.ws.tiles.every) {
        if (!tile.dirty) continue
        tile.stopAutosave()
        void this.#writeBack(tile)
      }
      if (this.writing?.dirty) void this.ws.guard(() => this.saveWriting())
    })
    window.addEventListener('offline', () => {
      this.ws.say('offline — what is typed is kept here, and saved once the server answers', 'error')
    })
  }

  /**
   * Write out every pending draft before doing something that would replace
   * one. Without this, clicking another document in the list discarded
   * unsaved work without a word.
   */
  async flush() {
    for (const tile of this.ws.tiles.every) await this.flushTile(tile)
  }

  /** The same for one tile only, as it is about to show something else. */
  async flushTile(tile = this.ws.tile) {
    tile.stopAutosave()
    if (await this.abandonIfBlank(tile)) return
    if (tile.open && tile.dirty) await this.ws.guard(() => this.save({ quiet: true, tile }))
  }

  /**
   * Drop the document `o` made if nothing was ever put in it. True when it
   * went, so the caller knows there is nothing open any more.
   */
  async abandonIfBlank(tile = this.ws.tile): Promise<boolean> {
    const open = tile.open
    if (!open || open.id !== tile.fresh) return false
    if (tile.draft.name.trim() || tile.draft.body.trim()) return false
    tile.clear()
    this.dropStash(open.id)
    this.ws.tiles.prune()
    try {
      await api.deleteNode(open.id)
    } catch {
      /* already gone, or the server is away; an empty row is no great loss */
    }
    await this.ws.tree.refresh()
    return true
  }

  /**
   * Write the draft. Refused when the document was saved somewhere else
   * since it was opened here — another tab, the phone, the SFTP mount —
   * unless `force`, which is `:w!`.
   */
  async save({ quiet = false, force = false, restore = false, tile = this.ws.tile } = {}) {
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
    if (!this.ws.tree.patch(saved)) await this.ws.tree.refresh()
    if (!quiet) this.ws.say(`"${saved.slug}" ${saved.body.split('\n').length}L written`)
  }

  /**
   * Read the open documents again after a rename or a move: their paths may
   * have changed with it, and their text too, where the server rewrote a
   * link to what moved. One being written is left as it is — saving it is
   * then refused, and `:e!` takes the rewritten text.
   */
  async reread() {
    for (const tile of this.ws.tiles.every) {
      if (!tile.open || tile.editing || tile.dirty) continue
      const node = await api.getNode(tile.open.id).catch(() => null)
      if (!node) continue
      tile.open = node
      tile.draft = { name: node.name, body: node.body, created_at: node.created_at }
    }
    if (this.ws.open) this.ws.tree.syncUrl(this.ws.open.path)
  }

  /*
   * Writing in place. A card's description and a note are written the way a
   * document is: saved by themselves a moment after the typing stops, `:w`
   * sooner, and `:q` or Esc leaves with what is still waiting written first.
   */

  /** Begin writing `kind`: what it says on the server, unless this browser kept newer. */
  write(kind: WritingKind, id: number | null, text: string, write: (text: string, writing: Writing) => Promise<void>) {
    const writing: Writing = new Writing(
      kind,
      id,
      text,
      (text) => write(text, writing),
      (error, retry) =>
        unreachable(error)
          ? this.ws.say(`not saved: the server cannot be reached — kept here, trying again in ${retry / 1000}s`, 'error')
          : void this.ws.guard(() => Promise.reject(error)),
    )
    this.writing = writing
    if (writing.recovered) this.ws.say(`recovered unsaved ${kind} — :w keeps it, :q! drops it`, 'error')
  }

  /** `:w`, Ctrl-S: what is written goes now rather than in a moment. */
  async saveWriting() {
    const writing = this.writing
    if (!writing) return
    await writing.save()
    if (writing.kind === 'description') this.ws.say('description written')
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
    else if (!(await this.ws.guard(() => writing.save()))) return false
    if (this.writing === writing) this.writing = null
    const note = this.ws.scratch.notes.find((n) => n.id === writing.id)
    if (writing.kind === 'note' && note && !note.body.trim()) await this.ws.guard(() => this.ws.scratch.dropNote(false, note))
    return true
  }

  /** Leaving a card's back, or the drawer, while something is written there keeps it, as leaving a document's tile does. */
  leaveCard() {
    if (this.ws.tile.kind === 'card' && this.ws.cards.describing) void this.leaveWriting()
  }

  /** Every tile showing `id` reads it again, after it was written somewhere else. */
  async rereadNode(id: number) {
    for (const tile of this.ws.tiles.every) {
      if (tile.open?.id !== id || tile.dirty) continue
      const node = await api.getNode(id)
      tile.open = node
      tile.draft = { name: node.name, body: node.body, created_at: node.created_at }
    }
  }
}
