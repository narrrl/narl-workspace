<script lang="ts">
  /*
   * The board of the current space: columns of cards, in the same terminal
   * idiom as the list. The keys live in App.svelte with every other key, so
   * this file only draws and — for a pointer — moves the two cursors.
   */
  import type { Card } from './api'
  import { workspace } from './store.svelte'
  import { formatDay } from './util'

  let columns = $state<HTMLDivElement>()

  // Keep the selected card on screen while walking the board with the keyboard.
  $effect(() => {
    const at = `${workspace.listCursor}:${workspace.cardCursor}`
    columns?.querySelector<HTMLElement>(`[data-at="${at}"]`)?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  })

  const midnight = () => {
    const today = new Date()
    today.setHours(0, 0, 0, 0)
    return Math.floor(today.getTime() / 1000)
  }

  /** `overdue`, `today` or nothing — a date only earns colour when it bites. */
  function urgency(card: Card): '' | 'today' | 'overdue' {
    if (card.due_at === null || card.done_at !== null) return ''
    const start = midnight()
    if (card.due_at < start) return 'overdue'
    return card.due_at < start + 86400 ? 'today' : ''
  }

  const total = $derived(workspace.board?.lists.reduce((n, list) => n + list.cards.length, 0) ?? 0)
  const lastIndex = $derived((workspace.board?.lists.length ?? 0) - 1)
  /** Done cards older than a week, kept off the board until `z` asks. */
  const hidden = $derived(
    (workspace.rawBoard?.lists.at(-1)?.cards.length ?? 0) - (workspace.board?.lists.at(-1)?.cards.length ?? 0),
  )

  /** What a column is for, by where it stands — the names are yours to choose. */
  function role(index: number): 'inbox' | 'done' | '' {
    if (lastIndex < 1) return ''
    return index === 0 ? 'inbox' : index === lastIndex ? 'done' : ''
  }

  /** The spaces that have a board, with the digit that switches to each. */
  const boards = $derived(
    workspace.spaces.flatMap((space, index) => (space.has_board && index < 10 ? [{ space, digit: (index + 1) % 10 }] : [])),
  )

  function pick(list: number, card: number) {
    workspace.listCursor = list
    workspace.cardCursor = card
  }
</script>

