<script lang="ts">
  import { api, type SharedDocument } from './api'
  import { renderMarkdown } from './markdown'
  import Outline from './Outline.svelte'
  import { formatStamp, formatWeekday } from './util'

  const { token, shareKey }: { token: string; shareKey: string } = $props()

  let doc = $state<SharedDocument | null>(null)
  let error = $state('')
  let page = $state<HTMLElement>()
  let article = $state<HTMLElement>()
  const html = $derived(doc ? renderMarkdown(doc.body, { token, key: shareKey }) : '')

  $effect(() => {
    if (!shareKey) {
      error = 'this link is incomplete — the part after # is missing from it.'
      return
    }
    api
      .readShared(token, shareKey)
      .then((data) => {
        doc = data
        document.title = data.name.trim() || `~/workspace ${formatStamp(data.created_at)}`
      })
      .catch(() => (error = 'this link is not valid, or the document is no longer shared.'))
  })
</script>

<main bind:this={page}>
  {#if error}
    <p class="alarm">{error}</p>
  {:else if doc}
    <article class="md resolve" bind:this={article}>
      <div class="meta label rule">
        <span class="marker"></span>
        <span>{formatStamp(doc.created_at)} {formatWeekday(doc.created_at)}</span>
        <span class="tag">shared document</span>
      </div>
      {#if doc.name.trim()}<h1>{doc.name}</h1>{/if}
      <!-- eslint-disable-next-line svelte/no-at-html-tags -- sanitised in renderMarkdown -->
      {@html html}
    </article>
    <aside class="side">
      <Outline root={article} scroller={page} source={html} />
    </aside>
    <footer class="label">— written in ~/workspace</footer>
  {:else}
    <p class="label">loading<span class="cursor">_</span></p>
  {/if}
</main>

<style>
  /* The same page as the reading pane: a centred column, the outline in the margin. */
  main {
    height: 100%;
    overflow-y: auto;
    padding: 56px 20px 20vh;
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 46rem) minmax(0, 1fr);
    align-content: start;
    align-items: start;
    container-type: inline-size;
    background:
      radial-gradient(120% 70% at 30% 0%, #141713 0%, transparent 60%),
      var(--base);
  }

  article, footer, p { grid-column: 2; }
  article { max-width: none; }
  .side {
    grid-column: 3;
    grid-row: 1;
    position: sticky;
    top: 0;
    max-width: 30ch;
    padding: 4px 0 0 44px;
  }
  @container (max-width: 1000px) {
    .side { display: none; }
  }

  .meta { margin-bottom: 28px; font-size: 10px; }
  .meta .marker { background: var(--light); }

  .tag { color: var(--light); }

  h1 { margin-top: 0 !important; font-size: 2.1em !important; }

  footer { margin-top: 56px; font-size: 10px; }
</style>
