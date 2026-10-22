<script lang="ts" module>
  import type { RevisionSummary } from './api'
  import { workspace } from './store.svelte'

  /** What Enter and the button do: the shown revision becomes the document again. */
  export async function restoreShown() {
    const revision = workspace.revision
    if (!revision) return
    if (!(await workspace.ask(`bring back the version from ${formatStamp(revision.saved_at)}? the text now is kept in :history`))) return
    await workspace.guard(() => workspace.restoreRevision(revision))
  }

  export function moveHistory(by: number) {
    const last = Math.max(workspace.history.length - 1, 0)
    const index = Math.min(Math.max(workspace.historyCursor + by, 0), last)
    if (index !== workspace.historyCursor) void workspace.guard(() => workspace.showRevision(index))
  }

  const REASONS: Record<string, string> = {
    edit: 'saved over',
    force: 'written over by :w!',
    sftp: 'saved over the mount',
    restore: 'replaced by a restore',
  }
  export const reason = (summary: RevisionSummary) => REASONS[summary.reason] ?? summary.reason
</script>

<script lang="ts">
  import Keys from './Keys.svelte'
  import { diffLines, folded } from './diff'
  import { formatStamp, relative } from './util'

  let list = $state<HTMLUListElement>()

  $effect(() => {
    const index = workspace.historyCursor
    list?.querySelector<HTMLElement>(`[data-index="${index}"]`)?.scrollIntoView({ block: 'nearest' })
  })

  // Against the text as it is now: what restoring would take out and put back.
  const rows = $derived(workspace.revision ? folded(diffLines(workspace.draft.body, workspace.revision.body)) : [])
  const changed = $derived(rows.some((row) => row.kind === 'gone' || row.kind === 'added'))
</script>

<div class="sheet">
  <header>
    <span class="label rule">
      <span class="marker"></span><span class="title">history</span>
      {workspace.open?.name.trim() || workspace.open?.slug} · {workspace.history.length} earlier
    </span>
    {#if workspace.revision}
      <button onclick={restoreShown}>restore</button>
    {/if}
    <button onclick={() => (workspace.overlay = 'none')}>esc</button>
  </header>

  <div class="split">
    <ul bind:this={list}>
      {#each workspace.history as summary, index (summary.id)}
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
        <li class:at={index === workspace.historyCursor} data-index={index} onclick={() => workspace.guard(() => workspace.showRevision(index))}>
          <span class="when">{formatStamp(summary.saved_at)}</span>
          <span class="muted">{relative(summary.saved_at)} · {reason(summary)} · {summary.size} chars</span>
        </li>
      {:else}
        <li class="muted">no earlier versions yet — one is kept when a save replaces the text.</li>
      {/each}
    </ul>

    <div class="diff">
      {#if workspace.revision}
        {#if workspace.revision.name !== workspace.draft.name}
          <div class="row gone">- # {workspace.draft.name}</div>
          <div class="row added">+ # {workspace.revision.name}</div>
        {/if}
        {#each rows as row, index (index)}
          {#if row.kind === 'fold'}
            <div class="row fold">··· {row.count} unchanged</div>
          {:else}
            <div class="row {row.kind}">{row.kind === 'gone' ? '-' : row.kind === 'added' ? '+' : ' '} {row.text}</div>
          {/if}
        {/each}
        {#if !changed}
          <div class="row fold">the text is the same as now</div>
        {/if}
      {/if}
    </div>
  </div>

  <Keys contexts={['history', 'sheet']}><span class="muted">- goes, + comes back</span></Keys>
</div>

<style>
  /* A free-standing panel, as the desktop draws one: hull, one rim, the frame's radius. */
  .sheet {
    display: flex;
    flex-direction: column;
    gap: 18px;
    height: 100%;
    background: color-mix(in oklab, var(--hull) 97%, transparent);
    border: 1px solid var(--surface-alt);
    border-radius: var(--frame-radius);
    padding: 20px 24px;
    animation: resolve var(--t-slow) var(--ease-snap) both;
    width: min(100%, 140ch);
    overflow: hidden;
  }

  header { display: flex; align-items: center; gap: 12px; }
  header .label { flex: 1; }
  header .marker { background: var(--light); }
  header .title { color: var(--light); }

  .split { display: flex; gap: 18px; min-height: 0; flex: 1; }

  ul { list-style: none; margin: 0; padding: 0; overflow-y: auto; display: flex; flex-direction: column; gap: 2px; width: 34ch; flex: none; }

  li {
    display: flex;
    flex-direction: column;
    padding: 6px 8px;
    border-radius: var(--radius-m);
    cursor: pointer;
    transition: background-color var(--t-base) var(--ease-glide);
  }
  li:hover { background: var(--surface); }
  li.at { background: var(--surface-alt); box-shadow: inset 2px 0 0 var(--light); }
  .when { color: var(--fg); font-size: 13px; }
  .muted { font-size: 11px; }

  .diff {
    flex: 1;
    min-width: 0;
    overflow: auto;
    border: 1px solid var(--rim);
    border-radius: var(--radius-s);
    padding: 8px 0;
    font-size: 12px;
  }
  .row { white-space: pre-wrap; overflow-wrap: anywhere; padding: 0 10px; }
  .row.gone { background: color-mix(in oklab, var(--alarm) 16%, transparent); }
  .row.added { background: color-mix(in oklab, var(--light) 14%, transparent); }
  .row.fold { color: var(--muted); }


  @media (max-width: 767px) {
    .split { flex-direction: column; }
    ul { width: auto; max-height: 30%; }
  }
</style>
