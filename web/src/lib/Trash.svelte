<script lang="ts" module>
  import type { TrashItem } from './api'
  import { workspace } from './store.svelte'

  /*
   * What a row's buttons do, out here so the keys in App.svelte do the same:
   * Enter or r restores, x deletes for good.
   */
  export function restoreTrashed(item: TrashItem) {
    return workspace.guard(() => workspace.restore(item.id))
  }

  export async function purgeTrashed(item: TrashItem) {
    const inside = item.inside > 0 ? ` and the ${item.inside} inside it` : ''
    if (!(await workspace.ask(`delete "${item.name.trim() || item.slug}"${inside} for good? this cannot be undone.`))) return
    await workspace.guard(() => workspace.purge(item.id))
  }

  export async function emptyTrash() {
    if (!(await workspace.ask('empty the trash? everything in it is deleted for good.'))) return
    await workspace.guard(() => workspace.emptyTrash())
  }
</script>

<script lang="ts">
  import { relative } from './util'

  let sheet = $state<HTMLDivElement>()

  $effect(() => {
    const index = workspace.trashCursor
    sheet?.querySelector<HTMLElement>(`[data-index="${index}"]`)?.scrollIntoView({ block: 'nearest' })
  })

  const days = (at: number) => Math.max(Math.ceil((at - Date.now() / 1000) / 86400), 0)
</script>

<div class="sheet" bind:this={sheet}>
  <header>
    <span class="label rule"><span class="marker"></span><span class="title">trash</span> {workspace.trash.length} deleted</span>
    {#if workspace.trash.length > 0}
      <button class="danger" onclick={emptyTrash}>empty</button>
    {/if}
    <button onclick={() => (workspace.overlay = 'none')}>esc</button>
  </header>

  <ul>
    {#each workspace.trash as item, index (item.id)}
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <li class:at={index === workspace.trashCursor} data-index={index} onpointerenter={() => (workspace.trashCursor = index)}>
        <span class="kind label">{item.kind}</span>

        <div class="info">
          <span class="name">{item.name.trim() || item.slug}</span>
          <span class="muted">
            {item.from || 'the root'} · deleted {relative(item.deleted_at)}{item.inside > 0 ? ` · ${item.inside} inside` : ''} ·
            gone in {days(item.purge_at)}d
          </span>
        </div>

        <div class="actions">
          <button onclick={() => restoreTrashed(item)}>restore</button>
          <button class="danger" onclick={() => purgeTrashed(item)}>rm</button>
        </div>
      </li>
    {:else}
      <li class="muted">the trash is empty.</li>
    {/each}
  </ul>

  <footer class="label">
    <span class="kbd">j</span><span class="kbd">k</span> move
    <span class="gap"></span>
    <span class="kbd">enter</span><span class="kbd">r</span> restore
    <span class="gap"></span>
    <span class="kbd">x</span> delete for good
    <span class="gap"></span>
    <span class="kbd">q</span><span class="kbd">esc</span> close
  </footer>
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
    align-items: center;
    gap: 14px;
    padding: 6px 8px;
    border-radius: var(--radius-m);
    transition: background-color var(--t-base) var(--ease-glide);
  }
  li:hover { background: var(--surface); }
  li.at { background: var(--surface-alt); box-shadow: inset 2px 0 0 var(--light); }

  footer { display: flex; align-items: center; gap: 4px; font-size: 10px; }
  footer .kbd { margin-right: 2px; }
  footer .gap { width: 14px; }
  @media (max-width: 767px), (pointer: coarse) {
    footer { display: none; }
  }

  .kind { width: 64px; flex: none; font-size: 9px; letter-spacing: 0.14em; }

  .info { display: flex; flex-direction: column; min-width: 0; flex: 1; font-size: 12px; }
  .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--fg); font-family: var(--font-sans); font-size: 14px; }
  .muted { font-size: 11px; }

  .actions { display: flex; gap: 4px; flex: none; }
  .danger:hover { background: var(--alarm); border-color: var(--alarm); color: var(--ink); }
</style>
