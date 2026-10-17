<script lang="ts">
  import type { NodeSummary } from './api'
  import { workspace } from './store.svelte'
  import { formatDay, formatTime, formatWeekday } from './util'

  let list = $state<HTMLUListElement>()

  // Keep the cursor line visible while navigating with j/k.
  $effect(() => {
    const index = workspace.cursor
    const node = list?.querySelector<HTMLElement>(`[data-index="${index}"]`)
    node?.scrollIntoView({ block: 'nearest' })
  })

  function label(node: NodeSummary) {
    return node.name.trim() || node.excerpt.trim() || node.slug
  }

  /* A folder is listed by what is in it; a document by when it was written. */
  const aside = (node: NodeSummary) =>
    node.kind === 'document' ? formatTime(node.created_at) : `${node.child_count}`

  function pick(index: number) {
    workspace.cursor = index
    void workspace.guard(() => workspace.openSelected())
  }
</script>

<div class="sidebar">
  <div class="head label rule">
    <span class="marker"></span>
    <span class="name">{workspace.space?.slug ?? 'diary'}</span>
    <span class="count">{workspace.nodes.length} {workspace.nodes.length === 1 ? 'item' : 'items'}</span>
  </div>

  {#if workspace.query}
    <div class="filter">
      <span class="muted">/</span><span class="query">{workspace.query}</span>
      <button title="clear search (n)" onclick={() => workspace.guard(() => workspace.search(''))}>esc</button>
    </div>
  {/if}

  <ul bind:this={list}>
    {#each workspace.nodes as node, index (node.id)}
      {@const previous = workspace.nodes[index - 1]}
      <!-- Day headings only make sense in a list that is a journal: documents,
           in date order, not a search and not a folder listing. -->
      {#if !workspace.query && node.kind === 'document' && (!previous || previous.kind !== 'document' || formatDay(previous.created_at) !== formatDay(node.created_at))}
        <li class="daybreak label rule">
          <span><span class="day">{formatDay(node.created_at)}</span> {formatWeekday(node.created_at)}</span>
        </li>
      {/if}
      <li>
        <button
          class="row"
          class:active={index === workspace.cursor}
          class:open={workspace.open?.id === node.id}
          data-index={index}
          onclick={() => pick(index)}
        >
          <span class="dot"></span>
          <span class="title">{label(node)}{#if node.kind !== 'document'}<span class="slash">/</span>{/if}</span>
          {#if workspace.marked?.id === node.id}<span class="badge" title="marked — p drops it in a folder">m</span>{/if}
          {#if node.shared}<span class="shared" title="shared"></span>{/if}
          <span class="aside">{aside(node)}</span>
        </button>
      </li>
    {:else}
      <li class="empty muted">
        {workspace.query ? 'no matches' : 'nothing here yet — o writes, O makes a folder'}
      </li>
    {/each}
  </ul>
</div>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 0;
    --pane: color-mix(in oklab, var(--surface) 55%, var(--base));
    background: var(--pane);
    border-right: 1px solid var(--rim);
  }

  .head {
    flex: none;
    padding: 16px 16px 10px 18px;
    white-space: nowrap;
  }
  .head .marker { background: var(--light); }
  .name { color: var(--light); min-width: 0; overflow: hidden; text-overflow: ellipsis; }
  .count { flex: none; }

  .filter {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0 10px 6px;
    padding: 4px 6px 4px 10px;
    border-radius: var(--radius-s);
    background: var(--surface-alt);
    font-size: 12px;
    flex: none;
  }
  .query { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--light); }

  ul {
    list-style: none;
    margin: 0;
    padding: 2px 8px 24px;
    overflow-y: auto;
    flex: 1;
  }

  .daybreak {
    position: sticky;
    top: 0;
    z-index: 1;
    padding: 14px 8px 6px 10px;
    font-size: 10px;
    background: linear-gradient(var(--pane) 70%, transparent);
  }
  .day { color: var(--subtle); }

  /*
   * A row rests bare, lifts a step on hover, and the cursor inverts it — light
   * fill, ink text — the same mark focus makes everywhere else.
   */
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    margin: 1px 0;
    padding: 5px 10px;
    border: 0;
    border-radius: var(--radius-s);
    text-align: left;
    font-size: 13px;
    letter-spacing: 0;
    text-transform: none;
    color: var(--subtle);
  }
  .row:hover { background: var(--surface-alt); color: var(--light); border-color: transparent; }
  .row:active { transform: none; }
  .row.open { color: var(--light); }
  .row.active, .row:focus-visible { background: var(--light); color: var(--ink); }
  .row.active :is(.aside, .slash, .badge), .row:focus-visible :is(.aside, .slash, .badge) { color: var(--ink); }

  /* The open document carries a dot; the others keep its place. */
  .dot {
    width: 6px;
    height: 6px;
    flex: none;
    border-radius: 999px;
    background: transparent;
    transition: background-color var(--t-base) var(--ease-glide);
  }
  .row.open .dot { background: var(--light); }
  .row.open.active .dot { background: var(--ink); }

  .title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-sans);
    font-size: 14px;
  }
  .slash { color: var(--muted); margin-left: 1px; }
  .aside { flex: none; font-size: 11px; color: var(--muted); font-variant-numeric: tabular-nums; }
  .badge { flex: none; font-size: 11px; font-weight: 700; color: var(--light-dim); }
  .shared {
    width: 7px;
    height: 7px;
    flex: none;
    border: 1.5px solid currentColor;
    border-radius: 999px;
  }
  .empty { padding: 12px 10px; font-size: 12px; }

  /* Roomier tap targets where there is no keyboard. */
  @media (pointer: coarse) {
    .row { padding: 9px 10px; }
  }
</style>
