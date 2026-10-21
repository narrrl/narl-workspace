import DOMPurify from 'dompurify'
import { marked, type TokenizerAndRendererExtension } from 'marked'
import { mathEngine } from './math.svelte'
import { hardWrapped, titleHref } from './util'

marked.setOptions({ gfm: true, breaks: true })

/*
 * Math the way GitHub writes it: `$x^2$` inline, `$$…$$` on lines of its own
 * or inline for display, and a ```math block. A `$` opening on a space or
 * closing before a digit is a price, not math — "$5 and $10" stays text — and
 * `\$` is a dollar sign as ever.
 */
const DISPLAY = /^\$\$((?:\\.|[^\\$])+?)\$\$/
const INLINE = /^\$(?!\s)((?:\\.|[^\\$\n])+?)(?<!\s)\$(?!\d)/

const mathBlock: TokenizerAndRendererExtension = {
  name: 'mathBlock',
  level: 'block',
  start: (src) => src.match(/^\$\$/m)?.index,
  tokenizer(src) {
    const match = /^\$\$((?:\\.|[^\\$])+?)\$\$[^\S\n]*(?:\n+|$)/.exec(src)
    if (match) return { type: 'mathBlock', raw: match[0], text: match[1].trim() }
  },
  renderer: (token) => tex(token.text, true),
}

const mathInline: TokenizerAndRendererExtension = {
  name: 'mathInline',
  level: 'inline',
  start: (src) => src.indexOf('$'),
  tokenizer(src) {
    const display = DISPLAY.exec(src)
    if (display) return { type: 'mathInline', raw: display[0], text: display[1].trim(), display: true }
    const inline = INLINE.exec(src)
    if (inline) return { type: 'mathInline', raw: inline[0], text: inline[1], display: false }
  },
  renderer: (token) => tex(token.text, token.display),
}

/*
 * `[[Plan]]` and `[[Plan|the plan]]`: a link to a document by its title. The
 * server says where each title leads, since it is the one that knows the
 * titles; one that leads nowhere is drawn dimmed, and following it offers to
 * write the document.
 */
const WIKILINK = /^\[\[([^[\]|\n]+)(?:\|([^[\]\n]*))?\]\]/

/** What the page being rendered says about its `[[titles]]`, for the renderer. */
let wikiContext: { leads: Record<string, string>; sharing: boolean } = { leads: {}, sharing: false }

const wikilink: TokenizerAndRendererExtension = {
  name: 'wikilink',
  level: 'inline',
  start: (src) => src.indexOf('[['),
  tokenizer(src) {
    const match = WIKILINK.exec(src)
    const target = match?.[1].trim()
    if (match && target) return { type: 'wikilink', raw: match[0], target, label: match[2]?.trim() || target }
  },
  renderer: (token) => {
    const label = escapeHtml(token.label)
    // The page it leads to is behind the login, as any /n/ link on a share is.
    if (wikiContext.sharing) return `<span class="private-link" title="a page of the private workspace">${label}</span>`
    const path = wikiContext.leads[token.target]
    if (path) return `<a href="/n/${encodeURI(path)}" class="wikilink">${label}</a>`
    return `<a href="${titleHref(token.target)}" class="wikilink missing" title="not written yet — follow it to write it">${label}</a>`
  },
}

/*
 * A line break typed in the browser is meant: the diary is written a line at a
 * time, so `breaks` is on. A paragraph an editor wrapped at its width is the
 * exception, and its breaks are read as the spaces they stand for. A break
 * asked for outright — two spaces or a `\` at the end of the line — stays.
 */
type Inline = { type: string; raw: string; tokens?: Inline[] }
function unwrap(tokens: Inline[]) {
  for (const [at, token] of tokens.entries()) {
    if (token.type === 'br' && /^ ?\n$/.test(token.raw)) tokens[at] = { type: 'text', raw: token.raw, text: ' ' } as Inline
    else if (token.tokens) unwrap(token.tokens)
  }
}

marked.use({
  extensions: [mathBlock, mathInline, wikilink],
  walkTokens(token) {
    if (token.type === 'paragraph' && hardWrapped(token.raw)) unwrap(token.tokens as Inline[])
  },
  renderer: {
    code(token) {
      return token.lang === 'math' ? tex(token.text, true) : false
    },
  },
})

/** TeX as KaTeX sets it, or as its source while KaTeX is still loading. */
function tex(source: string, display: boolean): string {
  const katex = mathEngine()
  if (!katex) {
    const text = escapeHtml(source)
    return display ? `<pre class="math">${text}</pre>` : `<code class="math">${text}</code>`
  }
  // An error shows in place, in red, rather than throwing away the page.
  return katex.renderToString(source, { displayMode: display, throwOnError: false })
}

/*
 * A link that opens another window leaves it no handle on this one. Only
 * `_blank` is kept as a target: a named one reuses whatever window has that
 * name, and browsers give `noopener` to `_blank` alone.
 */
DOMPurify.addHook('afterSanitizeAttributes', (node) => {
  if (node.tagName !== 'A' || !node.hasAttribute('target')) return
  if (node.getAttribute('target') === '_blank') node.setAttribute('rel', 'noopener noreferrer')
  else node.removeAttribute('target')
})

const SANITIZE = {
  // `semantics` and `annotation` carry KaTeX's MathML, for screen readers.
  ADD_TAGS: ['video', 'audio', 'source', 'semantics', 'annotation'],
  ADD_ATTR: ['controls', 'target', 'loading', 'playsinline', 'preload', 'encoding'],
}

function escapeHtml(text: string): string {
  return text.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;')
}

/**
 * Render entry markdown to sanitised HTML. On a share page, media URLs are
 * rewritten to the public route under the share's token. That route asks for
 * the cookie reading the document with its key handed out, so the key itself
 * never sits in a URL a proxy could log. A link to another document, an
 * address inside the private workspace that would lead the reader to its
 * login page, is kept as its text. `wikilinks` says where the `[[titles]]`
 * lead, as the server answered for the document.
 */
export function renderMarkdown(
  source: string,
  { share, wikilinks = {} }: { share?: { token: string }; wikilinks?: Record<string, string> } = {},
): string {
  const md = share
    ? source.replaceAll('/api/media/', `/api/share/${encodeURIComponent(share.token)}/media/`)
    : source

  wikiContext = { leads: wikilinks, sharing: !!share }
  const html = marked.parse(md, { async: false }) as string
  if (!share) return DOMPurify.sanitize(html, SANITIZE)

  // On the parsed page, not on its text: a pattern over HTML can end inside
  // an attribute and turn what was quoted there into markup.
  const page = DOMPurify.sanitize(html, { ...SANITIZE, RETURN_DOM_FRAGMENT: true })
  for (const link of page.querySelectorAll('a[href^="/n/"]')) {
    const text = document.createElement('span')
    text.className = 'private-link'
    text.title = 'a page of the private workspace'
    text.append(...link.childNodes)
    link.replaceWith(text)
  }
  const holder = document.createElement('div')
  holder.append(page)
  return holder.innerHTML
}

/** The markdown snippet that embeds a freshly uploaded file. */
export function embedSnippet(file: { url: string; filename: string; mime: string }): string {
  const name = file.filename.replaceAll(']', '')
  if (file.mime.startsWith('image/')) return `![${name}](${file.url})`
  if (file.mime.startsWith('video/')) return `<video src="${file.url}" controls playsinline></video>`
  if (file.mime.startsWith('audio/')) return `<audio src="${file.url}" controls></audio>`
  return `[${name}](${file.url})`
}
