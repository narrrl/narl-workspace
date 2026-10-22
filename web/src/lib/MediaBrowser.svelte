<script lang="ts" module>
  import { api, type MediaFile } from './api'
  import { hooks } from './commands'
  import { embedSnippet } from './markdown'
  import { workspace } from './store.svelte'

  /*
   * What a row's buttons do, out here so the keys in App.svelte do the same:
   * i inserts, y copies, x deletes.
   */
  export function insertMedia(file: MediaFile) {
    if (!workspace.editing) return workspace.say('insert works while writing — i on a document first', 'error')
    hooks.insertText(embedSnippet(file))
    workspace.closeSheet()
  }

  export async function copyMedia(file: MediaFile) {
    const copied = await workspace.copy(location.origin + file.url)
    workspace.say(copied ? `copied the address of ${file.filename}` : 'the clipboard said no', copied ? 'info' : 'error')
  }

  export async function removeMedia(file: MediaFile) {
    if (file.file_node !== null) return workspace.say(`${file.filename} is a file in the tree — delete it there`, 'error')
    const used = file.node_ids.length === 1 ? '1 document' : `${file.node_ids.length} documents`
    const warning = file.node_ids.length > 0 ? ` it is embedded in ${used}, which will break` : ''
    if (!(await workspace.ask(`delete ${file.filename}?${warning}`))) return
    await workspace.guard(async () => {
      await api.deleteMedia(file.id)
      await workspace.loadMedia()
      workspace.mediaCursor = Math.min(workspace.mediaCursor, Math.max(workspace.media.length - 1, 0))
    })
  }
</script>

<script lang="ts">
  import Sheet from './Sheet.svelte'
  import { formatBytes, formatStamp } from './util'

  const usage = (nodeIds: number[], fileNode: number | null) => {
    const embeds = nodeIds.map((id) => `#${id}`).join(' ')
    if (fileNode !== null) return embeds ? `in the tree · ${embeds}` : 'in the tree'
    return embeds || 'unattached'
  }
</script>

<Sheet title="media" detail="{workspace.media.length} files" cursor={workspace.mediaCursor} keys={['media', 'sheet']} hide={workspace.editing ? [] : ['insert']}>
  <ul class="items">
    {#each workspace.media as file, index (file.id)}
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <li class="item" class:at={index === workspace.mediaCursor} data-index={index} onpointerenter={() => (workspace.mediaCursor = index)}>
        <div class="thumb">
          {#if file.encrypted}
            <span class="label">sealed</span>
          {:else if file.mime.startsWith('image/')}
            <img src={file.url} alt={file.filename} loading="lazy" />
          {:else}
            <span class="label">{file.mime.split('/')[0]}</span>
          {/if}
        </div>

        <div class="info">
          <span class="name">{file.filename}</span>
          <span class="muted">
            {formatBytes(file.size)} · {formatStamp(file.created_at)} ·
            {usage(file.node_ids, file.file_node)}
          </span>
        </div>

        <div class="actions">
          {#if workspace.editing}
            <button onclick={() => insertMedia(file)}>insert</button>
          {/if}
          <button onclick={() => copyMedia(file)}>copy url</button>
          {#if file.file_node === null}
            <button class="danger" onclick={() => removeMedia(file)}>rm</button>
          {/if}
        </div>
      </li>
    {:else}
      <li class="muted">nothing uploaded yet — attach a file from the editor.</li>
    {/each}
  </ul>
</Sheet>

<style>
  .items > .item { flex-direction: row; align-items: center; gap: 14px; cursor: default; }

  .thumb {
    width: 52px;
    height: 38px;
    flex: none;
    display: grid;
    place-items: center;
    background: var(--surface);
    border: 1px solid var(--rim);
    border-radius: var(--radius-s);
    overflow: hidden;
  }
  .thumb .label { font-size: 9px; letter-spacing: 0.14em; }
  .thumb img { width: 100%; height: 100%; object-fit: cover; }

  .info { display: flex; flex-direction: column; min-width: 0; flex: 1; font-size: 12px; }

  .actions { display: flex; gap: 4px; flex: none; }
</style>
