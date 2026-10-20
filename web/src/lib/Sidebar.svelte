<script lang="ts">
  import { isContainer, type NodeSummary } from './api'
  import { carry, into, release } from './carry'
  import Icon from './Icon.svelte'
  import { workspace } from './store.svelte'
  import { formatBytes, formatDay, formatTime, formatWeekday, isTouchDevice } from './util'

  let list = $state<HTMLUListElement>()
  /** Files are being dragged over the list, which takes them into this folder. */
  let dropping = $state(false)

  // Keep the cursor line visible while navigating with j/k.
  $effect(() => {
    const index = workspace.cursor
    const node = list?.querySelector<HTMLElement>(`[data-index="${index}"]`)
    node?.scrollIntoView({ block: 'nearest' })
  })

  function label(node: NodeSummary) {
    return node.name.trim() || node.excerpt.trim() || node.slug
  }

  /* A folder is listed by what is in it, a document by when it was written, a file by its size. */
  const aside = (node: NodeSummary) =>
    node.file
      ? formatBytes(node.file.size)
      : node.kind === 'document'
        ? formatTime(node.created_at)
        : `${node.child_count}`

  const carriesFiles = (event: DragEvent) => event.dataTransfer?.types.includes('Files') ?? false

  function drop(event: DragEvent) {
    dropping = false
    const files = Array.from(event.dataTransfer?.files ?? [])
    if (files.length === 0 || workspace.query) return
    event.preventDefault()
    void workspace.guard(() => workspace.putFiles(files))
  }

  /* Ctrl or Cmd picks a row up, Shift every row from the cursor to it. */
  function click(event: MouseEvent, index: number, node: NodeSummary) {
    if (event.ctrlKey || event.metaKey) return workspace.mark(node, false)
    if (event.shiftKey) return workspace.markRange(index)
    pick(index)
  }

  function pick(index: number) {
    workspace.cursor = index
    void workspace.guard(() => workspace.openSelected())
  }

  /* A tap opens a row, so on a phone the other verbs need a door of their own. */
  function actions(index: number) {
    workspace.cursor = index
    workspace.overlay = 'actions'
  }

  /* Inside a folder, a phone has no h key and no crumbs on the bar: this is the way up. */
  const nested = $derived(isTouchDevice && !workspace.query && workspace.path.length > 1)
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="sidebar"
  class:dropping
  ondragover={(event) => {
    if (!carriesFiles(event) || workspace.query) return
    event.preventDefault()
    dropping = true
  }}
  ondragleave={(event) => {
    if (!(event.currentTarget as HTMLElement).contains(event.relatedTarget as Node | null)) dropping = false
  }}
  ondrop={drop}
