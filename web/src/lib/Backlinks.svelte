<script lang="ts" module>
  import type { Backlink } from './api'
  import { workspace } from './store.svelte'

  /** What Enter and a click do: the linking document is opened. */
  export function openBacklink(link: Backlink) {
    workspace.overlay = 'none'
    return workspace.guard(() => workspace.openPath(link.path))
  }
</script>

<script lang="ts">
  import Keys from './Keys.svelte'
  let sheet = $state<HTMLDivElement>()

  $effect(() => {
    const index = workspace.backlinksCursor
    sheet?.querySelector<HTMLElement>(`[data-index="${index}"]`)?.scrollIntoView({ block: 'nearest' })
  })
</script>

<div class="sheet" bind:this={sheet}>
  <header>
    <span class="label rule">
      <span class="marker"></span><span class="title">backlinks</span>
      {workspace.open?.name.trim() || workspace.open?.slug} · linked from {workspace.backlinks.length}
    </span>
    <button onclick={() => (workspace.overlay = 'none')}>esc</button>
  </header>

  <ul>
    {#each workspace.backlinks as link, index (link.id)}
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
      <li
        class:at={index === workspace.backlinksCursor}
        data-index={index}
        onpointerenter={() => (workspace.backlinksCursor = index)}
        onclick={() => openBacklink(link)}
      >
        <span class="name">{link.name.trim() || link.slug}</span>
        <span class="muted">{link.path}</span>
        {#if link.context}<span class="context">{link.context}</span>{/if}
      </li>
    {:else}
      <li class="muted">nothing links here yet — a link is [text](/n/…) or [[title]].</li>
    {/each}
  </ul>

  <Keys contexts={['backlinks', 'sheet']} />
</div>

<style>
  /* A free-standing panel, as the desktop draws one: hull, one rim, the frame's radius. */
  .sheet {
    display: flex;
    flex-direction: column;
    gap: 18px;
    max-height: 100%;
    background: color-mix(in oklab, var(--hull) 97%, transparent);
    border: 1px solid var(--surface-alt);
    border-radius: var(--frame-radius);
    padding: 20px 24px;
    animation: resolve var(--t-slow) var(--ease-snap) both;
    width: min(100%, 92ch);
    overflow: hidden;
  }

  header { display: flex; align-items: center; gap: 12px; }
  header .label { flex: 1; }
  header .marker { background: var(--light); }
  header .title { color: var(--light); }

  ul { list-style: none; margin: 0; padding: 0; overflow-y: auto; display: flex; flex-direction: column; gap: 2px; }

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

  .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--fg); font-family: var(--font-sans); font-size: 14px; }
  .muted { font-size: 11px; }
  .context { font-size: 12px; color: var(--subtle); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

</style>
