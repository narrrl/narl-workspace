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

/** What a share link's reader will miss: a share never serves a sealed file. */
export const sealedNote = (sealed: number) =>
  sealed > 0 ? ` — ${sealed} sealed file${sealed === 1 ? '' : 's'} in it stay hidden from readers` : ''
