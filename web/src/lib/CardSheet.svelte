<script lang="ts">
  /*
   * Enter on a card turns it over: a window floated over the board, as the
   * desktop floats one, with what the card holds on its back — labels, the
   * description, the checklist. The keys live in App.svelte with the others;
   * this draws and lets a pointer do the same.
   */
  import { renderMarkdown } from './markdown'
  import { workspace } from './store.svelte'
  import { formatDay, labelHue, urgency } from './util'

  let steps = $state<HTMLUListElement>()

  const card = $derived(workspace.card)
  const list = $derived(workspace.list)
  const lists = $derived(workspace.board?.lists.length ?? 0)
  const role = $derived(lists < 2 ? '' : workspace.listCursor === 0 ? 'inbox' : workspace.listCursor === lists - 1 ? 'done' : '')
  const ticked = $derived(card?.checklist.filter((task) => task.done).length ?? 0)
  const html = $derived(card?.body.trim() ? renderMarkdown(card.body) : '')

  // A card deleted or moved out of sight under the sheet takes the sheet with it.
  $effect(() => {
    if (!card) workspace.overlay = 'none'
  })

  $effect(() => {
    const at = workspace.taskCursor
    steps?.querySelector<HTMLElement>(`[data-index="${at}"]`)?.scrollIntoView({ block: 'nearest' })
  })

  function pick(index: number) {
    workspace.taskCursor = index
  }

  function tick(index: number) {
    workspace.taskCursor = index
    void workspace.guard(() => workspace.toggleTask())
  }
</script>

