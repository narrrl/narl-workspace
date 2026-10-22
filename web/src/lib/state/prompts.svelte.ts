import type { Workspace } from '../store.svelte'

/**
 * Questions asked in the command line's place, and Proton's human check:
 * each a promise answered from the keyboard.
 */
export class Prompts {
  readonly ws: Workspace

  constructor(ws: Workspace) {
    this.ws = ws
  }

  /*
   * A `[y/N]` question in the command line's place, instead of the browser's
   * confirm(): it names the thing about to go, and it is answered from the
   * keyboard without a modal the rest of the page cannot see past.
   */
  prompt = $state<{ text: string; resolve: (yes: boolean) => void } | null>(null)
  /** A password asked for in the same place, typed into a masked field. */
  secret = $state.raw<{ text: string; resolve: (value: string | null) => void } | null>(null)

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
      this.ws.sheets.open('verify')
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
}
