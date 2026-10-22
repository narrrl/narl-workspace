<script lang="ts">
  /*
   * Enter on a card turns it over: a tile beside the board with what the
   * card holds on its back — labels, the description, the checklist. It is
   * the card under the board's cursor, so walking the board leafs through
   * the backs. The keys live in actions.ts with the others; this draws and
   * lets a pointer do the same.
   */
  import Keys from './Keys.svelte'
  import { followLink } from './commands'
  import { renderMarkdown } from './markdown'
  import { workspace } from './store.svelte'
  import { formatDay, labelHue, urgency } from './util'

  const card = $derived(workspace.cards.card)
  const list = $derived(workspace.cards.list)
  const lists = $derived(workspace.cards.board?.lists.length ?? 0)
  const role = $derived(lists < 2 ? '' : workspace.cards.listCursor === 0 ? 'inbox' : workspace.cards.listCursor === lists - 1 ? 'done' : '')
  const ticked = $derived(card?.checklist.filter((task) => task.done).length ?? 0)
  const html = $derived(card?.body.trim() ? renderMarkdown(card.body, { wikilinks: card.wikilinks }) : '')

  let root = $state<HTMLDivElement>()

  // Another card's back starts at its first step.
  let shown: number | undefined
  $effect(() => {
    if (card?.id === shown) return
    shown = card?.id
    workspace.cards.taskCursor = 0
  })

  // Keep the step under the cursor on screen.
  $effect(() => {
    const index = workspace.cards.taskCursor
    root?.querySelector<HTMLElement>(`[data-index="${index}"]`)?.scrollIntoView({ block: 'nearest' })
  })

  /** A link in the description leaves the card for what it leads to, as one in a document does. */
  function follow(event: MouseEvent) {
    if (event.metaKey || event.ctrlKey || event.shiftKey || event.button !== 0) return
    const href = (event.target as HTMLElement | null)?.closest('a')?.getAttribute('href') ?? ''
    if (!href.startsWith('/n/')) return
    event.preventDefault()
    followLink(href)
  }

  function pick(index: number) {
    workspace.cards.taskCursor = index
  }

  function tick(index: number) {
    workspace.cards.taskCursor = index
    void workspace.guard(() => workspace.cards.toggleTask())
  }
</script>

<div class="back" bind:this={root}>
  <header>
    <span class="label rule">
      <span class="marker"></span><span class="title">card</span>
      {#if card}{workspace.cards.boardSpace?.slug ?? ''} / {list?.name ?? ''}{role ? ` · ${role}` : ''}{/if}
    </span>
    <span class="kbd tile-key" title="the key that turns a card over, on the board">↵</span>
  </header>

  {#if card}
    <div class="scroll">
      <h2 class:done={card.done_at !== null}>{card.title}</h2>

      {#if card.labels.length || card.due_at !== null || card.repeat !== null || card.node_id !== null}
        <div class="meta">
          {#each card.labels as label (label)}<span class="chip {labelHue(label)}">{label}</span>{/each}
          {#if card.due_at !== null}<span class={urgency(card)}>due {formatDay(card.due_at)}</span>{/if}
          {#if card.repeat !== null}<span>↻ {card.repeat}</span>{/if}
          {#if card.node_id !== null}
            <button class="doc" onclick={() => workspace.guard(() => workspace.cards.openCard())}>→ {card.node_name ?? `#${card.node_id}`}</button>
          {/if}
        </div>
      {/if}

      <section>
        <div class="label head"><span class="marker"></span>description</div>
        {#if workspace.cards.describing && workspace.drafts.writing}
          {#await import('./Editor.svelte')}
            <div class="empty label">loading editor</div>
          {:then { default: Editor }}
            {#key workspace.drafts.writing.session}
              <Editor writing={workspace.drafts.writing} hint="what this card is about, in short — [[ links a document" />
            {/key}
          {/await}
        {:else if html}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions, a11y_no_noninteractive_element_interactions -->
          <article class="md description" onclick={follow} ondblclick={() => workspace.cards.describe()}>
            <!-- eslint-disable-next-line svelte/no-at-html-tags -- sanitised in renderMarkdown -->
            {@html html}
          </article>
        {:else}
          <button class="empty" onclick={() => workspace.cards.describe()}>nothing written yet — <span class="kbd">e</span> writes it</button>
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
        <ul>
          {#each card.checklist as task, index (index)}
            <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
            <li class:at={index === workspace.cards.taskCursor && !workspace.cards.describing} class:ticked={task.done} data-index={index} onclick={() => pick(index)}>
              <button class="tick" aria-label={task.done ? 'untick' : 'tick'} onclick={(e) => (e.stopPropagation(), tick(index))}></button>
              <span class="text">{task.text}</span>
            </li>
          {:else}
            <li class="none muted">no steps — <span class="kbd">o</span> adds one</li>
          {/each}
        </ul>
      </section>
    </div>
  {:else}
    <p class="none muted">no card under the board's cursor — o on the board adds one</p>
  {/if}

  <Keys contexts={['card', 'views']} />
</div>

<style>
  .back {
    display: flex;
    flex-direction: column;
    gap: 18px;
    height: 100%;
    min-height: 0;
    padding: 20px 24px;
  }

  header { display: flex; align-items: center; gap: 12px; min-width: 0; }
  header .label { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  header .marker { background: var(--light); }
  header .title { color: var(--light); }
  .scroll { flex: 1; display: flex; flex-direction: column; gap: 18px; min-height: 0; overflow-y: auto; }

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

  @media (max-width: 767px) {
    .back { padding: 14px; gap: 14px; }
  }
</style>
