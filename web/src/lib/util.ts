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

/** What a share link's reader will miss: a share never serves a sealed file. */
export const sealedNote = (sealed: number) =>
  sealed > 0 ? ` — ${sealed} sealed file${sealed === 1 ? '' : 's'} in it stay hidden from readers` : ''
