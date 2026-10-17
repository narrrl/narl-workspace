<script lang="ts">
  import { api } from './api'
  import { hooks } from './commands'
  import { embedSnippet } from './markdown'
  import { workspace } from './store.svelte'
  import { formatBytes, formatStamp } from './util'

  const usage = (nodeIds: number[]) =>
    nodeIds.length === 0 ? 'unattached' : nodeIds.map((id) => `#${id}`).join(' ')

  async function remove(id: string, filename: string, nodeIds: number[]) {
    const used = nodeIds.length === 1 ? '1 document' : `${nodeIds.length} documents`
    const warning = nodeIds.length > 0 ? ` it is embedded in ${used}, which will break.` : ''
    if (!confirm(`delete ${filename}?${warning}`)) return
    await workspace.guard(async () => {
      await api.deleteMedia(id)
      await workspace.loadMedia()
    })
  }
</script>

<div class="sheet">
  <header>
    <span class="label rule"><span class="marker"></span><span class="title">media</span> {workspace.media.length} files</span>
    <button onclick={() => (workspace.overlay = 'none')}>esc</button>
  </header>

  <ul>
    {#each workspace.media as file (file.id)}
      <li>
        <div class="thumb">
          {#if file.mime.startsWith('image/')}
            <img src={file.url} alt={file.filename} loading="lazy" />
          {:else}
            <span class="label">{file.mime.split('/')[0]}</span>
          {/if}
        </div>

        <div class="info">
          <span class="name">{file.filename}</span>
          <span class="muted">
            {formatBytes(file.size)} · {formatStamp(file.created_at)} ·
            {usage(file.node_ids)}
          </span>
        </div>

        <div class="actions">
          {#if workspace.editing}
            <button
              onclick={() => {
                hooks.insertText(embedSnippet(file))
                workspace.overlay = 'none'
              }}>insert</button
            >
          {/if}
          <button onclick={() => workspace.copy(location.origin + file.url)}>copy url</button>
          <button class="danger" onclick={() => remove(file.id, file.filename, file.node_ids)}>rm</button>
        </div>
      </li>
    {:else}
      <li class="muted">nothing uploaded yet — attach a file from the editor.</li>
    {/each}
  </ul>
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
