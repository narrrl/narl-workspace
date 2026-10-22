<script lang="ts">
  import { followLink, hooks, openInfo, runCommand, type ScrollAmount, type SidewaysAmount } from './commands'
  import { renderMarkdown } from './markdown'
  import Outline from './Outline.svelte'
  import { workspace, type Tile } from './store.svelte'
  import Icon from './Icon.svelte'
  import {
    formatDay,
    formatStamp,
    formatWeekday,
    headingsIn,
    isTouchDevice,
    jumpHeading,
    readerHeadings,
    relative,
    scrollPage,
    scrollWide,
    slugPath,
  } from './util'

  /** The tile this pane draws; the keys only reach it while it has focus. */
  const { tile }: { tile: Tile } = $props()

  const focused = $derived(workspace.tile === tile)
  const doc = $derived(tile.open!)
  const html = $derived(renderMarkdown(tile.editing ? tile.draft.body : doc.body, { wikilinks: doc.wikilinks }))
  const words = $derived(
    (tile.editing ? tile.draft.body : doc.body).split(/\s+/).filter(Boolean).length,
  )
  /** What is still to be done about it, and when the first of that is due. */
  const tasks = $derived(doc.cards.filter((card) => card.done_at === null))
  const due = $derived(tasks.reduce<number | null>((first, card) => (card.due_at !== null && (first === null || card.due_at < first) ? card.due_at : first), null))
  let reader = $state<HTMLElement>()
  let article = $state<HTMLElement>()
  let title = $state<HTMLInputElement>()

  // `:name` with nothing after it, while writing: up into the title.
  $effect(() => {
    const field = title
    if (!field || !focused) return
    // A turn later: vim gives the focus back to the text once its `:` line
    // has run, and that would land after this.
    const focus = () =>
      setTimeout(() => {
        field.focus()
        field.select()
      })
    hooks.focusTitle = focus
    return () => {
      if (hooks.focusTitle === focus) hooks.focusTitle = () => {}
    }
  })

  /*
   * j/k reach this pane through the window handler in App.svelte, which has no
   * way to know how tall a line is here — so the pane does the arithmetic and
   * says whether it scrolled anything at all.
   */
  $effect(() => {
    // A space switch remounts this pane, and the old one's cleanup can run
    // after the new one has set the hook — so it only clears its own.
    if (!focused) return
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

  // `]]`, `[[` and `gO` while reading; the editor answers them while writing.
  $effect(() => {
    const scroller = reader
    const root = article
    if (!scroller || !root || !focused) return
    const jump = (direction: 1 | -1) => jumpHeading(scroller, headingsIn(root), direction)
    const list = () => readerHeadings(scroller, root)
    // The outline in the margin is links too, so `F` reaches it as well.
    const hints = () => scroller
    const sideways = (amount: SidewaysAmount) => scrollWide(scroller, root, amount)
    hooks.jumpHeading = jump
    hooks.headings = list
    hooks.hintRoot = hints
    hooks.scrollWide = sideways
    return () => {
      if (hooks.jumpHeading === jump) hooks.jumpHeading = () => false
      if (hooks.headings === list) hooks.headings = () => ({ headings: [], current: 0 })
      if (hooks.hintRoot === hints) hooks.hintRoot = () => null
      if (hooks.scrollWide === sideways) hooks.scrollWide = () => false
    }
  })

  /*
   * A sealed file embedded here will not load while the vault is shut: the
   * server answers 423 and the browser shows a broken image. Errors do not
   * bubble, so they are caught on the way down, and the broken embed is
   * swapped for a button that opens the vault and puts it back.
   */
  $effect(() => {
    const root = article
    if (!root) return
    const sealed = async (event: Event) => {
      const element = event.target as HTMLElement
      const media = element.closest('img, video, audio') as HTMLImageElement | HTMLMediaElement | null
      if (!media || !(media.getAttribute('src') ?? media.querySelector('source')?.getAttribute('src') ?? '').includes('/api/media/')) return
      await workspace.loadVault()
      if (!workspace.vault?.configured || workspace.vault.unlocked || !media.isConnected) return
      const button = document.createElement('button')
      button.className = 'sealed-embed'
      button.textContent = 'sealed — unlock to show'
      button.onclick = async (click) => {
        click.preventDefault()
        click.stopPropagation()
        if (!(await workspace.ensureUnlocked())) return
        button.replaceWith(media)
        if (media instanceof HTMLImageElement) media.src = media.src
        else media.load()
      }
      media.replaceWith(button)
    }
    root.addEventListener('error', sealed, true)
    return () => root.removeEventListener('error', sealed, true)
  })

  /**
   * `/n/<space>/<slug>` links — the ones the importer writes between imported
   * documents — are followed in place rather than reloading the application.
   */
  function follow(event: MouseEvent) {
    if (event.metaKey || event.ctrlKey || event.shiftKey || event.button !== 0) return
    const anchor = (event.target as HTMLElement | null)?.closest('a')
    const href = anchor?.getAttribute('href') ?? ''
    if (!href.startsWith('/n/')) return
    event.preventDefault()
    followLink(href)
  }
</script>

<section class="pane">
  <header>
    <div class="crumb label rule" title="{slugPath(doc.path)}">
      <!-- A phone has no crumbs on the bar and no h key: this is the way back. -->
      <button class="back" aria-label="back to the list" onclick={() => runCommand('q')}>
        <Icon name="back" size={16} />
      </button>
      <span class="marker" class:live={tile.editing}></span>
      <span class="path">{slugPath(doc.path)}</span>
      {#if tile.dirty}<span class="dirty">modified</span>{/if}
    </div>

    <!--
      The readout is the way to everything else about the document: its
      headings, its history, what links to it and who can read it — gi.
    -->
    <button class="meta" title="info (gi)" onclick={() => openInfo()}>
      <span class="field"><span class="caption">written</span>{formatStamp(tile.draft.created_at)} {formatWeekday(tile.draft.created_at)}</span>
      <span class="field"><span class="caption">edited</span>{relative(doc.updated_at)}</span>
      <span class="field"><span class="caption">words</span>{words}<span class="unit">words</span></span>
      {#if doc.backlinks > 0}
        <span class="field"><span class="caption">linked</span>from {doc.backlinks}</span>
      {/if}
      {#if doc.shared}
        <span class="field public"><span class="ring"></span>public</span>
      {/if}
    </button>
    <!-- The work about it, in a line: what is still to do here — gc lists it. -->
    {#if tasks.length > 0}
      <button class="meta tasks" title="cards (gc)" onclick={() => openInfo('cards')}>
        <span class="field">
          <span class="caption">{tasks.length === 1 ? 'card' : 'cards'}</span>
          {#if tasks.length === 1}<span class="what">{tasks[0].title}</span> · {tasks[0].list_name}{:else}{tasks.length} open{/if}
        </span>
        {#if due !== null}
          <span class="field" class:late={due < Date.now() / 1000}><span class="caption">due</span>{formatWeekday(due)} {formatDay(due)}</span>
        {/if}
      </button>
    {/if}
  </header>

  {#if tile.editing}
    <input
      class="title"
      bind:this={title}
      placeholder="Untitled"
      bind:value={tile.draft.name}
      oninput={() => workspace.touch(tile)}
      onkeydown={(event) => {
        // The title is the line above the text: any of these goes back down.
        if (['Enter', 'Escape', 'ArrowDown', 'Tab'].includes(event.key) && !event.shiftKey) {
          event.preventDefault()
          hooks.focusEditor()
        }
      }}
    />
    <div class="body">
      {#key doc.id}
        {#await import('./Editor.svelte')}
          <div class="loading label">loading editor<span class="cursor">_</span></div>
        {:then module}
          {@const Editor = module.default}
          <Editor {tile} />
        {/await}
      {/key}
    </div>
    {#if isTouchDevice}
      <div class="mobilebar">
        <button onclick={() => runCommand('upload')}>+ media</button>
        <button onclick={() => runCommand('w')}>save</button>
        <button onclick={() => runCommand('wq')}>done</button>
      </div>
    {/if}
  {:else}
    <div class="body reading" bind:this={reader}>
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
      <article class="md resolve" bind:this={article} onclick={follow}>
        {#if doc.name.trim()}<h1 class="entrytitle">{doc.name}</h1>{/if}
        <!-- eslint-disable-next-line svelte/no-at-html-tags -- sanitised in renderMarkdown -->
        {@html html}
        {#if !doc.body.trim()}
          <p class="muted">this document is empty — press <span class="key">i</span> to write.</p>
        {/if}
      </article>
      <aside class="side">
        <Outline root={article} scroller={reader} source={html} />
      </aside>
    </div>
    <!-- A thumb has no i and no gi. -->
    {#if isTouchDevice}
      <div class="mobilebar">
        <button onclick={() => runCommand('e')}>edit</button>
        <button onclick={() => openInfo()}>info</button>
      </div>
    {/if}
  {/if}
</section>

<style>
  .pane :global(.sealed-embed) {
    display: block;
    margin: 0.5rem 0;
    padding: 1.5rem;
    width: 100%;
    border: 1px dashed currentColor;
    background: none;
    color: inherit;
    opacity: 0.7;
    font: inherit;
    cursor: pointer;
  }

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
  .crumb .marker.live { background: var(--light); }
  .path {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--subtle);
    letter-spacing: 0.08em;
    text-transform: none;
  }
  .dirty { flex: none; color: var(--light); }

  /* Read as a line of the header, and a way into the info sheet. */
  .meta {
    all: unset;
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 4px 18px;
    min-width: 0;
    font-size: 11px;
    color: var(--fg);
    cursor: pointer;
  }
  .meta:hover .caption, .meta:focus-visible .caption { color: var(--light); }
  .caption { margin-right: 8px; color: var(--muted); letter-spacing: 0.18em; text-transform: uppercase; }
  .public { display: flex; align-items: center; gap: 6px; color: var(--light); }
  .tasks .what { color: var(--light); }
  .tasks .late { color: var(--alarm); }
  .ring { width: 7px; height: 7px; flex: none; border: 1.5px solid var(--light); border-radius: 999px; }

  .title {
    flex: none;
    margin: 8px 24px 4px 28px;
    padding: 2px 0 6px;
    background: transparent;
    border: 0;
    border-bottom: 1px solid var(--overlay);
    border-radius: 0;
    color: var(--light);
    font-family: var(--font-sans);
    font-size: 24px;
    font-weight: 300;
  }
  .title:focus { border-bottom-color: var(--light-dim); }

  .loading { padding: 16px 28px; }

  .body { flex: 1; min-height: 0; overflow: hidden; }
  /*
   * The text is a column in the middle of the pane, as on a page; the margin
   * to its right holds the outline while there is room for one.
   */
  .reading {
    overflow-y: auto;
    padding: 18px 28px 30vh;
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 46rem) minmax(0, 1fr);
    align-items: start;
    container-type: inline-size;
  }
  .reading > article { grid-column: 2; max-width: none; }
  .side {
    grid-column: 3;
    position: sticky;
    top: 0;
    max-width: 30ch;
    padding: 6px 0 0 32px;
  }
  @container (max-width: 1000px) {
    .side { display: none; }
  }
  .entrytitle { margin-top: 0 !important; font-size: 2.1em !important; }

  .key {
    font-family: var(--font-mono);
    font-size: 12px;
    padding: 0 5px;
    border-radius: 4px;
    background: var(--light);
    color: var(--ink);
  }

  .mobilebar {
    display: flex;
    gap: 6px;
    padding: 8px 12px calc(8px + env(safe-area-inset-bottom));
    border-top: 1px solid var(--rim);
    flex: none;
  }
  .mobilebar button { flex: 1; padding: 8px; }

  .back { display: none; }
  .unit { display: none; }

  /*
   * A phone: a back button where the bar's crumbs would be, and the readout
   * on one line without its captions. Closing is the back button's job.
   */
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
    .meta { flex-wrap: nowrap; gap: 0; padding-left: 6px; overflow: hidden; white-space: nowrap; }
    .caption { display: none; }
    .field + .field::before { content: '·'; margin: 0 8px; color: var(--muted); }
    .unit { display: inline; margin-left: 0.5ch; }
    .title { margin: 8px 16px 4px; }
    .reading { padding: 14px 16px 30vh; grid-template-columns: minmax(0, 1fr); }
    .reading > article { grid-column: 1; }
  }
</style>