<div class="board">
  <div class="head label rule">
    <span class="marker"></span>
    <span class="name">{workspace.boardSpace?.slug ?? ''}</span>
    <span>board</span>
    {#if workspace.board}<span class="count">{total} {total === 1 ? 'card' : 'cards'}</span>{/if}
    <!-- Every board is a digit away: the digit switches space, and the board
         follows the space. -->
    {#if boards.length > 1}
      <span class="boards">
        {#each boards as { space, digit } (space.id)}
          <button class="other" class:here={space.id === workspace.boardSpaceId} onclick={() => workspace.guard(() => workspace.switchSpace(digit === 0 ? 9 : digit - 1))}>
            <span class="n">{digit}</span>{space.slug}
          </button>
        {/each}
      </span>
    {/if}
    <span class="kbd tile-key" title="the key that comes to this tile">b</span>
  </div>

  <div class="columns" bind:this={columns}>
    {#each workspace.board?.lists ?? [] as list, listIndex (list.id)}
      {@const kind = role(listIndex)}
      <section class="column" class:on={listIndex === workspace.listCursor} class:finished={kind === 'done'}>
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
        <header class="label" onclick={() => pick(listIndex, 0)}>
          <span class="marker"></span>
          <span class="name">{list.name}</span>
          {#if kind}<span class="role">{kind}</span>{/if}
          <span class="n">{list.cards.length}</span>
        </header>

        <ul>
          {#each list.cards as card, cardIndex (card.id)}
            <li>
              <button
                class="card"
                class:active={listIndex === workspace.listCursor && cardIndex === workspace.cardCursor}
                class:done={card.done_at !== null}
                data-at="{listIndex}:{cardIndex}"
                onclick={() => pick(listIndex, cardIndex)}
                ondblclick={() => workspace.guard(() => workspace.openCard())}
                title={card.node_id === null ? 'Enter starts a document for it' : 'Enter opens its document'}
              >
                <span class="title">
                  <span class="tick" class:ticked={card.done_at !== null}></span>
                  <span class="text">{card.title}</span>
                </span>
                {#if card.due_at !== null || card.node_id !== null || card.repeat !== null}
                  <span class="meta">
                    {#if card.due_at !== null}
                      <span class={urgency(card)}>due {formatDay(card.due_at)}</span>
                    {/if}
                    {#if card.repeat !== null}
                      <span title="comes back in the inbox once done">↻ {card.repeat}</span>
                    {/if}
                    {#if card.node_id !== null}
                      <span class="doc">→ {card.node_name ?? `#${card.node_id}`}</span>
                    {/if}
                  </span>
                {/if}
              </button>
            </li>
          {:else}
            <li class="empty muted">
              {#if kind === 'done'}nothing finished this week{:else if kind === 'inbox'}empty — c captures from anywhere{:else}no cards — o adds one{/if}
            </li>
          {/each}
          {#if kind === 'done' && (hidden > 0 || workspace.showOldDone)}
            <li>
              <button class="older" onclick={() => (workspace.showOldDone = !workspace.showOldDone)}>
                {workspace.showOldDone ? 'hide older' : `+${hidden} older`} <span class="kbd">z</span>
              </button>
            </li>
          {/if}
        </ul>
      </section>
    {:else}
      <div class="empty muted">
        {#if !workspace.boardSpace}
          no space has a board — :set board gives the one you are in one.
        {:else}
          no lists yet — :list &lt;name&gt; makes a column. then o adds a card to it.
        {/if}
      </div>
    {/each}
  </div>

  <!-- The board's keys are not the list's, so they are spelled out where they apply. -->
  <footer class="label keys">
    <span><span class="kbd">h</span><span class="kbd">j</span><span class="kbd">k</span><span class="kbd">l</span> move</span>
    <span><span class="kbd">H</span><span class="kbd">L</span> send</span>
    <span><span class="kbd">␣</span> done</span>
    <span><span class="kbd">↵</span> notes</span>
    <span><span class="kbd">o</span> new</span>
    <span><span class="kbd">D</span> due</span>
    <span><span class="kbd">R</span> rename</span>
    <span><span class="kbd">dd</span> delete</span>
    <span><span class="kbd">1</span>…<span class="kbd">9</span> board</span>
    <span><span class="kbd">tab</span> tile</span>
    <span><span class="kbd">esc</span> leave</span>
    <span class="rule-hint">first list is the inbox · last list is done</span>
  </footer>
</div>

<style>
  .board {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  .head {
    flex: none;
    padding: 16px 20px 10px 22px;
    white-space: nowrap;
  }
  .head .marker { background: var(--light); }
  .head .name { color: var(--light); }
  .boards { display: flex; gap: 4px; margin-left: 8px; }
  /* After the rule, at the right end, where the other tiles have theirs. */
  .head .tile-key { order: 1; margin-left: 12px; }
  .other {
    display: flex;
    gap: 6px;
    padding: 2px 8px;
    border: 0;
    border-radius: var(--radius-s);
    font-size: 11px;
    letter-spacing: 0.06em;
    text-transform: none;
    color: var(--muted);
  }
  .other .n { color: var(--outline); }
  .other.here { background: var(--surface-alt); color: var(--light); }
  .other.here .n { color: var(--light-dim); }

  .columns {
    flex: 1;
    min-height: 0;
    display: flex;
    align-items: stretch;
    gap: 10px;
    padding: 4px 14px 14px;
    overflow: auto;
  }

  .column {
    display: flex;
    flex-direction: column;
    min-width: 28ch;
    flex: 1 1 0;
    max-width: 46ch;
    min-height: 0;
    border-radius: var(--radius-m);
    background: color-mix(in oklab, var(--surface) 70%, transparent);
    border: 1px solid transparent;
    transition: border-color var(--t-base) var(--ease-glide), background-color var(--t-base) var(--ease-glide);
  }

  /* The list the cursor is on: its marker lights and its edge shows. */
  .column.on { border-color: var(--rim); background: var(--surface); }

  .column header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px 8px;
  }
  .column header .name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .column.on header .marker { background: var(--light); }
  .column.on header .name { color: var(--light); }
  .n { font-variant-numeric: tabular-nums; }
  .role { color: var(--muted); font-size: 10px; letter-spacing: 0.18em; }
  .column header { cursor: default; }
  .column.finished:not(.on) { opacity: 0.7; }

  .older {
    display: flex;
    gap: 6px;
    align-items: center;
    width: 100%;
    margin-top: 4px;
    padding: 4px 10px;
    border: 0;
    font-size: 11px;
    color: var(--muted);
  }
  .older:hover { color: var(--light); background: transparent; }

  .keys {
    flex: none;
    display: flex;
    flex-wrap: wrap;
    gap: 6px 16px;
    align-items: center;
    padding: 8px 22px 12px;
    font-size: 11px;
  }
  .keys > span { display: flex; gap: 4px; align-items: center; }
  .keys .rule-hint { margin-left: auto; color: var(--muted); text-transform: none; letter-spacing: 0.04em; }

  ul {
    list-style: none;
    margin: 0;
    padding: 2px 6px 10px;
    overflow-y: auto;
    flex: 1;
  }

  .card {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 3px;
    width: 100%;
    margin: 1px 0;
    border: 0;
    border-radius: var(--radius-s);
    padding: 7px 10px;
    text-align: left;
    white-space: normal;
    font-size: 13px;
    letter-spacing: 0;
    text-transform: none;
    color: var(--fg);
  }

  .card:hover { background: var(--surface-alt); color: var(--light); border-color: transparent; }
  .card:active { transform: none; }
  .card.active, .card:focus-visible { background: var(--light); color: var(--ink); }
  .card.active :is(.meta, .today, .overdue, .doc) { color: var(--ink); }
  .card.active .tick { border-color: var(--ink); }
  .card.active .tick.ticked { background: var(--ink); }
  .card.done .text { text-decoration: line-through; color: var(--muted); }
  .card.done.active .text { color: var(--ink); }

  .title { display: flex; align-items: baseline; gap: 9px; font-family: var(--font-sans); font-size: 14px; }
  .tick {
    width: 9px;
    height: 9px;
    flex: none;
    border: 1px solid var(--outline);
    border-radius: 2px;
    transform: translateY(1px);
  }
  .tick.ticked { background: var(--light-dim); border-color: var(--light-dim); }
  .meta { display: flex; gap: 10px; font-size: 11px; padding-left: 18px; color: var(--muted); }
  .meta .today { color: var(--light); }
  .meta .overdue { color: var(--alarm); }
  .doc { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 24ch; }

  .empty { padding: 12px 10px; font-size: 12px; }

  @media (pointer: coarse) {
    .card { padding: 10px; }
  }
  @media (max-width: 767px) {
    .keys > span:not(.rule-hint) { display: none; }
    .keys .rule-hint { margin-left: 0; }
  }
</style>
