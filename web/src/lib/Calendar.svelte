<script lang="ts">
  /*
   * `C`: the diary by month, the way a paper one is leafed through — the days
   * written in marked, the day under the cursor shown below. The keys live in
   * App.svelte with the others; this draws and lets a pointer do the same.
   */
  import { workspace } from './store.svelte'
  import { formatDay } from './util'

  const WEEKDAYS = ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun']

  const month = $derived(workspace.calendarDay.slice(0, 7))
  const title = $derived.by(() => {
    const [y, m] = month.split('-').map(Number)
    return new Date(y, m - 1, 1).toLocaleDateString(undefined, { month: 'long', year: 'numeric' }).toLowerCase()
  })

  /** The month's days in weeks from Monday, with blanks before the first. */
  const cells = $derived.by(() => {
    const [y, m] = month.split('-').map(Number)
    const lead = (new Date(y, m - 1, 1).getDay() + 6) % 7
    const length = new Date(y, m, 0).getDate()
    const days: (string | null)[] = Array(lead).fill(null)
    for (let d = 1; d <= length; d++) days.push(`${month}-${String(d).padStart(2, '0')}`)
    return days
  })

  const today = $derived(formatDay(Math.floor(Date.now() / 1000)))
  const counts = $derived(
    workspace.calendarEntries.reduce((all, entry) => all.set(entry.day, (all.get(entry.day) ?? 0) + 1), new Map<string, number>()),
  )
  const chosen = $derived(workspace.calendarEntries.filter((entry) => entry.day === workspace.calendarDay))

  function pick(day: string) {
    workspace.calendarDay = day
    void workspace.guard(() => workspace.openCalendarDay())
  }
</script>

<div class="sheet">
  <header>
    <span class="label rule"><span class="marker"></span><span class="title">{title}</span> {workspace.calendarEntries.length} entries</span>
    <button onclick={() => workspace.guard(() => workspace.moveCalendar(0, -1))} aria-label="previous month">‹</button>
    <button onclick={() => workspace.guard(() => workspace.moveCalendar(0, 1))} aria-label="next month">›</button>
    <button onclick={() => (workspace.overlay = 'none')}>esc</button>
  </header>

  <ol class="grid">
    {#each WEEKDAYS as weekday (weekday)}
      <li class="weekday label">{weekday}</li>
    {/each}
    {#each cells as day, index (day ?? `blank-${index}`)}
      {#if day}
        <li>
          <button
            class="day"
            class:at={day === workspace.calendarDay}
            class:today={day === today}
            class:written={counts.has(day)}
            onclick={() => pick(day)}
          >
            <span class="n">{Number(day.slice(8))}</span>
            {#if (counts.get(day) ?? 0) > 1}<span class="many">{counts.get(day)}</span>{:else if counts.has(day)}<span class="dot"></span>{/if}
          </button>
        </li>
      {:else}
        <li></li>
      {/if}
    {/each}
  </ol>

  <section class="chosen">
    <h2>{workspace.calendarDay}</h2>
    {#each chosen as entry (entry.id)}
      <p class="entry"><span class="name">{entry.name.trim() || entry.excerpt || 'untitled'}</span></p>
    {:else}
      <p class="none">nothing written — <span class="kbd">↵</span> starts it</p>
    {/each}
  </section>

  <footer class="label">
    <span class="kbd">h</span><span class="kbd">l</span> day
    <span class="gap"></span>
    <span class="kbd">j</span><span class="kbd">k</span> week
    <span class="gap"></span>
    <span class="kbd">H</span><span class="kbd">L</span> month
    <span class="gap"></span>
    <span class="kbd">.</span> today
    <span class="gap"></span>
    <span class="kbd">↵</span> open
    <span class="gap"></span>
    <span class="kbd">C</span><span class="kbd">esc</span> close
  </footer>
</div>

<style>
  .sheet {
    display: flex;
    flex-direction: column;
    gap: 16px;
    width: min(100%, 60ch);
    max-height: 100%;
    overflow-y: auto;
    background: color-mix(in oklab, var(--hull) 97%, transparent);
    border: 1px solid var(--surface-alt);
    border-radius: var(--frame-radius);
    padding: 20px 24px;
  }

  header { display: flex; align-items: center; gap: 12px; }
  header .label { flex: 1; }
  header .marker { background: var(--light); }
  header .title { color: var(--light); }

  .grid {
    display: grid;
    grid-template-columns: repeat(7, 1fr);
    gap: 4px;
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .weekday { text-align: center; font-size: 10px; color: var(--muted); }

  .day {
    position: relative;
    display: grid;
    place-items: center;
    width: 100%;
    aspect-ratio: 1.4;
    padding: 0;
    border: 1px solid transparent;
    border-radius: var(--radius-s);
    font-size: 13px;
    letter-spacing: 0;
    color: var(--subtle);
    font-variant-numeric: tabular-nums;
  }
  .day:active { transform: none; }
  .day.written { color: var(--fg); }
  .day.today { border-color: var(--rim); }
  .day.at { background: var(--light); color: var(--ink); }
  .dot, .many { position: absolute; bottom: 4px; }
  .dot { width: 5px; height: 5px; border-radius: 999px; background: var(--light); }
  .many { font-size: 9px; color: var(--light); }
  .day.at :is(.dot) { background: var(--ink); }
  .day.at .many { color: var(--ink); }

  .chosen h2 {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 0 0 6px;
    font-size: 10px;
    font-weight: 400;
    letter-spacing: 0.22em;
    color: var(--muted);
  }
  .chosen h2::after { content: ''; height: 1px; flex: 1; background: var(--overlay); }
  .entry, .none { margin: 2px 0; font-size: 13px; }
  .entry .name { font-family: var(--font-sans); font-size: 14px; color: var(--fg); }
  .none { color: var(--subtle); }

  footer { display: flex; flex-wrap: wrap; align-items: center; gap: 4px; font-size: 10px; }
  footer .kbd { margin-right: 2px; }
  footer .gap { width: 14px; }

  @media (max-width: 767px), (pointer: coarse) {
    footer { display: none; }
  }
</style>
