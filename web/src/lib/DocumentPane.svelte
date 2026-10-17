<script lang="ts">
  import { hooks, runCommand, type ScrollAmount } from './commands'
  import { renderMarkdown } from './markdown'
  import { workspace } from './store.svelte'
  import { formatStamp, formatWeekday, isTouchDevice, relative } from './util'

  const doc = $derived(workspace.open!)
  const html = $derived(renderMarkdown(workspace.editing ? workspace.draft.body : doc.body))
  const words = $derived(
    (workspace.editing ? workspace.draft.body : doc.body).split(/\s+/).filter(Boolean).length,
  )
  /*
   * Only the token half of a share link is recoverable — the key lives in the
   * fragment, and the server kept nothing but its hash — so the bar shows the
   * link's shape and offers a fresh one rather than a copy of the old.
   */
  const sharePath = $derived(doc.share_token ? `${location.origin}/s/${doc.share_token}#…` : null)

  let reader = $state<HTMLElement>()

  /*
   * j/k reach this pane through the window handler in App.svelte, which has no
   * way to know how tall a line is here — so the pane does the arithmetic and
   * says whether it scrolled anything at all.
   */
  $effect(() => {
    hooks.scrollReader = (amount: ScrollAmount) => {
      if (!reader) return false
      const line = parseFloat(getComputedStyle(reader).lineHeight) || 24
      const half = reader.clientHeight / 2
      switch (amount) {
        case 'top':
          reader.scrollTo({ top: 0 })
          break
        case 'bottom':
          reader.scrollTo({ top: reader.scrollHeight })
          break
        case 'halfdown':
          reader.scrollBy({ top: half })
          break
        case 'halfup':
          reader.scrollBy({ top: -half })
          break
        default:
          reader.scrollBy({ top: amount * line * 3 })
      }
      return true
    }
    return () => (hooks.scrollReader = () => false)
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
    const path = href.slice(3).split('#')[0]
    void workspace.guard(() => workspace.openPath(decodeURIComponent(path)))
  }
</script>

<section class="pane">
  <header>
    <div class="crumb label rule" title="{workspace.here}/{doc.slug}">
      <span class="marker" class:live={workspace.editing}></span>
      <span class="path">{workspace.here}/{doc.slug}</span>
      {#if workspace.dirty}<span class="dirty">modified</span>{/if}
    </div>

    <div class="row">
      <dl class="meta">
        <dt>written</dt>
        <dd>{formatStamp(workspace.draft.created_at)} {formatWeekday(workspace.draft.created_at)}</dd>
        <dt>edited</dt>
        <dd>{relative(doc.updated_at)}</dd>
        <dt>words</dt>
        <dd>{words}</dd>
      </dl>

      <div class="actions">
        {#if isTouchDevice}
          <button onclick={() => runCommand('q')}>← list</button>
        {/if}
        {#if workspace.editing}
          <button onclick={() => runCommand('w')}>:w</button>
          <button onclick={() => runCommand('upload')}>attach</button>
          <button onclick={() => runCommand('wq')}>done</button>
        {:else}
          <button onclick={() => runCommand('e')}>edit</button>
        {/if}
        <button class:on={doc.shared} onclick={() => runCommand(doc.shared ? 'unshare' : 'share')}>
          {doc.shared ? 'unshare' : 'share'}
        </button>
        <button class="danger" onclick={() => runCommand('d')}>delete</button>
        <button onclick={() => runCommand('q')}>close</button>
      </div>
    </div>
  </header>

  {#if sharePath}
    <div class="sharebar">
      <span class="ring"></span>
      <span class="label public">public</span>
      <span class="link">{sharePath}</span>
      <button onclick={() => runCommand('link')}>new link</button>
    </div>
  {/if}

  {#if workspace.editing}
    <input
      class="title"
      placeholder="Untitled"
      bind:value={workspace.draft.name}
      oninput={() => workspace.touch()}
      onkeydown={(event) => {
        if (event.key === 'Enter') {
          event.preventDefault()
          document.querySelector<HTMLElement>('.cm-content')?.focus()
        }
      }}
    />
    <div class="body">
      {#key doc.id}
        {#await import('./Editor.svelte')}
          <div class="loading label">loading editor<span class="cursor">_</span></div>
        {:then module}
          {@const Editor = module.default}
          <Editor />
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
      <article class="md resolve" onclick={follow}>
        {#if doc.name.trim()}<h1 class="entrytitle">{doc.name}</h1>{/if}
        <!-- eslint-disable-next-line svelte/no-at-html-tags -- sanitised in renderMarkdown -->
        {@html html}
        {#if !doc.body.trim()}
          <p class="muted">this document is empty — press <span class="key">i</span> to write.</p>
        {/if}
      </article>
    </div>
  {/if}
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
  .actions .on { color: var(--light); border-color: var(--light-dim); }
  .actions .danger:hover { background: var(--alarm); border-color: var(--alarm); color: var(--ink); }

  .sharebar {
    display: flex;
    gap: 10px;
    align-items: center;
    margin: 0 24px 8px 28px;
    padding: 6px 6px 6px 12px;
    border: 1px solid var(--rim);
    border-radius: var(--radius-m);
    background: var(--surface);
    font-size: 12px;
    overflow-x: auto;
    white-space: nowrap;
    flex: none;
  }
  .ring {
    width: 8px;
    height: 8px;
    flex: none;
    border: 1.5px solid var(--light);
    border-radius: 999px;
  }
  .public { color: var(--light); }
  .sharebar .link { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; color: var(--muted); }

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
  .reading { overflow-y: auto; padding: 18px 28px 30vh; }
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

  @media (max-width: 760px) {
    header { padding: 14px 16px 10px; }
    .row { flex-direction: column; align-items: flex-start; }
    .sharebar { margin: 0 16px 8px; }
    .title { margin: 8px 16px 4px; }
    .reading { padding: 14px 16px 30vh; }
  }
</style>
