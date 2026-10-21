const pad = (n: number) => String(n).padStart(2, '0')

export function toDate(seconds: number): Date {
  return new Date(seconds * 1000)
}

/** `2026-09-05` — the diary is organised by day, so this is the primary label. */
export function formatDay(seconds: number): string {
  const d = toDate(seconds)
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
}

export function formatTime(seconds: number): string {
  const d = toDate(seconds)
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`
}

export function formatStamp(seconds: number): string {
  return `${formatDay(seconds)} ${formatTime(seconds)}`
}

export function formatWeekday(seconds: number): string {
  return toDate(seconds).toLocaleDateString(undefined, { weekday: 'short' }).toLowerCase()
}

export function relative(seconds: number): string {
  const delta = Math.floor(Date.now() / 1000) - seconds
  if (delta < 60) return 'just now'
  if (delta < 3600) return `${Math.floor(delta / 60)}m ago`
  if (delta < 86400) return `${Math.floor(delta / 3600)}h ago`
  if (delta < 86400 * 30) return `${Math.floor(delta / 86400)}d ago`
  return formatDay(seconds)
}

export function formatBytes(bytes: number): string {
  const units = ['B', 'K', 'M', 'G']
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit++
  }
  return `${value < 10 && unit > 0 ? value.toFixed(1) : Math.round(value)}${units[unit]}`
}

/** Parse the `YYYY-MM-DD` argument of `:date` into a Unix timestamp at noon. */
export function parseDay(input: string): number | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(input.trim())
  if (!match) return null
  const [, y, m, d] = match
  const date = new Date(Number(y), Number(m) - 1, Number(d), 12, 0, 0)
  return Number.isNaN(date.getTime()) ? null : Math.floor(date.getTime() / 1000)
}

const WEEKDAYS = ['sunday', 'monday', 'tuesday', 'wednesday', 'thursday', 'friday', 'saturday']

/**
 * A due date the way it is said rather than written: `today`, `tm` or
 * `tomorrow`, a weekday (`fri` is the next Friday, never today), `+3d`, `+2w`,
 * `10-31` (this year, or next if that has passed) or a full `2026-10-31`.
 * Noon, like `parseDay`, so no timezone shift can tip it into another day.
 */
export function parseDue(input: string, now = new Date()): number | null {
  const text = input.trim().toLowerCase()
  const day = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 12, 0, 0)
  const at = (d: Date) => Math.floor(d.getTime() / 1000)
  const shifted = (days: number) => at(new Date(day.getFullYear(), day.getMonth(), day.getDate() + days, 12))

  if (text === 'today' || text === 'td') return at(day)
  if (text === 'tomorrow' || text === 'tm') return shifted(1)
  const weekday = text.length >= 2 ? WEEKDAYS.findIndex((name) => name.startsWith(text)) : -1
  if (weekday >= 0) return shifted(((weekday - day.getDay() + 6) % 7) + 1)
  const offset = /^\+(\d+)([dw])$/.exec(text)
  if (offset) return shifted(Number(offset[1]) * (offset[2] === 'w' ? 7 : 1))
  const monthDay = /^(\d{1,2})-(\d{1,2})$/.exec(text)
  if (monthDay) {
    const [m, d] = [Number(monthDay[1]) - 1, Number(monthDay[2])]
    let date = new Date(day.getFullYear(), m, d, 12)
    if (date.getMonth() !== m) return null
    if (date < day) date = new Date(day.getFullYear() + 1, m, d, 12)
    return at(date)
  }
  return parseDay(text)
}

/**
 * True on phones and tablets, where modal editing would be a burden. Both
 * checks are needed: a touchscreen laptop reports touch points but still has a
 * fine primary pointer, and headless browsers report `hover: none`.
 */
export const isTouchDevice =
  typeof window !== 'undefined' &&
  navigator.maxTouchPoints > 0 &&
  window.matchMedia('(pointer: coarse)').matches

/** A heading as an anchor id: lowercase, dashes, the same idea as the server's slugs. */
export function slugify(text: string): string {
  return text
    .toLowerCase()
    .replace(/ä/g, 'ae')
    .replace(/ö/g, 'oe')
    .replace(/ü/g, 'ue')
    .replace(/ß/g, 'ss')
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 80)
}

/** How far a reading view is asked to scroll: lines, a half page, or an end. */
export type ScrollAmount = number | 'top' | 'bottom' | 'halfdown' | 'halfup'

/** j/k, gg/G and Ctrl-d/u on a scrolling element, three lines to a step. */
export function scrollPage(el: HTMLElement, amount: ScrollAmount) {
  const line = parseFloat(getComputedStyle(el).lineHeight) || 24
  const half = el.clientHeight / 2
  switch (amount) {
    case 'top':
      el.scrollTo({ top: 0 })
      break
    case 'bottom':
      el.scrollTo({ top: el.scrollHeight })
      break
    case 'halfdown':
      el.scrollBy({ top: half })
      break
    case 'halfup':
      el.scrollBy({ top: -half })
      break
    default:
      el.scrollBy({ top: amount * line * 3 })
  }
}

/** `zl` / `zh` a few columns, `zL` / `zH` half a width, as vim scrolls sideways. */
export type SidewaysAmount = 1 | -1 | 'halfright' | 'halfleft'

/**
 * Scroll whatever is too wide for the column — a table, a code block, a
 * formula — sideways: the first one that reaches down past the top of the
 * view, the one being read. False when there is none on screen.
 */
export function scrollWide(scroller: HTMLElement, root: HTMLElement, amount: SidewaysAmount): boolean {
  const view = scroller.getBoundingClientRect()
  const wide = [...root.querySelectorAll<HTMLElement>('table, pre, .katex-display, .table')].find((el) => {
    if (el.scrollWidth <= el.clientWidth + 1) return false
    const box = el.getBoundingClientRect()
    return box.bottom > view.top + 8 && box.top < view.bottom
  })
  if (!wide) return false
  const step = typeof amount === 'number' ? amount * 80 : (amount === 'halfright' ? 1 : -1) * (wide.clientWidth / 2)
  wide.scrollBy({ left: step })
  return true
}

/** A heading to jump to: the reader's, or a line of the text being written. */
export interface Heading {
  text: string
  level: number
  go: () => void
}

/** Every heading of a rendered article, the document's own title left out. */
export const headingsIn = (root: HTMLElement) =>
  Array.from(root.querySelectorAll<HTMLElement>('h1, h2, h3, h4, h5, h6')).filter(
    (element) => !element.classList.contains('entrytitle'),
  )

/* How far below the top of the view a heading is left by a jump: enough room
   that it does not look cut off, little enough that it reads as the top. */
const HEADING_GAP = 12

/**
 * `]]` / `[[`: the next heading further down than the top of the view, or the
 * last one above it. A heading a jump has just parked at the top counts as
 * the one being read, so pressing again moves on rather than staying put.
 */
export function jumpHeading(scroller: HTMLElement, headings: HTMLElement[], direction: 1 | -1): boolean {
  const top = scroller.getBoundingClientRect().top
  const offset = (element: HTMLElement) => element.getBoundingClientRect().top - top
  let target: HTMLElement | undefined
  for (const element of headings) {
    const at = offset(element)
    if (direction > 0 && at > HEADING_GAP + 4) {
      target = element
      break
    }
    if (direction < 0 && at < HEADING_GAP - 4) target = element
  }
  if (!target) return false
  scroller.scrollBy({ top: offset(target) - HEADING_GAP })
  return true
}

/**
 * `gO` in the reader: its headings, and which of them is being read — the
 * last one that has passed the upper third of the view, as the outline lights.
 */
export function readerHeadings(scroller: HTMLElement, root: HTMLElement): { headings: Heading[]; current: number } {
  const elements = headingsIn(root)
  const top = scroller.getBoundingClientRect().top
  const line = top + scroller.clientHeight / 3
  let current = 0
  elements.forEach((element, i) => {
    if (element.getBoundingClientRect().top <= line) current = i
  })
  const headings = elements.map((element) => ({
    text: element.textContent?.trim() ?? '',
    level: Number(element.tagName[1]),
    go: () => scroller.scrollBy({ top: element.getBoundingClientRect().top - top - HEADING_GAP }),
  }))
  return { headings, current }
}

/**
 * The `#` lines of markdown source, with the line each stands on. Lines inside
 * a fenced block are code, whatever they start with.
 */
export function sourceHeadings(source: string): { text: string; level: number; line: number }[] {
  const found: { text: string; level: number; line: number }[] = []
  let fence: string | null = null
  source.split('\n').forEach((text, i) => {
    const marker = /^\s{0,3}(`{3,}|~{3,})/.exec(text)?.[1]
    if (marker) {
      if (fence === null) fence = marker[0]
      else if (marker[0] === fence) fence = null
      return
    }
    if (fence !== null) return
    const heading = /^\s{0,3}(#{1,6})\s+(.*?)(?:\s+#+)?\s*$/.exec(text)
    if (heading && heading[2]) found.push({ text: heading[2], level: heading[1].length, line: i + 1 })
  })
  return found
}

/**
 * The link the cursor stands on in a line of markdown — `[text](href)`, an
 * embed's `![alt](src)`, `[[title]]`, `<https://…>` or a bare address — or else the first
 * one after it on the line, as vim's `gf` looks ahead to the next name.
 */
export function linkAt(line: string, column: number): string | null {
  const pattern =
    /\[\[([^[\]|\n]+)(?:\|[^[\]\n]*)?\]\]|!?\[[^\]]*\]\(\s*<?([^)\s>]+)>?(?:\s+"[^"]*")?\s*\)|<((?:https?:\/\/|\/n\/)[^>\s]+)>|((?:https?:\/\/|\/n\/)[^\s<>()]+)/g
  let ahead: string | null = null
  for (const match of line.matchAll(pattern)) {
    // A sentence that ends on a bare address ends with a full stop, not the address.
    const href = match[1]?.trim()
      ? titleHref(match[1].trim())
      : (match[2] ?? match[3] ?? match[4]?.replace(/[.,;:!?]+$/, ''))
    if (!href) continue
    const start = match.index
    if (column >= start && column < start + match[0].length) return href
    if (start >= column) ahead ??= href
  }
  return ahead
}

/**
 * Where a `[[title]]` points when the page cannot say where it leads: the
 * title itself, for following it to look it up, or to offer to write it.
 */
export const titleHref = (title: string) => `/n/?title=${encodeURIComponent(title)}`

/** What a share link's reader will miss: a share never serves a sealed file. */
export const sealedNote = (sealed: number) =>
  sealed > 0 ? ` — ${sealed} sealed file${sealed === 1 ? '' : 's'} in it stay hidden from readers` : ''

/** Where a tile sits, as fractions of the space the tiles share. */
export type Rect = { x: number; y: number; w: number; h: number }
export type Direction = 'h' | 'j' | 'k' | 'l'

/**
 * Hyprland's dwindle: the first tile takes half, and each one after it takes
 * half of what is left, side by side and then one over the other in turn.
 */
export function dwindle(count: number): Rect[] {
  const rects: Rect[] = []
  let rest: Rect = { x: 0, y: 0, w: 1, h: 1 }
  for (let i = 0; i < count; i++) {
    if (i === count - 1) {
      rects.push(rest)
    } else if (i % 2 === 0) {
      rects.push({ ...rest, w: rest.w / 2 })
      rest = { ...rest, x: rest.x + rest.w / 2, w: rest.w / 2 }
    } else {
      rects.push({ ...rest, h: rest.h / 2 })
      rest = { ...rest, y: rest.y + rest.h / 2, h: rest.h / 2 }
    }
  }
  return rects
}

/**
 * The tile next to `from` in a direction: one that touches its side and
 * shares some of its length, the nearest and then the most overlapping.
 * -1 when it is at the edge.
 */
export function neighbour(rects: Rect[], from: number, direction: Direction): number {
  const a = rects[from]
  if (!a) return -1
  const near = 1e-6
  let best = -1
  let bestGap = Infinity
  let bestOverlap = 0
  rects.forEach((b, index) => {
    if (index === from) return
    const across = direction === 'h' || direction === 'l'
    const overlap = across
      ? Math.min(a.y + a.h, b.y + b.h) - Math.max(a.y, b.y)
      : Math.min(a.x + a.w, b.x + b.w) - Math.max(a.x, b.x)
    if (overlap <= near) return
    const gap =
      direction === 'l' ? b.x - (a.x + a.w)
      : direction === 'h' ? a.x - (b.x + b.w)
      : direction === 'j' ? b.y - (a.y + a.h)
      : a.y - (b.y + b.h)
    if (gap < -near) return
    if (gap < bestGap - near || (Math.abs(gap - bestGap) <= near && overlap > bestOverlap)) {
      best = index
      bestGap = gap
      bestOverlap = overlap
    }
  })
  return best
}

/**
 * Whether a paragraph was wrapped by an editor at a fixed width rather than
 * broken by hand: every line but the last is long, and each break comes
 * where the next word would not have fitted on the line before. Markdown
 * written elsewhere — an import, a save over the SFTP mount from vim with
 * `textwidth`, a pasted README — is wrapped that way, and keeping its breaks
 * puts one in the middle of every sentence. Lines broken by hand rarely pass:
 * a short sentence followed by a short word would have fitted, and a line
 * broken by hand mostly ends where a sentence does, which a wrapped one
 * seldom does.
 */
export function hardWrapped(paragraph: string): boolean {
  const lines = paragraph.trimEnd().split('\n').map((line) => line.trimEnd())
  if (lines.length < 2) return false
  const body = lines.slice(0, -1)
  const width = Math.max(...lines.map((line) => line.length))
  const fits = body.every((line, at) => {
    const next = lines[at + 1].trimStart().split(/\s/, 1)[0]
    return line.length >= 40 && line.length + 1 + next.length > width
  })
  const sentences = body.filter((line) => /[.!?:;]$/.test(line)).length
  return fits && sentences <= Math.floor((body.length - 1) / 3)
}

/**
 * A search excerpt cut into what to draw plainly and what to light: the
 * server puts each matched word between U+0002 and U+0003. A cut through the
 * middle of a match leaves an opening without its close, lit to the end.
 */
export function hitParts(excerpt: string): { text: string; lit: boolean }[] {
  const parts: { text: string; lit: boolean }[] = []
  let lit = false
  for (const piece of excerpt.split(/([\u0002\u0003])/)) {
    if (piece === '\u0002') lit = true
    else if (piece === '\u0003') lit = false
    else if (piece) parts.push({ text: piece, lit })
  }
  return parts
}