>
  <div class="head label rule">
    <span class="marker"></span>
    <span class="name">{workspace.space?.slug ?? 'diary'}</span>
    <span class="count">{workspace.nodes.length} {workspace.nodes.length === 1 ? 'item' : 'items'}</span>
  </div>

  {#if workspace.query}
    <div class="filter">
      <span class="muted">/</span><span class="query">{workspace.query}</span>
      {#if workspace.globalSearch}<span class="muted scope">all spaces</span>{/if}
      <button title="clear search (n)" onclick={() => workspace.guard(() => workspace.search(''))}>esc</button>
    </div>
  {/if}

  <ul bind:this={list}>
    {#if nested}
      <li>
        <button class="row up" onclick={() => workspace.guard(() => workspace.up())}>
          <span class="dot"></span>
          <span class="title">..<span class="slash">/</span></span>
          <span class="aside">up</span>
        </button>
      </li>
    {/if}
    {#each workspace.nodes as node, index (node.id)}
      {@const previous = workspace.nodes[index - 1]}
      <!-- Day headings only make sense in a list that is a journal: documents,
           in date order, not a search and not a folder listing. -->
      {#if !workspace.query && node.kind === 'document' && (!previous || previous.kind !== 'document' || formatDay(previous.created_at) !== formatDay(node.created_at))}
        <li class="daybreak label rule">
          <span><span class="day">{formatDay(node.created_at)}</span> {formatWeekday(node.created_at)}</span>
        </li>
      {/if}
      <li
        class="item"
        draggable={!isTouchDevice}
        ondragstart={(event) => carry(event, node)}
        ondragend={release}
        {@attach into(isContainer(node) ? node.id : null, `into ${label(node)}`)}
      >
        <button
          class="row"
          oncontextmenu={(event) => {
            event.preventDefault()
            actions(index)
          }}
          class:active={index === workspace.cursor}
          class:open={workspace.tiles.some((tile) => tile.open?.id === node.id)}
          class:marked={workspace.isMarked(node.id)}
          data-index={index}
          onclick={(event) => click(event, index, node)}
        >
          <span class="dot"></span>
          <span class="title">
            {label(node)}{#if isContainer(node)}<span class="slash">/</span>{/if}
            {#if workspace.query && node.location}<span class="where">{node.location}</span>{/if}
            {#if workspace.query && node.in_text}<span class="hit" title="found in the file's text">{node.excerpt}</span>{/if}
          </span>
          {#if workspace.isMarked(node.id)}<span class="badge" title="marked — p drops what is marked in a folder">m</span>{/if}
          {#if node.shared}<span class="shared" title="shared"></span>{/if}
          {#if node.file?.encrypted}<span class="sealed" title="in the vault"><Icon name="lock" size={12} /></span>{/if}
          {#if node.sealed}<span class="sealed" title="sealed — files added here go into the vault"><Icon name="lock" size={12} /></span>{/if}
          <span class="aside">{aside(node)}</span>
        </button>
        {#if isTouchDevice}
          <button class="more" aria-label="actions for {label(node)}" onclick={() => actions(index)}>
            <Icon name="more" size={18} />
          </button>
        {/if}
      </li>
    {:else}
      <li class="empty muted">
        {#if workspace.query}
          no matches
        {:else}
          nothing here yet — <span class="kbd">o</span> writes, <span class="kbd">⇧ O</span> makes a folder, files dropped here land here
        {/if}
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
  }

  /* Files held over the list: the whole pane is the target. */
  .sidebar.dropping { outline: 1px dashed var(--light-dim); outline-offset: -6px; }

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
  .scope { flex: none; font-size: 10px; letter-spacing: 0.18em; text-transform: uppercase; }
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
    background: linear-gradient(var(--pane-bg) 70%, transparent);
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
  /* Picked up: a rule down the left edge, which the cursor's fill still shows. */
  .row.marked { box-shadow: inset 2px 0 0 var(--light-dim); color: var(--light); }
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
  /* Where a search match sits, so two of the same name can be told apart. */
  .where { margin-left: 8px; font-family: var(--font-mono); font-size: 11px; color: var(--muted); }
  .row.active .where { color: var(--ink); }
  /* A file found by its text, not its name: the words it was found by, below. */
  .hit {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--muted);
  }
  .row.active .hit { color: var(--ink); }
  .aside { flex: none; font-size: 11px; color: var(--muted); font-variant-numeric: tabular-nums; }
  .badge { flex: none; font-size: 11px; font-weight: 700; color: var(--light-dim); }
  .shared {
    width: 7px;
    height: 7px;
    flex: none;
    border: 1.5px solid currentColor;
    border-radius: 999px;
  }
  .sealed { flex: none; display: grid; color: var(--muted); }
  .row.active .sealed { color: var(--ink); }
  .empty { padding: 12px 10px; font-size: 12px; }
  .item { display: flex; align-items: center; }
  .more { flex: none; display: grid; place-items: center; width: 40px; height: 40px; padding: 0; border: 0; background: none; color: var(--muted); }
  .up .title { font-family: var(--font-mono); }

  /* Roomier tap targets where there is no keyboard. */
  @media (pointer: coarse) {
    .row { padding: 9px 10px; }
  }
</style>
