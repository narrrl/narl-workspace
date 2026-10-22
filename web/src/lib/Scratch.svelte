<script lang="ts">
  /*
   * `Q`: the scratchpad, a drawer lowered from under the bar over whatever
   * is on screen, the way the desktop lowers its own. The notes on the left,
   * newest first; the one under the cursor on the right, read or written.
   * The keys live in App.svelte with the others; this draws and lets a
   * pointer do the same.
   */
  import Keys from './Keys.svelte'
  import { renderMarkdown } from './markdown'
  import { workspace } from './store.svelte'
  import { headline, relative } from './util'

  let list = $state<HTMLUListElement>()

  const note = $derived(workspace.scratch.note)
  const html = $derived(note && !workspace.scratch.editing ? renderMarkdown(note.body) : '')

  $effect(() => {
    const at = workspace.scratch.cursor
    list?.querySelector<HTMLElement>(`[data-index="${at}"]`)?.scrollIntoView({ block: 'nearest' })
  })

  function pick(index: number) {
    if (workspace.scratch.editing) return
    workspace.scratch.cursor = index
  }
</script>

<div class="drawer">
  <header class="label rule">
    <span class="marker"></span><span class="title">scratch</span>
    {workspace.scratch.notes.length} {workspace.scratch.notes.length === 1 ? 'note' : 'notes'}
    <span class="kbd tile-key" title="the key that lowers and raises the scratchpad">Q</span>
  </header>

  <div class="body">
    <ul class="notes" bind:this={list}>
      {#if workspace.scratch.editing && workspace.scratch.writingNew}
        <li class="at new"><span class="line">{headline(workspace.drafts.writing?.text ?? '') || 'new note'}</span><span class="when">now</span></li>
      {/if}
      {#each workspace.scratch.notes as entry, index (entry.id)}
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
        <li
          class:at={index === workspace.scratch.cursor && !workspace.scratch.writingNew}
          class:dim={workspace.scratch.editing}
          data-index={index}
          onclick={() => pick(index)}
          ondblclick={() => workspace.scratch.editNote()}
        >
          <span class="line">{headline(entry.body) || '—'}</span>
          <span class="when">{relative(entry.updated_at)}</span>
        </li>
      {:else}
        {#if !workspace.scratch.writingNew}
          <li class="none muted">nothing here — <span class="kbd">o</span> takes a note</li>
        {/if}
      {/each}
    </ul>

    <div class="page">
      {#if workspace.scratch.editing && workspace.drafts.writing}
        {#await import('./Editor.svelte')}
          <div class="muted label">loading editor</div>
        {:then { default: Editor }}
          {#key workspace.drafts.writing.session}
            <Editor writing={workspace.drafts.writing} fill hint="anything — it saves itself; :keep makes it a document" />
          {/key}
        {/await}
      {:else if html}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <article class="md" ondblclick={() => workspace.scratch.editNote()}>
          <!-- eslint-disable-next-line svelte/no-at-html-tags -- sanitised in renderMarkdown -->
          {@html html}
        </article>
      {:else}
        <p class="muted hint">a note is a few lines, kept here until it is clear where they belong.</p>
      {/if}
    </div>
  </div>

  {#if workspace.scratch.editing}
    <footer class="label">
      <span>writing — it saves itself</span>
      <span><span class="kbd">esc</span> back to the notes</span>
      <span><span class="kbd">⌃ Q</span> back to where you were</span>
      <span><span class="kbd">:keep</span> make it a document</span>
    </footer>
  {:else}
    <Keys contexts={['drawer']} />
  {/if}
</div>

<style>
  /*
   * The upper part of the porthole, come down over what is in it: the same
   * opening under the bar, the same rounded corners, and the focused
   * window's lit rim, since it has the keys while it is down. It is
   * uncovered from the top, as the desktop's scratchpad comes from under its
   * bar.
   */
  .drawer {
    --pane-bg: color-mix(in oklab, var(--surface) 40%, var(--hull));
    position: absolute;
    top: var(--bar-h);
    left: var(--frame);
    right: var(--rail-w);
    z-index: 25;
    height: min(62vh, 640px, calc(100% - var(--bar-h) - var(--status-h)));
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px 20px 12px;
    border: 1px solid transparent;
    border-radius: var(--frame-radius);
    background:
      linear-gradient(var(--pane-bg), var(--pane-bg)) padding-box,
      linear-gradient(135deg, var(--light), var(--light-dim)) border-box;
    box-shadow: 0 18px 40px color-mix(in oklab, var(--ink) 70%, transparent);
    animation: lower var(--t-slow) var(--ease-snap) both;
  }
  @keyframes lower {
    from { clip-path: inset(0 0 100% 0 round var(--frame-radius)); }
    to { clip-path: inset(0 0 0 0 round var(--frame-radius)); }
  }

  header { display: flex; align-items: center; gap: 10px; }
  header .marker { background: var(--light); }
  header .title { color: var(--light); }
  header .tile-key { order: 1; margin-left: 12px; background: var(--light); border-color: var(--light); color: var(--ink); }

  .body { flex: 1; min-height: 0; display: grid; grid-template-columns: minmax(22ch, 32ch) 1fr; gap: 16px; }

  .notes { list-style: none; margin: 0; padding: 0; overflow-y: auto; display: flex; flex-direction: column; gap: 1px; }
  .notes li {
    display: flex;
    align-items: baseline;
    gap: 10px;
    padding: 6px 10px;
    border-radius: var(--radius-s);
    cursor: default;
  }
  .notes li:hover { background: var(--surface); }
  .notes li.at { background: var(--light); color: var(--ink); }
  .notes li.at.dim { background: var(--surface-alt); color: var(--light); }
  .notes li.new { font-style: italic; }
  .line { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-family: var(--font-sans); font-size: 14px; }
  .when { flex: none; font-size: 10px; color: var(--muted); }
  .notes li.at:not(.dim) .when { color: var(--ink); }
  .none { font-size: 12px; }

  .page { min-height: 0; overflow-y: auto; border-left: 1px solid var(--rim); padding-left: 16px; }
  .page .md { font-size: 14px; }
  .hint { font-size: 12px; margin: 6px 0; }

  footer { display: flex; flex-wrap: wrap; gap: 6px 14px; font-size: 10px; }
  footer > span { display: flex; gap: 3px; align-items: center; }

  @media (max-width: 767px) {
    .drawer { left: var(--frame); right: var(--frame); height: 70vh; }
    .body { grid-template-columns: 1fr; grid-template-rows: auto 1fr; }
    .notes { max-height: 30%; }
    .page { border-left: 0; padding-left: 0; }
    footer { display: none; }
  }
</style>
