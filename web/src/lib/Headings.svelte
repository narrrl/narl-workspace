<script lang="ts">
  /*
   * `gO`: the headings of what is on screen, as a list to walk — the outline
   * in the margin, for when there is no margin, or no pointer to click it
   * with. The keys live in App.svelte with the other overlays'.
   */
  import Keys from './Keys.svelte'
  import { workspace } from './store.svelte'

  let sheet = $state<HTMLDivElement>()

  $effect(() => {
    const index = workspace.outlineCursor
    sheet?.querySelector<HTMLElement>(`[data-index="${index}"]`)?.scrollIntoView({ block: 'nearest' })
  })

  // The shallowest level in the document is the left edge, so a text that
  // starts at `##` is not indented by a level it never uses.
  const top = $derived(Math.min(...workspace.outline.map((heading) => heading.level)))

  function go(index: number) {
    const heading = workspace.outline[index]
    workspace.overlay = 'none'
    heading?.go()
  }
</script>

<div class="sheet" bind:this={sheet}>
  <header>
    <span class="label rule"><span class="marker"></span><span class="title">outline</span> {workspace.outline.length} headings</span>
    <button onclick={() => (workspace.overlay = 'none')}>esc</button>
  </header>

  <ol>
    {#each workspace.outline as heading, index (index)}
      <li style="--depth: {heading.level - top}">
        <button
          class="row"
          class:at={index === workspace.outlineCursor}
          data-index={index}
          onclick={() => go(index)}
          onpointerenter={() => (workspace.outlineCursor = index)}
        >{heading.text}</button>
      </li>
    {/each}
  </ol>

  <Keys contexts={['outline', 'sheet']} />
</div>

<style>
  .sheet {
    display: flex;
    flex-direction: column;
    gap: 16px;
    width: min(100%, 64ch);
    max-height: 100%;
    overflow-y: auto;
    background: color-mix(in oklab, var(--hull) 97%, transparent);
    border: 1px solid var(--surface-alt);
    border-radius: var(--frame-radius);
    padding: 20px 24px;
  }

  header { display: flex; align-items: center; gap: 12px; }
  header .label { flex: 1; }
  header .marker { background: var(--light); }
  header .title { color: var(--light); }

  ol { list-style: none; margin: 0; padding: 0; }
  li { padding-left: calc(var(--depth) * 18px); }

  .row {
    display: block;
    width: 100%;
    margin: 1px 0;
    padding: 6px 10px;
    border: 0;
    border-radius: var(--radius-s);
    text-align: left;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-sans);
    font-size: 14px;
    letter-spacing: 0;
    text-transform: none;
    color: var(--fg);
  }
  .row:hover { background: var(--surface-alt); border-color: transparent; }
  .row:active { transform: none; }
  .row.at { background: var(--light); color: var(--ink); }


  @media (max-width: 767px) {
    .sheet { padding: 14px; gap: 14px; }
  }
</style>
