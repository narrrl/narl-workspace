/*
 * ```typst blocks, typeset on the server. A block shows as its source until
 * its SVG arrives, as math does until KaTeX has loaded; the answers are
 * state, so markdown rendered without one renders again once it is here.
 * Each block is asked for once per page load, however often the page is
 * drawn — while writing, the preview asks only for a block that changed.
 */
import { api } from './api'

type Answer = { svg: string } | { error: string }

let answers = $state.raw(new Map<string, Answer>())
const asked = new Set<string>()

/** Forget a block once it is this far out of date: the preview of a long session asks for many. */
const KEEP = 200

export function typeset(source: string, share?: string): Answer | null {
  const key = `${share ?? ''}\n${source}`
  const answer = answers.get(key)
  if (answer || asked.has(key)) return answer ?? null
  asked.add(key)
  api
    .typeset(source, share)
    .then((done) => (done.svg !== undefined ? { svg: done.svg } : { error: done.error ?? 'typst said nothing' }))
    .catch((e: Error) => ({ error: e.message }))
    .then((done) => {
      const next = new Map(answers)
      if (next.size >= KEEP) next.delete(next.keys().next().value!)
      answers = next.set(key, done)
    })
  return null
}
