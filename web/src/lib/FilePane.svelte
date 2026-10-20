<script lang="ts">
  import CertView from './CertView.svelte'
  import { hooks, runCommand, type ScrollAmount, type SidewaysAmount } from './commands'
  import Icon from './Icon.svelte'
  import { renderMarkdown } from './markdown'
  import { looksLikeText, parseTable, previewKind, separatorOf } from './preview'
  import { workspace, type Tile } from './store.svelte'
  import { formatBytes, formatStamp, headingsIn, jumpHeading, readerHeadings, relative, scrollPage, scrollWide } from './util'

  /** The tile this pane draws; the keys only reach it while it has focus. */
  const { tile }: { tile: Tile } = $props()

  const focused = $derived(workspace.tile === tile)
  const node = $derived(tile.open!)
  const file = $derived(node.file!)
  /* A sealed file cannot be fetched while the vault is shut. */
  const locked = $derived(file.encrypted && !workspace.vault?.unlocked)
  const kind = $derived(previewKind(node.name, file.mime))
  /* What the browser can show by itself; everything else is a download. */
  const viewable = $derived(file.mime !== 'application/octet-stream')

  /*
   * Text is shown inline, but only its beginning: a log file can run to
   * gigabytes, and the download has the rest. A file nothing is known about
   * gets its first few kilobytes looked at, and is shown as text if that is
   * what they are.
   */
  const TEXT_PREVIEW = 256 * 1024
  const SNIFF = 8 * 1024
  let text = $state<string | null>(null)
  /* Markdown is read rendered, unless asked for its source. */
  let source = $state(false)
  $effect(() => {
    text = null
    source = false
    if (locked || file.size === 0) return
    if (kind !== 'text' && kind !== 'markdown' && kind !== 'table' && kind !== 'none') return
    const url = file.url
    const sniff = kind === 'none'
    void (async () => {
      if (sniff) {
        const start = await head(url, SNIFF)
        if (!start || !looksLikeText(start)) return
      }
      const bytes = await head(url, TEXT_PREVIEW)
      if (bytes !== null && url === file.url) text = new TextDecoder().decode(bytes)
    })()
  })

  const lines = $derived(text === null ? [] : text.replace(/\n$/, '').split('\n'))

  const html = $derived(kind === 'markdown' && text !== null ? renderMarkdown(text) : '')

  const TABLE_ROWS = 500
  const table = $derived(kind === 'table' && text !== null ? parseTable(text, separatorOf(node.name), TABLE_ROWS) : [])

  /** The first `limit` bytes of a download; the rest is never fetched. */
  async function head(url: string, limit: number): Promise<Uint8Array | null> {
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
    return bytes.subarray(0, size)
  }

  let reader = $state<HTMLElement>()

  let cert = $state<CertView>()

  // Same arrangement as the document pane: j/k scroll whatever is shown here,
  // and Enter does the one thing the file is waiting for — the vault's
  // password while it is sealed, a keystore's when it has one.
  $effect(() => {
    if (!focused) return
    const scroll = (amount: ScrollAmount) => {
      if (!reader) return false
      scrollPage(reader, amount)
      return true
    }
    const act = () => {
      if (locked) return void runCommand('unlock'), true
      return cert?.act() ?? false
    }
    const hints = () => reader ?? null
    // A CSV's table or a long line of text is what runs off the side here.
    const sideways = (amount: SidewaysAmount) => !!reader && scrollWide(reader, reader, amount)
    const flip = () => {
      if (kind !== 'markdown' || text === null) return false
      source = !source
      return true
    }
    hooks.scrollReader = scroll
    hooks.fileAction = act
    hooks.hintRoot = hints
    hooks.toggleSource = flip
    hooks.scrollWide = sideways
    return () => {
      if (hooks.scrollWide === sideways) hooks.scrollWide = () => false
      if (hooks.scrollReader === scroll) hooks.scrollReader = () => false
      if (hooks.fileAction === act) hooks.fileAction = () => false
      if (hooks.hintRoot === hints) hooks.hintRoot = () => null
      if (hooks.toggleSource === flip) hooks.toggleSource = () => false
    }
  })

  // Markdown read rendered has headings to jump between, as a document does.
  let article = $state<HTMLElement>()
  $effect(() => {
    const scroller = reader
    const root = article
    if (!scroller || !root || !focused) return
    const jump = (direction: 1 | -1) => jumpHeading(scroller, headingsIn(root), direction)
    const list = () => readerHeadings(scroller, root)
    hooks.jumpHeading = jump
    hooks.headings = list
    return () => {
      if (hooks.jumpHeading === jump) hooks.jumpHeading = () => false
      if (hooks.headings === list) hooks.headings = () => ({ headings: [], current: 0 })
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
        {#if file.encrypted}
          <dt>vault</dt>
          <dd class="light">{locked ? 'sealed, locked' : 'sealed, open'}</dd>
        {/if}
      </dl>

      <div class="actions">
        {#if locked}
          <button onclick={() => runCommand('unlock')}>unlock</button>
        {:else}
          {#if kind === 'markdown' && text !== null}
            <button onclick={() => (source = !source)}>{source ? 'rendered' : 'source'}</button>
          {/if}
          {#if viewable}
            <a class="button" href={file.url} target="_blank" rel="noopener">open</a>
          {/if}
          <a class="button" href={file.url} download={node.name}>download</a>
        {/if}
        <button onclick={() => runCommand(file.encrypted ? 'decrypt' : 'encrypt')}>
          {file.encrypted ? 'decrypt' : 'encrypt'}
        </button>
        <button class="danger" onclick={() => runCommand('d')}>delete</button>
        <button class="close" onclick={() => runCommand('q')}>close</button>
      </div>
    </div>
  </header>

  <div class="body" bind:this={reader}>
    <h1 class="name">{node.name}</h1>
    {#key `${file.url}:${locked}`}
      {#if locked}
        <p class="sealed">
          <Icon name="lock" size={16} />
          sealed in the vault — <span class="kbd">↵</span> or <button onclick={() => runCommand('unlock')}>unlock</button> to open it.
        </p>
      {:else if kind === 'image'}
        <img src={file.url} alt={node.name} />
      {:else if kind === 'video'}
        <!-- svelte-ignore a11y_media_has_caption -->
        <video src={file.url} controls playsinline preload="metadata"></video>
      {:else if kind === 'audio'}
        <audio src={file.url} controls preload="metadata"></audio>
      {:else if kind === 'pdf'}
        <iframe class="pdf" src={file.url} title={node.name}></iframe>
      {:else if kind === 'cert'}
        <CertView bind:this={cert} id={node.id} name={node.name} />
      {:else if text !== null}
        {#if kind === 'markdown' && !source}
          <!-- eslint-disable-next-line svelte/no-at-html-tags -- sanitised in renderMarkdown -->
          <article class="md" bind:this={article}>{@html html}</article>
        {:else if kind === 'table'}
          <div class="table">
            <table>
              <thead>
                <tr>
                  {#each table[0] ?? [] as cell, j (j)}<th>{cell}</th>{/each}
                </tr>
              </thead>
              <tbody>
                {#each table.slice(1) as row, i (i)}
                  <tr>
                    {#each row as cell, j (j)}<td>{cell}</td>{/each}
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
          {#if table.length === TABLE_ROWS}
            <p class="muted">the first {TABLE_ROWS} rows — download it for the rest.</p>
          {/if}
        {:else}
          <pre class="lines">{#each lines as line, i (i)}<span>{line}
</span>{/each}</pre>
        {/if}
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

  .lines { counter-reset: line; }
  .lines span::before {
    counter-increment: line;
    content: counter(line);
    display: inline-block;
    width: 4ch;
    margin-right: 1.5ch;
    color: var(--muted);
    text-align: right;
    user-select: none;
  }
  .pdf {
    display: block;
    width: 100%;
    height: 80vh;
    border: 1px solid var(--rim);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .table { overflow-x: auto; }
  table { border-collapse: collapse; font-size: 12px; }
  th, td {
    padding: 4px 10px;
    border: 1px solid var(--rim);
    text-align: left;
    vertical-align: top;
    white-space: pre-wrap;
  }
  th { color: var(--light); background: var(--surface); }

  .light { color: var(--light); }
  .sealed { display: flex; align-items: center; gap: 8px; color: var(--subtle); }

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
