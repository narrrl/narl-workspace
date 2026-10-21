/*
 * A fenced code block, coloured the way the editor colours it: the same
 * parser for the language named after the fence, the same classes. Each
 * language is fetched the first time a block on the page is written in it,
 * as KaTeX is for math; until it arrives the block shows plain. The languages
 * are state, so markdown rendered without one renders again once it is here.
 */
import type { Language } from '@codemirror/language'

/** What colours a block once there is one to colour, fetched with its first language. */
type Engine = { highlightCode: typeof import('@lezer/highlight').highlightCode; highlighter: typeof import('./highlight').highlighter }
let engine: Engine | null = null

/** By the name after the fence; null for a name no language answers to. */
let loaded = $state.raw(new Map<string, Language | null>())
const asked = new Set<string>()

/** The block as coloured HTML, or null while its language loads or when there is none. */
export function highlighted(source: string, fence: string): string | null {
  const name = fence.trim().split(/\s/)[0].toLowerCase()
  if (!name) return null
  const language = loaded.get(name)
  if (language === undefined) {
    load(name)
    return null
  }
  if (language === null || !engine) return null

  const { highlightCode, highlighter } = engine
  let html = ''
  highlightCode(
    source,
    language.parser.parse(source),
    highlighter,
    (text, classes) => (html += classes ? `<span class="${classes}">${escape(text)}</span>` : escape(text)),
    () => (html += '\n'),
  )
  return html
}

function load(name: string) {
  if (asked.has(name)) return
  asked.add(name)
  const settle = (language: Language | null) => (loaded = new Map(loaded).set(name, language))
  // The list of languages and the parsers' machinery come with CodeMirror,
  // which the reader does not otherwise need, so they are fetched with the
  // first language.
  Promise.all([
    import('@codemirror/language'),
    import('@codemirror/language-data'),
    import('@lezer/highlight'),
    import('./highlight'),
  ])
    .then(([{ LanguageDescription }, { languages }, { highlightCode }, { highlighter }]) => {
      engine = { highlightCode, highlighter }
      const description = LanguageDescription.matchLanguageName(languages, name, true)
      return description ? description.load().then((support) => support.language) : null
    })
    .then(settle)
    .catch((e) => {
      console.error(`code: could not load ${name}`, e)
      settle(null)
    })
}

function escape(text: string): string {
  return text.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;')
}
