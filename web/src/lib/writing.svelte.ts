/*
 * A short text written in place — a card's description, a note in the
 * drawer — kept the way a document's draft is: in this browser on every
 * change, cheap and synchronous, and on the server a moment after the typing
 * stops. Nothing has to be asked of it: `:w` only writes sooner, and `:q`
 * writes what is still waiting before it leaves.
 */
import { unreachable } from './api'

/** How long the typing has to stop before a draft goes to the server. */
export const AUTOSAVE_MS = 2500

export type WritingKind = 'description' | 'note'

let sessions = 0

export class Writing {
  /** Counts the times something was opened to write, so the editor is made anew only then. */
  readonly session = ++sessions
  /** The card or the note written, or null for a note not saved yet. */
  id = $state<number | null>(null)
  text = $state('')
  /** Changed since the server last had it. */
  dirty = $state(false)
  /** The text was found in this browser, newer than what the server has. */
  readonly recovered: boolean

  #write: (text: string) => Promise<void>
  #fail: (error: unknown, retry: number) => void
  #timer: ReturnType<typeof setTimeout> | undefined
  #saving: Promise<unknown> = Promise.resolve()
  #retry = 0

  /**
   * `write` sends the text to the server, and may set `id` on the way, as
   * the first save of a new note does. `fail` hears of an autosave that did
   * not make it, and in how long one that found no server is tried again.
   */
  constructor(
    readonly kind: WritingKind,
    id: number | null,
    text: string,
    write: (text: string) => Promise<void>,
    fail: (error: unknown, retry: number) => void,
  ) {
    this.id = id
    this.#write = write
    this.#fail = fail
    const stashed = read(this.key)
    this.recovered = stashed !== null && stashed !== text
    this.text = this.recovered ? stashed! : text
    this.dirty = this.recovered
  }

  get key() {
    return `${this.kind}:draft:${this.id ?? 'new'}`
  }

  /** The editor's every change. */
  change(text: string) {
    this.text = text
    this.dirty = true
    keep(this.key, text)
    clearTimeout(this.#timer)
    this.#timer = setTimeout(() => void this.#autosave(), AUTOSAVE_MS)
  }

  /*
   * Saves run one after another, so the first save of a new note has made it
   * before the next looks for its id.
   */
  save(): Promise<void> {
    clearTimeout(this.#timer)
    const run = this.#saving.then(async () => {
      if (!this.dirty) return
      const text = this.text
      const key = this.key
      await this.#write(text)
      forget(key)
      if (this.text === text) this.dirty = false
      // Typed on while it was on its way, or under a key the note has only now.
      else keep(this.key, this.text)
    })
    this.#saving = run.catch(() => {})
    return run
  }

  async #autosave() {
    try {
      await this.save()
      this.#retry = 0
    } catch (error) {
      // Nothing is lost — it is kept here — but it is not on the server either,
      // so a server that cannot be reached is tried again, less often the
      // longer it stays away.
      if (unreachable(error)) {
        this.#retry = Math.min(Math.max(this.#retry * 2, 2000), 60_000)
        this.#timer = setTimeout(() => void this.#autosave(), this.#retry)
      }
      this.#fail(error, this.#retry)
    }
  }

  /** `:q!`: what the server does not have yet is let go. */
  discard() {
    clearTimeout(this.#timer)
    forget(this.key)
    this.dirty = false
  }
}

/* A full or disabled localStorage must not stop the typing. */

function read(key: string): string | null {
  try {
    return localStorage.getItem(key)
  } catch {
    return null
  }
}

function keep(key: string, text: string) {
  try {
    localStorage.setItem(key, text)
  } catch {
    /* nothing to do */
  }
}

function forget(key: string) {
  try {
    localStorage.removeItem(key)
  } catch {
    /* nothing to do */
  }
}
