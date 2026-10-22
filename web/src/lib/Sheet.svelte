<script lang="ts">
  /*
   * A sheet: a panel floated over the tiles, as the desktop floats one —
   * hull, one rim, the frame's radius. Every overlay is drawn in one: its
   * title and what it is about, the buttons beside them, a body, and the
   * footer of keys its contexts read.
   *
   * A sheet with a list keeps the row under its cursor on screen: the rows
   * carry `data-index`, and the sheet scrolls to the one at `cursor`. Rows
   * drawn as `.item` look the same on every sheet; `.at` is the one under
   * the cursor.
   */
  import type { Snippet } from 'svelte'
  import Keys from './Keys.svelte'
  import type { Context } from './keymap'
  import { workspace } from './store.svelte'

  const {
    title,
    detail = '',
    width = 92,
    fill = false,
    cursor,
    keys = [],
    hide,
    actions,
    footer,
    children,
  }: {
    title: string
    detail?: string
    /** How wide it may grow, in characters; 0 is as wide as the screen. */
    width?: number
    /** As tall as the screen, for a body that scrolls on its own. */
    fill?: boolean
    /** The row to keep on screen. */
    cursor?: number
    keys?: Context[]
    hide?: string[]
    /** Buttons in the header, before the close one. */
    actions?: Snippet
    /** Added to the footer after the keys. */
    footer?: Snippet
    children: Snippet
  } = $props()

  let root = $state<HTMLDivElement>()

  $effect(() => {
    const index = cursor
    if (index === undefined) return
    root?.querySelector<HTMLElement>(`[data-index="${index}"]`)?.scrollIntoView({ block: 'nearest' })
  })
</script>

<div class="sheet" class:fill style:--width={width ? `${width}ch` : '100%'} bind:this={root}>
  <header>
    <span class="label rule"><span class="marker"></span><span class="title">{title}</span> {detail}</span>
    {@render actions?.()}
    <button onclick={() => workspace.closeSheet()}>esc</button>
  </header>

  {@render children()}

  {#if keys.length || footer}
    <Keys contexts={keys} {hide} children={footer} />
  {/if}
</div>

<style>
  .sheet {
    display: flex;
    flex-direction: column;
    gap: 18px;
    width: min(100%, var(--width));
    max-height: 100%;
    min-height: 0;
    overflow: hidden;
    background: color-mix(in oklab, var(--hull) 97%, transparent);
    border: 1px solid var(--surface-alt);
    border-radius: var(--frame-radius);
    padding: 20px 24px;
    animation: resolve var(--t-slow) var(--ease-snap) both;
  }
  .sheet.fill { height: 100%; }

  header { display: flex; align-items: center; gap: 12px; min-width: 0; }
  header .label { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  header .marker { background: var(--light); }
  header .title { color: var(--light); }

  /* The list a sheet walks, and its rows. */
  .sheet :global(.items) {
    list-style: none;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .sheet :global(.item) {
    display: flex;
    flex-direction: column;
    padding: 6px 8px;
    border-radius: var(--radius-m);
    cursor: pointer;
    transition: background-color var(--t-base) var(--ease-glide);
  }
  .sheet :global(.item:hover) { background: var(--surface); }
  .sheet :global(.item.at) { background: var(--surface-alt); box-shadow: inset 2px 0 0 var(--light); }
  .sheet :global(.item .name) {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--fg);
    font-family: var(--font-sans);
    font-size: 14px;
  }
  .sheet :global(.item .muted) { font-size: 11px; }
  .sheet :global(.danger:hover) { background: var(--alarm); border-color: var(--alarm); color: var(--ink); }

  /* The parts of a sheet with several, one button each — info, settings. */
  .sheet :global(.tabs) { display: flex; flex-wrap: wrap; gap: 4px; flex: none; }
  .sheet :global(.tabs button) { text-transform: none; letter-spacing: 0; }
  .sheet :global(.tabs .on) { background: var(--light); border-color: var(--light); color: var(--ink); }

  @media (max-width: 767px) {
    .sheet { padding: 14px; gap: 14px; }
  }
</style>
