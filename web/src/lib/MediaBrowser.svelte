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
    workspace.overlay = 'none'
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
  import { formatBytes, formatStamp } from './util'

  let sheet = $state<HTMLDivElement>()

  $effect(() => {
    const index = workspace.mediaCursor
    sheet?.querySelector<HTMLElement>(`[data-index="${index}"]`)?.scrollIntoView({ block: 'nearest' })
  })

  const usage = (nodeIds: number[], fileNode: number | null) => {
    const embeds = nodeIds.map((id) => `#${id}`).join(' ')
    if (fileNode !== null) return embeds ? `in the tree · ${embeds}` : 'in the tree'
    return embeds || 'unattached'
  }
</script>

<div class="sheet" bind:this={sheet}>
  <header>
    <span class="label rule"><span class="marker"></span><span class="title">media</span> {workspace.media.length} files</span>
    <button onclick={() => (workspace.overlay = 'none')}>esc</button>
  </header>

  <ul>
    {#each workspace.media as file, index (file.id)}
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <li class:at={index === workspace.mediaCursor} data-index={index} onpointerenter={() => (workspace.mediaCursor = index)}>
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

  <footer class="label">
    <span class="kbd">j</span><span class="kbd">k</span> move
    <span class="gap"></span>
    {#if workspace.editing}
      <span class="kbd">i</span> insert
      <span class="gap"></span>
    {/if}
    <span class="kbd">y</span> copy url
    <span class="gap"></span>
    <span class="kbd">x</span> delete
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
  }

  header { display: flex; align-items: center; gap: 12px; }
  header .label { flex: 1; }
  header .marker { background: var(--light); }
  header .title { color: var(--light); }
  .sheet { width: min(100%, 92ch); overflow: hidden; }

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
  .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--fg); font-family: var(--font-sans); font-size: 14px; }
  .muted { font-size: 11px; }

  .actions { display: flex; gap: 4px; flex: none; }
  .danger:hover { background: var(--alarm); border-color: var(--alarm); color: var(--ink); }
</style>
