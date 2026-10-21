<script lang="ts">
  /*
   * `Q`: the scratchpad, a drawer lowered from under the bar over whatever
   * is on screen, the way the desktop lowers its own. The notes on the left,
   * newest first; the one under the cursor on the right, read or written.
   * The keys live in App.svelte with the others; this draws and lets a
   * pointer do the same.
   */
  import { renderMarkdown } from './markdown'
  import { workspace } from './store.svelte'
  import { relative } from './util'

  let list = $state<HTMLUListElement>()

  const note = $derived(workspace.scratchNote)
  const html = $derived(note && !workspace.scratchEditing ? renderMarkdown(note.body) : '')

  $effect(() => {
    const at = workspace.scratchCursor
    list?.querySelector<HTMLElement>(`[data-index="${at}"]`)?.scrollIntoView({ block: 'nearest' })
  })

  /** What a note is called in the list: its first line that says something. */
  function headline(body: string): string {
    return body.split('\n').find((line) => line.trim())?.replace(/^#+\s*/, '').trim() ?? ''
  }

  function pick(index: number) {
    if (workspace.scratchEditing) return
    workspace.scratchCursor = index
  }
</script>

<div class="drawer">
  <header class="label rule">
    <span class="marker"></span><span class="title">scratch</span>
    {workspace.scratchNotes.length} {workspace.scratchNotes.length === 1 ? 'note' : 'notes'}
    <span class="kbd tile-key" title="the key that lowers and raises the scratchpad">Q</span>
  </header>

  <div class="body">
    <ul class="notes" bind:this={list}>
      {#if workspace.scratchEditing && workspace.scratchNew}
        <li class="at new"><span class="line">{headline(workspace.scratchDraft) || 'new note'}</span><span class="when">now</span></li>
      {/if}
      {#each workspace.scratchNotes as entry, index (entry.id)}
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
        <li
          class:at={index === workspace.scratchCursor && !workspace.scratchNew}
          class:dim={workspace.scratchEditing}
          data-index={index}
          onclick={() => pick(index)}
          ondblclick={() => workspace.editScratchNote()}
        >
          <span class="line">{headline(entry.body) || '—'}</span>
          <span class="when">{relative(entry.updated_at)}</span>
        </li>
      {:else}
        {#if !workspace.scratchNew}
          <li class="none muted">nothing here — <span class="kbd">o</span> takes a note</li>
        {/if}
      {/each}
    </ul>

    <div class="page">
      {#if workspace.scratchEditing}
        {#key workspace.scratchSession}
          {#await import('./TextEditor.svelte')}
            <div class="muted label">loading editor</div>
          {:then { default: TextEditor }}
            <TextEditor
              doc={workspace.scratchDraft}
              placeholder="anything — it saves itself; :keep makes it a document"
              onchange={(text) => workspace.scratchChanged(text)}
              onsave={() => workspace.guard(() => workspace.saveScratchNote())}
              onleave={() => workspace.leaveScratchNote()}
            />
          {/await}
        {/key}
      {:else if html}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <article class="md" ondblclick={() => workspace.editScratchNote()}>
          <!-- eslint-disable-next-line svelte/no-at-html-tags -- sanitised in renderMarkdown -->
          {@html html}
        </article>
      {:else}
        <p class="muted hint">a note is a few lines, kept here until it is clear where they belong.</p>
      {/if}
    </div>
  </div>

  <footer class="label">
    {#if workspace.scratchEditing}
      <span>writing — it saves itself</span>
      <span><span class="kbd">esc</span> back to the notes</span>
      <span><span class="kbd">:keep</span> make it a document</span>
    {:else}
      <span><span class="kbd">j</span><span class="kbd">k</span> note</span>
      <span><span class="kbd">o</span> new</span>
      <span><span class="kbd">↵</span> write</span>
      <span><span class="kbd">K</span> keep as document</span>
      <span><span class="kbd">dd</span> drop</span>
      <span><span class="kbd">Q</span><span class="kbd">q</span> raise</span>
    {/if}
  </footer>
</div>

<style>
  /*
   * Hung from under the bar across the frame's opening, as the desktop's
   * scratchpad slides down from under its bar: hull, a rim, the lower corners
   * rounded, and the light along its lower edge to say it has the keys.
   */
  .drawer {
    position: absolute;
    top: calc(var(--bar-h) + var(--frame));
    left: calc(var(--frame) + 24px);
    right: calc(var(--rail-w) + 24px);
    z-index: 25;
    height: min(62vh, 640px);
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px 20px 12px;
    background: var(--hull);
    border: 1px solid var(--surface-alt);
    border-top: 0;
    border-bottom-color: var(--light-dim);
    border-radius: 0 0 var(--frame-radius) var(--frame-radius);
    box-shadow: 0 18px 40px color-mix(in oklab, var(--ink) 70%, transparent);
    animation: lower var(--t-slow) var(--ease-snap) both;
  }
  @keyframes lower {
    from { transform: translateY(-104%); }
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
  .page :global(.writing) { height: 100%; }
  .page :global(.writing .editor) { flex: 1; max-height: none; }
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
