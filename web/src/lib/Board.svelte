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

  function pick(list: number, card: number) {
    workspace.listCursor = list
    workspace.cardCursor = card
  }
</script>

<div class="board">
  <div class="head label rule">
    <span class="marker"></span>
    <span class="name">{workspace.space?.slug ?? ''}</span>
    <span>board</span>
    <span class="count">
      {workspace.board?.lists.reduce((n, list) => n + list.cards.length, 0) ?? 0} cards
    </span>
  </div>

  <div class="columns" bind:this={columns}>
    {#each workspace.board?.lists ?? [] as list, listIndex (list.id)}
      <section class="column" class:on={listIndex === workspace.listCursor}>
        <header class="label">
          <span class="marker"></span>
          <span class="name">{list.name}</span>
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
              >
                <span class="title">
                  <span class="tick" class:ticked={card.done_at !== null}></span>
                  <span class="text">{card.title}</span>
                </span>
                {#if card.due_at !== null || card.node_id !== null}
                  <span class="meta">
                    {#if card.due_at !== null}
                      <span class={urgency(card)}>due {formatDay(card.due_at)}</span>
                    {/if}
                    {#if card.node_id !== null}
                      <span class="doc" title="Enter opens it">→ {card.node_name ?? `#${card.node_id}`}</span>
                    {/if}
                  </span>
                {/if}
              </button>
            </li>
          {:else}
            <li class="empty muted">no cards — o adds one</li>
          {/each}
        </ul>
      </section>
    {:else}
      <div class="empty muted">
        no lists yet — :list &lt;name&gt; makes a column. then o adds a card to it.
      </div>
    {/each}
  </div>
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
</style>
