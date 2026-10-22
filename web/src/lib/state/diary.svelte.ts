import { api, type DiaryEntry } from '../api'
import { formatDay } from '../util'
import type { Workspace } from '../store.svelte'

/** The diary: today's entry, the calendar, and stepping from one day to the next. */
export class Diary {
  readonly ws: Workspace

  constructor(ws: Workspace) {
    this.ws = ws
  }

  /** `C`: the day the calendar's cursor is on, `2026-10-20`, and its month's entries. */
  calendarDay = $state('')
  calendarEntries = $state.raw<DiaryEntry[]>([])

  /** `C`: the calendar, on the day of the entry being read or else today. */
  async openCalendar() {
    const open = this.ws.open
    const reading = open && this.ws.tree.spaces.find((space) => space.id === open.path[0]?.id)?.role === 'diary'
    this.calendarDay = formatDay(reading ? open.created_at : Math.floor(Date.now() / 1000))
    await this.ws.tiles.show('calendar')
  }

  async loadCalendar() {
    this.calendarEntries = await api.diaryDays(this.calendarDay.slice(0, 7))
  }

  /** The calendar's cursor, `days` along or `months` over; a month keeps the day where it can. */
  async moveCalendar(days: number, months = 0) {
    const [y, m, d] = this.calendarDay.split('-').map(Number)
    const last = new Date(y, m - 1 + months + 1, 0).getDate()
    const date = new Date(y, m - 1 + months, Math.min(d, last) + days, 12)
    const before = this.calendarDay.slice(0, 7)
    this.calendarDay = formatDay(Math.floor(date.getTime() / 1000))
    if (this.calendarDay.slice(0, 7) !== before) await this.loadCalendar()
  }

  /** Enter in the calendar: that day's entry, or a new one dated that day. */
  async openCalendarDay() {
    const day = this.calendarDay
    const entry = this.calendarEntries.find((e) => e.day === day)
    // Beside the calendar, so `q` on the entry comes back to it.
    if (entry) return this.ws.tree.openPath(entry.path)
    await this.openDay(day)
  }

  /**
   * `T`, and a day in the calendar without an entry: the server finds the
   * day's entry or starts it, by its own clock and at the root of the diary,
   * as a capture does, so the three never write two entries for one day.
   */
  async openDay(day?: string) {
    const home = this.ws.tree.spaces.find((space) => space.role === 'diary')
    if (!home) return this.ws.say('there is no diary to write in', 'error')
    const entry = await api.diaryDay(day)
    if (this.ws.tree.spaceId !== home.id) await this.ws.tree.enterSpace(home.id)
    else if (this.ws.tree.query) await this.ws.tree.search('')
    if (entry.created) await this.ws.tree.refresh()
    await this.ws.tree.openNode(entry.id, true)
    if (entry.created) {
      // Left as it was started, it goes again, as a new document from `o` does.
      this.ws.tile.fresh = entry.id
      this.ws.enterInsert = true
    }
  }

  /** `[d` / `]d`: the diary entry before or after the one being read. */
  async stepEntry(by: -1 | 1) {
    const open = this.ws.open
    if (!open) return this.ws.say('nothing open', 'error')
    const { prev, next } = await api.diaryAround(open.id)
    const entry = by < 0 ? prev : next
    if (!entry) return this.ws.say(by < 0 ? 'no entry before this one' : 'no entry after this one')
    await this.ws.tree.openPath(entry.path)
    this.ws.say(entry.day)
  }

  /** `:today <text>`: a line at the end of today's diary entry, without going there. */
  async logToday(text: string) {
    await this.ws.drafts.flush()
    const landed = await api.capture({ text, to: 'diary' })
    await this.ws.drafts.rereadNode(landed.id)
    this.ws.say(`logged in ${landed.where}`)
  }
}
