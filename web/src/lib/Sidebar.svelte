<script lang="ts">
  import { isContainer, type NodeSummary } from './api'
  import { carry, into, release } from './carry'
  import Icon from './Icon.svelte'
  import { workspace } from './store.svelte'
  import { formatBytes, formatDay, formatTime, formatWeekday, headline, hitParts, isTouchDevice, relative } from './util'

  let list = $state<HTMLUListElement>()
  /** Files are being dragged over the list, which takes them into this folder. */
  let dropping = $state(false)

  // Keep the cursor line visible while navigating with j/k.
  $effect(() => {
    const index = workspace.tree.cursor
    const node = list?.querySelector<HTMLElement>(`[data-index="${index}"]`)
    node?.scrollIntoView({ block: 'nearest' })
  })

  function label(node: NodeSummary) {
    return node.name.trim() || node.excerpt.trim() || node.slug
  }

  /*
   * A folder is listed by what is in it, a document by when it was written —
   * and how many files it has, folded or not — a file by its size.
   */
  const aside = (node: NodeSummary) =>
    node.file
      ? formatBytes(node.file.size)
      : node.kind === 'document'
        ? `${node.child_count > 0 ? `+${node.child_count} · ` : ''}${formatTime(node.created_at)}`
        : `${node.child_count}`

  /* The row a day heading compares with: the one above, past a document's files. */
  const above = (index: number) => workspace.tree.nodes.slice(0, index).filter((n) => !workspace.tree.isNested(n)).at(-1)

  const carriesFiles = (event: DragEvent) => event.dataTransfer?.types.includes('Files') ?? false

  function drop(event: DragEvent) {
    dropping = false
    const files = Array.from(event.dataTransfer?.files ?? [])
    if (files.length === 0 || workspace.tree.query) return
    event.preventDefault()
    void workspace.guard(() => workspace.tree.putFiles(files))
  }

  /* Ctrl or Cmd picks a row up, Shift every row from the cursor to it. */
  function click(event: MouseEvent, index: number, node: NodeSummary) {
    if (event.ctrlKey || event.metaKey) return workspace.tree.mark(node, false)
    if (event.shiftKey) return workspace.tree.markRange(index)
    pick(index)
  }

  function pick(index: number) {
    workspace.tree.cursor = index
    void workspace.guard(() => workspace.tree.openSelected())
  }

  /* A tap opens a row, so on a phone the other verbs need a door of their own. */
  function actions(index: number) {
    workspace.tree.cursor = index
    workspace.sheets.open('actions')
  }

  /* What is in the folder itself, not the files folded open under its documents. */
  const items = $derived(workspace.tree.nodes.filter((node) => !workspace.tree.isNested(node)).length)

  /* Inside a folder, a phone has no h key and no crumbs on the bar: this is the way up. */
  const nested = $derived(isTouchDevice && !workspace.tree.query && workspace.tree.path.length > 1)
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="sidebar"
  class:dropping
  ondragover={(event) => {
    if (!carriesFiles(event) || workspace.tree.query) return
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
    <span class="name">{workspace.tree.space?.slug ?? 'diary'}</span>
    <span class="count">
      {items} {items === 1 ? 'item' : 'items'}{#if workspace.tree.foundCards.length > 0}
        · {workspace.tree.foundCards.length} {workspace.tree.foundCards.length === 1 ? 'card' : 'cards'}{/if}{#if workspace.tree.foundNotes.length > 0}
        · {workspace.tree.foundNotes.length} {workspace.tree.foundNotes.length === 1 ? 'note' : 'notes'}{/if}
    </span>
  </div>

  {#if workspace.tree.query}
    <div class="filter">
      <span class="muted">/</span><span class="query">{workspace.tree.query}</span>
      {#if workspace.tree.globalSearch}<span class="muted scope">all spaces</span>{/if}
      <button title="clear search (n)" onclick={() => workspace.guard(() => workspace.tree.search(''))}>esc</button>
    </div>
  {/if}

  <ul bind:this={list}>
    {#if nested}
      <li>
        <button class="row up" onclick={() => workspace.guard(() => workspace.tree.up())}>
          <span class="dot"></span>
          <span class="title">..<span class="slash">/</span></span>
          <span class="aside">up</span>
        </button>
      </li>
    {/if}
    {#each workspace.tree.nodes as node, index (node.id)}
      {@const previous = above(index)}
      {@const nested = workspace.tree.isNested(node)}
      <!-- Day headings only make sense in a list that is a journal: documents,
           in date order, not a search and not a folder listing. -->
      {#if !workspace.tree.query && !nested && node.kind === 'document' && (!previous || previous.kind !== 'document' || formatDay(previous.created_at) !== formatDay(node.created_at))}
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
          class:active={index === workspace.tree.cursor}
          class:open={workspace.tiles.all.some((tile) => tile.open?.id === node.id)}
          class:marked={workspace.tree.isMarked(node.id)}
          class:nested
          data-index={index}
          onclick={(event) => click(event, index, node)}
        >
          <span class="dot"></span>
          <span class="title">
            {label(node)}{#if isContainer(node)}<span class="slash">/</span>{/if}
            {#if workspace.tree.query && node.location}<span class="where">{node.location}</span>{/if}
            {#if workspace.tree.query && node.in_text}<span class="hit" title="found in the file's text">{#each hitParts(node.excerpt) as part, at (at)}{#if part.lit}<mark>{part.text}</mark>{:else}{part.text}{/if}{/each}</span>{/if}
          </span>
          {#if workspace.tree.isMarked(node.id)}<span class="badge" title="marked — p drops what is marked in a folder">m</span>{/if}
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
    {/each}
    <!-- The cards a search found, after the nodes: Enter turns one over on its board. -->
    {#if workspace.tree.foundCards.length > 0}
      <li class="daybreak label rule"><span>cards</span></li>
    {/if}
    {#each workspace.tree.foundCards as card, at (card.id)}
      {@const index = workspace.tree.nodes.length + at}
      <li class="item">
        <button class="row card" class:active={index === workspace.tree.cursor} class:done={card.done_at !== null} data-index={index} onclick={() => pick(index)}>
          <span class="dot"></span>
          <span class="title">{card.title}<span class="where">{card.space_slug}/{card.list_name}</span></span>
          <span class="aside">{card.done_at !== null ? 'done' : card.due_at !== null ? formatDay(card.due_at) : 'card'}</span>
        </button>
      </li>
    {/each}
    <!-- Then the notes it found on the scratchpad: Enter lowers it, at the note. -->
    {#if workspace.tree.foundNotes.length > 0}
      <li class="daybreak label rule"><span>notes</span></li>
    {/if}
    {#each workspace.tree.foundNotes as note, at (note.id)}
      {@const index = workspace.tree.nodes.length + workspace.tree.foundCards.length + at}
      <li class="item">
        <button class="row note" class:active={index === workspace.tree.cursor} data-index={index} onclick={() => pick(index)}>
          <span class="dot"></span>
          <span class="title">{headline(note.body) || '—'}<span class="where">scratch</span></span>
          <span class="aside">{relative(note.updated_at)}</span>
        </button>
      </li>
    {/each}
    {#if workspace.tree.rows === 0}
      <li class="empty muted">
        {#if workspace.tree.query}
          no matches
        {:else}
          nothing here yet — <span class="kbd">o</span> writes, <span class="kbd">⇧ O</span> makes a folder, files dropped here land here
        {/if}
      </li>
    {/if}
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
  /* A card is told from a node by its square. */
  .row.card .dot { border: 1px solid currentColor; border-radius: 1px; }
  .row.note .dot { border: 1px dashed currentColor; background: none; }
  .row.card.done .title { color: var(--muted); text-decoration: line-through; }

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
  .hit mark { background: none; color: var(--light); }
  .row.active .hit mark { color: var(--ink); font-weight: 700; }
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
  /* A document's own files, folded open under it. */
  .row.nested { padding-left: 28px; }
  .row.nested .title { font-size: 13px; }

  /* Roomier tap targets where there is no keyboard. */
  @media (pointer: coarse) {
    .row { padding: 9px 10px; }
  }
</style>