{#if card}
  <div class="sheet">
    <header>
      <span class="label rule">
        <span class="marker"></span><span class="title">card</span>
        {workspace.boardSpace?.slug ?? ''} / {list?.name ?? ''}{#if role}<span class="role">{role}</span>{/if}
      </span>
      <button onclick={() => workspace.closeCardSheet()}>esc</button>
    </header>

    <div class="scroll">
      <h2 class:done={card.done_at !== null}>{card.title}</h2>

      {#if card.labels.length || card.due_at !== null || card.repeat !== null || card.node_id !== null}
        <div class="meta">
          {#each card.labels as label (label)}<span class="chip {labelHue(label)}">{label}</span>{/each}
          {#if card.due_at !== null}<span class={urgency(card)}>due {formatDay(card.due_at)}</span>{/if}
          {#if card.repeat !== null}<span>↻ {card.repeat}</span>{/if}
          {#if card.node_id !== null}
            <button class="doc" onclick={() => workspace.guard(() => workspace.openCard())}>→ {card.node_name ?? `#${card.node_id}`}</button>
          {/if}
        </div>
      {/if}

      <section>
        <div class="label head"><span class="marker"></span>description</div>
        {#if workspace.describing}
          {#await import('./CardEditor.svelte')}
            <div class="empty label">loading editor</div>
          {:then module}
            <module.default />
          {/await}
        {:else if html}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
          <article class="md description" ondblclick={() => workspace.describe()}>
            <!-- eslint-disable-next-line svelte/no-at-html-tags -- sanitised in renderMarkdown -->
            {@html html}
          </article>
        {:else}
          <button class="empty" onclick={() => workspace.describe()}>nothing written yet — <span class="kbd">e</span> writes it</button>
        {/if}
      </section>

      <section>
        <div class="label head">
          <span class="marker"></span>checklist
          {#if card.checklist.length}
            <span class="count" class:complete={ticked === card.checklist.length}>{ticked}/{card.checklist.length}</span>
          {/if}
        </div>
        {#if card.checklist.length}
          <div class="progress"><span style="width: {(ticked / card.checklist.length) * 100}%"></span></div>
        {/if}
        <ul bind:this={steps}>
          {#each card.checklist as task, index (index)}
            <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
            <li class:at={index === workspace.taskCursor && !workspace.describing} class:ticked={task.done} data-index={index} onclick={() => pick(index)}>
              <button class="tick" aria-label={task.done ? 'untick' : 'tick'} onclick={(e) => (e.stopPropagation(), tick(index))}></button>
              <span class="text">{task.text}</span>
            </li>
          {:else}
            <li class="none muted">no steps — <span class="kbd">o</span> adds one</li>
          {/each}
        </ul>
      </section>
    </div>

    <footer class="label">
      <span><span class="kbd">j</span><span class="kbd">k</span> step</span>
      <span><span class="kbd">␣</span> tick</span>
      <span><span class="kbd">o</span> add</span>
      <span><span class="kbd">r</span> reword</span>
      <span><span class="kbd">J</span><span class="kbd">K</span> order</span>
      <span><span class="kbd">dd</span> drop</span>
      <span><span class="kbd">e</span> describe</span>
      <span><span class="kbd">a</span> label</span>
      <span><span class="kbd">D</span> due</span>
      <span><span class="kbd">H</span><span class="kbd">L</span> send</span>
      <span><span class="kbd">gf</span> notes</span>
      <span><span class="kbd">q</span> close</span>
    </footer>
  </div>
{/if}

<style>
  /* A float, as Hyprland draws one over the tiles: hull, one rim, the frame's radius. */
  .sheet {
    display: flex;
    flex-direction: column;
    gap: 14px;
    max-height: 100%;
    width: min(100%, 76ch);
    background: color-mix(in oklab, var(--hull) 97%, transparent);
    border: 1px solid var(--surface-alt);
    border-radius: var(--frame-radius);
    padding: 20px 24px;
    animation: resolve var(--t-slow) var(--ease-snap) both;
    overflow: hidden;
  }

  header { display: flex; align-items: center; gap: 12px; }
  header .label { flex: 1; }
  header .marker { background: var(--light); }
  header .title { color: var(--light); }
  .role { margin-left: 8px; color: var(--muted); font-size: 10px; letter-spacing: 0.18em; }

  .scroll { display: flex; flex-direction: column; gap: 18px; min-height: 0; overflow-y: auto; }

  h2 { margin: 0; font-family: var(--font-sans); font-size: 20px; font-weight: 600; color: var(--light); }
  h2.done { text-decoration: line-through; color: var(--muted); }

  .meta { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 12px; font-size: 11px; color: var(--muted); }
  .meta .today { color: var(--light); }
  .meta .overdue { color: var(--alarm); }
  .meta .doc { padding: 0 6px; border: 0; font-size: 11px; text-transform: none; letter-spacing: 0; color: var(--slate-br); }

  section { display: flex; flex-direction: column; gap: 8px; }
  .head { display: flex; align-items: center; gap: 10px; }
  .count { font-variant-numeric: tabular-nums; }
  .count.complete { color: var(--moss); }

  .description { padding: 4px 2px; font-size: 14px; }
  .description :global(> :last-child) { margin-bottom: 0; }
  .empty { align-self: flex-start; border: 0; padding: 4px 2px; font-size: 12px; text-transform: none; letter-spacing: 0; color: var(--muted); }

  .progress { height: 2px; border-radius: 1px; background: var(--surface-alt); overflow: hidden; }
  .progress span { display: block; height: 100%; background: var(--moss); transition: width var(--t-base) var(--ease-glide); }

  ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 1px; }
  li {
    display: flex;
    align-items: baseline;
    gap: 10px;
    padding: 5px 8px;
    border-radius: var(--radius-s);
    font-family: var(--font-sans);
    font-size: 14px;
    cursor: default;
  }
  li:hover { background: var(--surface); }
  li.at { background: var(--light); color: var(--ink); }
  li.ticked .text { text-decoration: line-through; color: var(--muted); }
  li.at.ticked .text { color: var(--ink); }
  li.none { font-family: var(--font-mono); font-size: 12px; }

  .tick {
    width: 11px;
    height: 11px;
    flex: none;
    padding: 0;
    border: 1px solid var(--outline);
    border-radius: 2px;
    transform: translateY(1px);
  }
  li.ticked .tick { background: var(--moss); border-color: var(--moss); }
  li.at .tick { border-color: var(--ink); }
  li.at.ticked .tick { background: var(--ink); }

  footer { display: flex; flex-wrap: wrap; gap: 6px 14px; font-size: 10px; }
  footer > span { display: flex; gap: 3px; align-items: center; }
  @media (max-width: 767px), (pointer: coarse) {
    footer { display: none; }
  }
</style>
