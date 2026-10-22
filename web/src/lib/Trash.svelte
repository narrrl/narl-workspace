<script lang="ts" module>
  import type { TrashItem } from './api'
  import { workspace } from './store.svelte'

  /*
   * What a row's buttons do, out here so the keys in actions.ts do the same:
   * Enter or r restores, x deletes for good.
   */
  export function restoreTrashed(item: TrashItem) {
    return workspace.guard(() => workspace.tree.restore(item.id))
  }

  export async function purgeTrashed(item: TrashItem) {
    const inside = item.inside > 0 ? ` and the ${item.inside} inside it` : ''
    if (!(await workspace.prompts.ask(`delete "${item.name.trim() || item.slug}"${inside} for good? this cannot be undone.`))) return
    await workspace.guard(() => workspace.sheets.purge(item.id))
  }

  export async function emptyTrash() {
    if (!(await workspace.prompts.ask('empty the trash? everything in it is deleted for good.'))) return
    await workspace.guard(() => workspace.sheets.emptyTrash())
  }
</script>

<script lang="ts">
  import Sheet from './Sheet.svelte'
  import { relative } from './util'

  const days = (at: number) => Math.max(Math.ceil((at - Date.now() / 1000) / 86400), 0)
</script>

<Sheet title="trash" detail="{workspace.sheets.trash.length} deleted" cursor={workspace.sheets.trashCursor} keys={['trash', 'sheet']}>
  {#snippet actions()}
    {#if workspace.sheets.trash.length > 0}
      <button class="danger" onclick={emptyTrash}>empty</button>
    {/if}
  {/snippet}

  <ul class="items">
    {#each workspace.sheets.trash as item, index (item.id)}
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <li class="item" class:at={index === workspace.sheets.trashCursor} data-index={index} onpointerenter={() => (workspace.sheets.trashCursor = index)}>
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
</Sheet>

<style>
  .items > .item { flex-direction: row; align-items: center; gap: 14px; cursor: default; }

  .kind { width: 64px; flex: none; font-size: 9px; letter-spacing: 0.14em; }

  .info { display: flex; flex-direction: column; min-width: 0; flex: 1; font-size: 12px; }

  .actions { display: flex; gap: 4px; flex: none; }
</style>
