<script lang="ts">
  import { hooks, runCommand, type ScrollAmount } from './commands'
  import Icon from './Icon.svelte'
  import { workspace } from './store.svelte'
  import { formatBytes, formatStamp, relative, scrollPage } from './util'

  const node = $derived(workspace.open!)
  const file = $derived(node.file!)
  const kind = $derived(file.mime.split('/')[0])
  /* What the browser can show by itself; everything else is a download. */
  const viewable = $derived(file.mime !== 'application/octet-stream')

  /*
   * Plain text is shown inline, but only its beginning: a log file can run to
   * gigabytes, and the download has the rest.
   */
  const TEXT_PREVIEW = 256 * 1024
  let text = $state<string | null>(null)
  $effect(() => {
    text = null
    if (file.mime !== 'text/plain') return
    const url = file.url
    void head(url, TEXT_PREVIEW).then((body) => {
      if (body !== null && url === file.url) text = body
    })
  })

  /** The first `limit` bytes of a download, as text; the rest is never fetched. */
  async function head(url: string, limit: number): Promise<string | null> {
    const response = await fetch(url)
    if (!response.ok || !response.body) return null
    const reader = response.body.getReader()
    const bytes = new Uint8Array(limit)
    let size = 0
    while (size < limit) {
      const { done, value } = await reader.read()
      if (done) break
      const take = value.subarray(0, limit - size)
      bytes.set(take, size)
      size += take.length
    }
    void reader.cancel()
    return new TextDecoder().decode(bytes.subarray(0, size))
  }

  let reader = $state<HTMLElement>()

  // Same arrangement as the document pane: j/k scroll whatever is shown here.
  $effect(() => {
    const scroll = (amount: ScrollAmount) => {
      if (!reader) return false
      scrollPage(reader, amount)
      return true
    }
    hooks.scrollReader = scroll
    return () => {
      if (hooks.scrollReader === scroll) hooks.scrollReader = () => false
    }
  })
</script>

<section class="pane">
  <header>
    <div class="crumb label rule" title="{workspace.here}/{node.slug}">
      <button class="back" aria-label="back to the list" onclick={() => runCommand('q')}>
        <Icon name="back" size={16} />
      </button>
      <span class="marker"></span>
      <span class="path">{workspace.here}/{node.slug}</span>
    </div>

    <div class="row">
      <dl class="meta">
        <dt>size</dt>
        <dd>{formatBytes(file.size)}</dd>
        <dt>type</dt>
        <dd>{file.mime}</dd>
        <dt>uploaded</dt>
        <dd title={formatStamp(node.created_at)}>{relative(node.created_at)}</dd>
      </dl>

      <div class="actions">
        {#if viewable}
          <a class="button" href={file.url} target="_blank" rel="noopener">open</a>
        {/if}
        <a class="button" href={file.url} download={node.name}>download</a>
        <button class="danger" onclick={() => runCommand('d')}>delete</button>
        <button class="close" onclick={() => runCommand('q')}>close</button>
      </div>
    </div>
  </header>

  <div class="body" bind:this={reader}>
    <h1 class="name">{node.name}</h1>
    {#key file.url}
      {#if kind === 'image'}
        <img src={file.url} alt={node.name} />
      {:else if kind === 'video'}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video src={file.url} controls playsinline preload="metadata"></video>
      {:else if kind === 'audio'}
        <audio src={file.url} controls preload="metadata"></audio>
      {:else if text !== null}
        <pre>{text}</pre>
        {#if file.size > TEXT_PREVIEW}
          <p class="muted">the first {formatBytes(TEXT_PREVIEW)} of {formatBytes(file.size)} — download it for the rest.</p>
        {/if}
      {:else}
        <p class="muted">
          no preview for {file.mime} —
          {#if viewable}<a href={file.url} target="_blank" rel="noopener">open it</a> or{/if}
          <a href={file.url} download={node.name}>download it</a>.
        </p>
      {/if}
    {/key}
  </div>
</section>

<style>
  .pane {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 0;
  }

  header {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 16px 24px 12px 28px;
    flex: none;
  }

  .crumb { min-width: 0; white-space: nowrap; }
  .path {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--subtle);
    letter-spacing: 0.08em;
    text-transform: none;
  }

  .row {
    display: flex;
    gap: 12px;
    align-items: center;
    justify-content: space-between;
    min-width: 0;
  }

  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 8px;
    margin: 0;
    font-size: 11px;
    min-width: 0;
  }
  .meta dt { color: var(--muted); letter-spacing: 0.18em; text-transform: uppercase; }
  .meta dd { margin: 0 10px 0 0; color: var(--fg); }

  .actions { display: flex; gap: 4px; flex-wrap: wrap; flex: none; }
  .actions .danger:hover { background: var(--alarm); border-color: var(--alarm); color: var(--ink); }

  .body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 18px 28px 30vh;
  }
  .name {
    margin: 0 0 18px;
    color: var(--light);
    font-family: var(--font-sans);
    font-size: 24px;
    font-weight: 300;
    overflow-wrap: anywhere;
  }
  img, video {
    display: block;
    max-width: 100%;
    max-height: 75vh;
    border-radius: var(--radius-m);
  }
  audio { width: 100%; max-width: 46rem; }
  pre {
    margin: 0;
    padding: 14px 16px;
    border: 1px solid var(--rim);
    border-radius: var(--radius-m);
    background: var(--surface);
    font-family: var(--font-mono);
    font-size: 12px;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .back { display: none; }

  @media (max-width: 767px) {
    header { padding: 10px 14px 10px 8px; gap: 8px; }
    .crumb { gap: 8px; }
    .back {
      display: grid;
      place-items: center;
      width: 32px;
      height: 32px;
      flex: none;
      padding: 0;
      border: 0;
      color: var(--light);
    }
    .row { flex-direction: column; align-items: stretch; gap: 10px; padding-left: 6px; }
    .actions { display: grid; grid-auto-flow: column; grid-auto-columns: 1fr; gap: 6px; }
    .actions :is(button, .button) { height: 36px; }
    .actions .close { display: none; }
    .body { padding: 14px 16px 30vh; }
  }
</style>
