<script lang="ts">
  /*
   * `w`: every space at once, the way the desktop's overview shows every
   * workspace with its windows — here, with what was last written in it. The
   * keys live in App.svelte with the others; this only draws and tells them
   * how many cards fit in a row.
   */
  import Keys from './Keys.svelte'
  import { api, type NodeSummary } from './api'
  import { hooks } from './commands'
  import { workspace } from './store.svelte'
  import { relative } from './util'

  let grid = $state<HTMLOListElement>()

  $effect(() => {
    hooks.overviewColumns = () =>
      grid ? getComputedStyle(grid).gridTemplateColumns.split(' ').length : 1
    return () => (hooks.overviewColumns = () => 1)
  })

  $effect(() => {
    const index = workspace.overviewCursor
    grid?.querySelector<HTMLElement>(`[data-index="${index}"]`)?.scrollIntoView({ block: 'nearest' })
  })

  function enter(spaceId: number) {
    workspace.overlay = 'none'
    void workspace.guard(() => workspace.enterSpace(spaceId))
  }

  /** A recent document opens in place, in its own space. */
  function open(doc: NodeSummary) {
    workspace.overlay = 'none'
    void workspace.guard(async () => {
      const node = await api.getNode(doc.id)
      await workspace.openPath(node.path.map((crumb) => crumb.slug).join('/'))
    })
  }
</script>

<div class="sheet">
  <header>
    <span class="label rule"><span class="marker"></span><span class="title">overview</span> {workspace.overview.length} spaces</span>
    <button onclick={() => (workspace.overlay = 'none')}>esc</button>
  </header>

  <ol class="grid" bind:this={grid}>
    {#each workspace.overview as { space, recent }, index (space.id)}
      <li>
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
        <div
          class="card"
          class:at={index === workspace.overviewCursor}
          class:current={space.id === workspace.spaceId}
          data-index={index}
          onclick={() => enter(space.id)}
          onpointerenter={() => (workspace.overviewCursor = index)}
        >
          <div class="top">
            <span class="index">{index < 10 ? (index + 1) % 10 : ''}</span>
            <span class="name">{space.name}</span>
            {#if space.has_board}<span class="tag">board</span>{/if}
            <span class="count">{space.child_count}</span>
          </div>

          <ul>
            {#each recent as doc (doc.id)}
              <li>
                <button
                  class="doc"
                  onclick={(event) => {
                    event.stopPropagation()
                    open(doc)
                  }}
                >
                  <span class="text">{doc.name.trim() || doc.excerpt.trim() || doc.slug}</span>
                  <span class="when">{relative(doc.updated_at)}</span>
                </button>
              </li>
            {:else}
              <li class="empty">nothing written yet</li>
            {/each}
          </ul>
        </div>
      </li>
    {/each}
  </ol>

  <Keys contexts={['overview']} />
</div>

<style>
  .sheet {
    display: flex;
    flex-direction: column;
    gap: 18px;
    width: min(100%, 140ch);
    max-height: 100%;
    background: color-mix(in oklab, var(--hull) 97%, transparent);
    border: 1px solid var(--surface-alt);
    border-radius: var(--frame-radius);
    padding: 20px 24px;
  }

  header { display: flex; align-items: center; gap: 12px; }
  header .label { flex: 1; }
  header .marker { background: var(--light); }
  header .title { color: var(--light); }

  .grid {
    list-style: none;
    margin: 0;
    padding: 2px;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(30ch, 1fr));
    gap: 10px;
    overflow-y: auto;
  }

  /*
   * A card is a miniature window: overlay border at rest, the active border
   * gradient under the cursor. The space being worked in is the inverted one,
   * as its pill is on the bar.
   */
  .card {
    display: flex;
    flex-direction: column;
    gap: 10px;
    height: 100%;
    padding: 12px 12px 10px;
    border: 1px solid transparent;
    border-radius: var(--radius-m);
    background:
      linear-gradient(var(--surface), var(--surface)) padding-box,
      linear-gradient(var(--overlay), var(--overlay)) border-box;
    cursor: pointer;
  }
  .card.at {
    background:
      linear-gradient(var(--surface), var(--surface)) padding-box,
      linear-gradient(135deg, var(--light), var(--light-dim)) border-box;
  }

  .top { display: flex; align-items: center; gap: 10px; }
  .index {
    display: grid;
    place-items: center;
    min-width: 22px;
    height: 20px;
    border-radius: 4px;
    font-size: 11px;
    color: var(--muted);
    background: var(--surface-alt);
  }
  .current .index { background: var(--light); color: var(--ink); }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-sans);
    font-size: 15px;
    color: var(--fg);
  }
  .at .name, .current .name { color: var(--light); }
  .tag, .count {
    font-size: 10px;
    letter-spacing: 0.18em;
    text-transform: uppercase;
    color: var(--muted);
  }
  .count { font-variant-numeric: tabular-nums; letter-spacing: 0; }

  ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 1px; }

  .doc {
    display: flex;
    align-items: baseline;
    gap: 10px;
    width: 100%;
    padding: 4px 8px;
    border: 0;
    border-radius: var(--radius-s);
    text-align: left;
    font-size: 13px;
    letter-spacing: 0;
    text-transform: none;
    color: var(--subtle);
  }
  .doc:active { transform: none; }
  .text {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-sans);
  }
  .when { flex: none; font-size: 10px; color: var(--muted); }
  .empty { padding: 4px 8px; font-size: 12px; color: var(--muted); }


  /* Keys mean nothing to a thumb. */
  @media (max-width: 767px) {
    .sheet { padding: 14px; gap: 14px; }
  }
</style>
