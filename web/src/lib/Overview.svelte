<script lang="ts">
  /*
   * `w`: every space at once, the way the desktop's overview shows every
   * workspace with its windows — here, with what was last written in it. The
   * keys live in actions.ts with the others; this only draws and tells them
   * how many cards fit in a row.
   */
  import Sheet from './Sheet.svelte'
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

  function enter(spaceId: number) {
    workspace.sheets.close()
    void workspace.guard(() => workspace.tree.enterSpace(spaceId))
  }

  /** A recent document opens in place, in its own space. */
  function open(doc: NodeSummary) {
    workspace.sheets.close()
    void workspace.guard(async () => {
      const node = await api.getNode(doc.id)
      await workspace.tree.openPath(node.path.map((crumb) => crumb.slug).join('/'))
    })
  }
</script>

<Sheet title="overview" detail="{workspace.sheets.overview.length} spaces" width={140} cursor={workspace.sheets.overviewCursor} keys={['overview']}>
  <ol class="grid" bind:this={grid}>
    {#each workspace.sheets.overview as { space, recent }, index (space.id)}
      <li>
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
        <div
          class="card"
          class:at={index === workspace.sheets.overviewCursor}
          class:current={space.id === workspace.tree.spaceId}
          data-index={index}
          onclick={() => enter(space.id)}
          onpointerenter={() => (workspace.sheets.overviewCursor = index)}
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
</Sheet>

<style>
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
</style>
