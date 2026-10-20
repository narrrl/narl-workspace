<script lang="ts">
  /*
   * A shared document, seen through the same frame as the workspace it was
   * written in: the bar, a porthole with the outline and the document tiled
   * in it as two windows, the band along the bottom. No rail — a reader has
   * no tools — but the reading keys work: j/k, gg/G, Ctrl-d/u.
   */
  import { api, unreachable, type SharedDocument } from './api'
  import Clock from './Clock.svelte'
  import { renderMarkdown } from './markdown'
  import Outline from './Outline.svelte'
  import { formatStamp, formatWeekday, scrollPage, slugify, type ScrollAmount } from './util'

  const { token, shareKey }: { token: string; shareKey: string } = $props()

  let doc = $state<SharedDocument | null>(null)
  let error = $state('')
  let scroller = $state<HTMLElement>()
  let article = $state<HTMLElement>()
  let position = $state('All')
  // Links into the private workspace come back as their text; see renderMarkdown.
  const html = $derived(doc ? renderMarkdown(doc.body, { token, key: shareKey }) : '')
  const title = $derived(doc?.name.trim() ?? '')
  const words = $derived(doc ? doc.body.split(/\s+/).filter(Boolean).length : 0)
  // The outline gets a window of its own only when there is something to outline.
  const sections = $derived((html.match(/<h[23][\s>]/g) ?? []).length)

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
      .catch((e) =>
        (error = unreachable(e)
          ? 'the server cannot be reached — try again in a moment.'
          : 'this link is not valid, or the document is no longer shared.'),
      )
  })

  /** Where the view is, the way vim's ruler says it: Top, Bot, All or a percentage. */
  function measure() {
    if (!scroller) return
    const room = scroller.scrollHeight - scroller.clientHeight
    const top = scroller.scrollTop
    position = room <= 1 ? 'All' : top <= 1 ? 'Top' : top >= room - 1 ? 'Bot' : `${Math.round((top / room) * 100)}%`
  }

  $effect(() => {
    void html
    requestAnimationFrame(measure)
  })

  let pending = ''
  function onkeydown(event: KeyboardEvent) {
    if (!scroller || event.altKey || event.metaKey) return
    let amount: ScrollAmount | null = null
    if (event.ctrlKey) {
      if (event.key === 'd') amount = 'halfdown'
      else if (event.key === 'u') amount = 'halfup'
    } else if (event.key === 'j' || event.key === 'ArrowDown') amount = 1
    else if (event.key === 'k' || event.key === 'ArrowUp') amount = -1
    else if (event.key === 'G') amount = 'bottom'
    else if (event.key === 'g' && pending === 'g') amount = 'top'
    pending = event.key === 'g' && pending !== 'g' ? 'g' : ''
    if (amount === null) return
    event.preventDefault()
    scrollPage(scroller, amount)
  }
</script>

<svelte:window {onkeydown} />

<div class="shell shared">
  <header class="bar">
    <span class="mark" aria-hidden="true"></span>
    <span class="divider"></span>
    <span class="pill" aria-current="true">shared</span>
    <span class="divider"></span>
    <span class="title">{title}</span>
    <span class="spacer"></span>
    <span class="readout ro"><span class="key">mode</span><span class="value">read only</span></span>
    <span class="divider ro"></span>
    <Clock />
  </header>

  <main class="porthole">
    <div class="panes switch" class:split={sections >= 2}>
      {#if doc && sections >= 2}
        <aside class="window toc">
          <Outline root={article} {scroller} source={html} />
        </aside>
      {/if}

      <div class="window doc" class:focused={sections >= 2}>
        <div class="body" bind:this={scroller} onscroll={measure}>
          {#if error}
            <div class="standby">
              <h2 class="label rule">link</h2>
              <p class="alarm">{error}</p>
            </div>
          {:else if doc}
            <article class="md resolve" bind:this={article}>
              <div class="meta label rule">
                <span class="marker"></span>
                <span>{formatStamp(doc.created_at)} {formatWeekday(doc.created_at)}</span>
                <span class="tag">shared document</span>
              </div>
              {#if title}<h1>{title}</h1>{/if}
              <!-- eslint-disable-next-line svelte/no-at-html-tags -- sanitised in renderMarkdown -->
              {@html html}
              <footer class="label">— written in ~/workspace</footer>
            </article>
          {:else}
            <div class="standby">
              <p class="label">loading<span class="cursor">_</span></p>
            </div>
          {/if}
        </div>
      </div>
    </div>
  </main>

  <footer class="status">
    <span class="mode">read</span>
    <span class="file">shared/{slugify(title) || token.slice(0, 8)}</span>
    <span class="msg"></span>
    {#if doc}
      <span class="readout">{words}<span class="unit">W</span></span>
      <span class="readout">{position}</span>
    {/if}
    <span class="hint">j k scroll · gg G ends · ^d ^u half</span>
  </footer>
</div>

<style>
  /* No rail: the porthole and the bands run to the frame on the right too. */
  .shared { --rail-w: var(--frame); }

  .title {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    letter-spacing: 0.06em;
    text-transform: none;
    color: var(--light);
  }

  .panes { grid-template-columns: minmax(0, 1fr); }
  .panes.split { grid-template-columns: minmax(26ch, 22vw) minmax(0, 1fr); }

  .toc { --pane-bg: color-mix(in oklab, var(--surface) 60%, var(--base)); padding: 22px 16px 22px 20px; overflow-y: auto; }

  .doc { display: flex; flex-direction: column; }
  /* A lone window has nothing to stand out from, so it keeps the plain rim. */
  .doc:not(.focused) { border-color: transparent; background: transparent; }

  .body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 40px 28px 30vh;
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 46rem) minmax(0, 1fr);
    align-content: start;
  }
  .body > * { grid-column: 2; }
  article { max-width: none; }

  .meta { margin-bottom: 28px; font-size: 10px; }
  .meta .marker { background: var(--light); }
  .tag { color: var(--light); }

  h1 { margin-top: 0 !important; font-size: 2.1em !important; }

  article footer { margin-top: 56px; font-size: 10px; }

  .standby { padding-top: 18vh; }
  .standby h2 { margin: 0 0 14px; font-weight: 400; }

  .status .mode { background: var(--light); color: var(--ink); }
  .file { color: var(--subtle); }
  .msg { flex: 1; }

  @media (max-width: 1000px) {
    .panes.split { grid-template-columns: minmax(0, 1fr); }
    .toc { display: none; }
    .doc.focused {
      background: linear-gradient(var(--pane-bg), var(--pane-bg)) padding-box,
        linear-gradient(var(--overlay), var(--overlay)) border-box;
      box-shadow: none;
    }
  }

  @media (max-width: 767px) {
    .bar .ro { display: none; }
    .body { padding: 24px 18px 30vh; grid-template-columns: minmax(0, 1fr); }
    .body > * { grid-column: 1; }
    .status { padding: 0 12px; gap: 12px; }
    .hint, .file { display: none; }
  }
</style>
