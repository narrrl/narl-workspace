<script lang="ts">
  /*
   * `t`: what needs you, from every board at once — what is late, what is due
   * today and this week, what is already in hand — and whether the diary has
   * had today yet. The keys live in App.svelte with the others; this draws
   * and lets a pointer do the same.
   */
  import Keys from './Keys.svelte'
  import type { AgendaCard, DiaryEntry } from './api'
  import { workspace } from './store.svelte'
  import { runCommand } from './commands'
  import Icon from './Icon.svelte'
  import { formatDay, formatWeekday, isTouchDevice } from './util'

  let sheet = $state<HTMLDivElement>()

  const titles = { overdue: 'late', today: 'today', week: 'this week', doing: 'in hand' } as const

  const groups = $derived(workspace.cards.agendaGroups.filter((group) => group.cards.length > 0))
  const total = $derived(workspace.cards.agendaCards.length)

  $effect(() => {
    const index = workspace.cards.agendaCursor
    sheet?.querySelector<HTMLElement>(`[data-index="${index}"]`)?.scrollIntoView({ block: 'nearest' })
  })

  /** An entry from an earlier year, in place in the diary. */
  function open(entry: DiaryEntry) {
    void workspace.guard(() => workspace.tree.openPath(entry.path))
  }

  /** Where a card sits in the walked order, across the groups. */
  function indexOf(card: AgendaCard) {
    return workspace.cards.agendaCards.findIndex((c) => c.id === card.id)
  }
</script>

<div class="sheet" bind:this={sheet}>
  <header>
    <span class="label rule"><span class="marker"></span><span class="title">agenda</span> {total} open</span>
    <span class="kbd tile-key" title="the key that comes to this tile">t</span>
  </header>

  {#if workspace.cards.agenda.diary_today === false}
    <p class="diary">
      <span class="dot"></span> nothing in the diary today
      {#if isTouchDevice}
        <button class="write" onclick={() => runCommand('today')}>write it</button>
      {:else}
        <span class="kbd">T</span> writes it
      {/if}
    </p>
  {/if}

  {#each groups as group (group.key)}
    <section class={group.key}>
      <h2>{titles[group.key]} <span class="n">{group.cards.length}</span></h2>
      <ul>
        {#each group.cards as card (card.id)}
          {@const index = indexOf(card)}
          <li>
            <button
              class="row"
              class:at={index === workspace.cards.agendaCursor}
              data-index={index}
              onclick={() => workspace.guard(() => workspace.cards.goToCard(card))}
              onpointerenter={() => (workspace.cards.agendaCursor = index)}
            >
              <span class="text">{card.title}</span>
              <span class="where">{card.space_slug}/{card.list_name}</span>
              {#if card.repeat !== null}
                <span class="due" title="comes back in the inbox once done">↻ {card.repeat}</span>
              {/if}
              {#if card.due_at !== null}
                <span class="due">{formatWeekday(card.due_at)} {formatDay(card.due_at).slice(5)}</span>
              {/if}
            </button>
            {#if isTouchDevice}
              <!-- Space, for a thumb. -->
              <button class="finish" aria-label="done: {card.title}" onclick={() => workspace.guard(() => workspace.cards.toggleDone(card))}>
                <Icon name="check" size={18} />
              </button>
            {/if}
          </li>
        {/each}
      </ul>
    </section>
  {:else}
    <p class="clear">nothing late, nothing due this week, nothing in hand. <span class="kbd">c</span> captures the next thing.</p>
  {/each}

  {#if workspace.cards.agenda.on_this_day.length}
    <section class="past">
      <h2>on this day <span class="n">{workspace.cards.agenda.on_this_day.length}</span></h2>
      <ul>
        {#each workspace.cards.agenda.on_this_day as entry, at (entry.id)}
          {@const index = total + at}
          <li>
            <button
              class="row"
              class:at={index === workspace.cards.agendaCursor}
              data-index={index}
              onclick={() => open(entry)}
              onpointerenter={() => (workspace.cards.agendaCursor = index)}
            >
              <span class="text">{entry.name.trim() || entry.excerpt || 'untitled'}</span>
              <span class="due">{entry.day.slice(0, 4)}</span>
            </button>
          </li>
        {/each}
      </ul>
    </section>
  {/if}

  <Keys contexts={['agenda', 'views']} />
</div>

<style>
  li:has(.finish) { display: flex; align-items: center; }
  .finish { flex: none; display: grid; place-items: center; width: 44px; height: 40px; padding: 0; border: 0; background: none; color: var(--muted); }

  /* A tile's window is its frame, so it only fills it. */
  .sheet {
    display: flex;
    flex-direction: column;
    gap: 16px;
    height: 100%;
    overflow-y: auto;
    padding: 20px 24px;
  }

  header { display: flex; align-items: center; gap: 12px; }
  header .label { flex: 1; }
  header .marker { background: var(--light); }
  header .title { color: var(--light); }

  .diary {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0;
    padding: 8px 12px;
    border: 1px solid var(--rim);
    border-radius: var(--radius-m);
    font-size: 13px;
    color: var(--fg);
  }
  .diary .dot { width: 7px; height: 7px; border-radius: 999px; background: var(--light); }

  section { min-width: 0; }
  h2 {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 0 0 6px;
    font-size: 10px;
    font-weight: 400;
    letter-spacing: 0.22em;
    text-transform: uppercase;
    color: var(--muted);
  }
  h2::after { content: ''; height: 1px; flex: 1; background: var(--overlay); }
  .overdue h2 { color: var(--alarm); }
  .today h2 { color: var(--light); }
  .n { font-variant-numeric: tabular-nums; letter-spacing: 0; }

  ul { list-style: none; margin: 0; padding: 0; }

  .row {
    display: flex;
    align-items: baseline;
    gap: 12px;
    width: 100%;
    margin: 1px 0;
    padding: 6px 10px;
    border: 0;
    border-radius: var(--radius-s);
    text-align: left;
    font-size: 13px;
    letter-spacing: 0;
    text-transform: none;
    color: var(--fg);
  }
  .row:hover { background: var(--surface-alt); border-color: transparent; }
  .row:active { transform: none; }
  .row.at { background: var(--light); color: var(--ink); }
  .row.at :is(.where, .due) { color: var(--ink); }
  .text {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-sans);
    font-size: 14px;
  }
  .where, .due { flex: none; font-size: 11px; color: var(--muted); font-variant-numeric: tabular-nums; }
  .overdue .due { color: var(--alarm); }
  .today .due { color: var(--light); }

  .clear { margin: 0; font-size: 13px; color: var(--subtle); }


  @media (max-width: 767px) {
    .sheet { padding: 14px; gap: 14px; }
    .where { display: none; }
  }
</style>
