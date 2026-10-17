<script lang="ts">
  import { api, type SharedDocument } from './api'
  import { renderMarkdown } from './markdown'
  import { formatStamp, formatWeekday } from './util'

  const { token, shareKey }: { token: string; shareKey: string } = $props()

  let doc = $state<SharedDocument | null>(null)
  let error = $state('')

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

<main>
  {#if error}
    <p class="alarm">{error}</p>
  {:else if doc}
    <article class="md resolve">
      <div class="meta label rule">
        <span class="marker"></span>
        <span>{formatStamp(doc.created_at)} {formatWeekday(doc.created_at)}</span>
        <span class="tag">shared document</span>
      </div>
      {#if doc.name.trim()}<h1>{doc.name}</h1>{/if}
      <!-- eslint-disable-next-line svelte/no-at-html-tags -- sanitised in renderMarkdown -->
      {@html renderMarkdown(doc.body, { token, key: shareKey })}
    </article>
    <footer class="label">— written in ~/workspace</footer>
  {:else}
    <p class="label">loading<span class="cursor">_</span></p>
  {/if}
</main>

<style>
  main {
    height: 100%;
    overflow-y: auto;
    padding: 56px 20px 20vh;
    display: flex;
    flex-direction: column;
    align-items: center;
    background:
      radial-gradient(120% 70% at 30% 0%, #141713 0%, transparent 60%),
      var(--base);
  }

  article { width: min(100%, 72ch); }

  .meta { margin-bottom: 28px; font-size: 10px; }
  .meta .marker { background: var(--light); }

  .tag { color: var(--light); }

  h1 { margin-top: 0 !important; font-size: 2.1em !important; }

  footer { width: min(100%, 72ch); margin-top: 56px; font-size: 10px; }
  p { width: min(100%, 72ch); }
</style>
