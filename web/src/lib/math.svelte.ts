/*
 * KaTeX, fetched the first time something on the page has math in it rather
 * than with the app: it is larger than the rest of the app put together.
 * Until it arrives the math shows as its source. The engine is state, so
 * markdown rendered without it renders again once it is here.
 */
import type Katex from 'katex'

let engine = $state.raw<typeof Katex | null>(null)
let loading = false

export function mathEngine(): typeof Katex | null {
  if (!engine && !loading) {
    loading = true
    Promise.all([import('katex'), import('katex/dist/katex.min.css')])
      .then(([katex]) => (engine = katex.default))
      .catch((e) => console.error('math: could not load KaTeX', e))
  }
  return engine
}
