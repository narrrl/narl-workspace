<script lang="ts">
  /*
   * The footer of a panel: the keys its contexts read, straight from the
   * keymap — each one with a hint, in the table's order — and whatever the
   * panel adds after them. A phone has no keys to press, so there it shows
   * only what the panel adds.
   */
  import type { Snippet } from 'svelte'
  import { hints, type Context } from './keymap'
  import { isTouchDevice } from './util'

  const {
    contexts,
    hide = [],
    children,
  }: {
    contexts: Context[]
    /** Actions whose hint is left out while they would do nothing. */
    hide?: string[]
    children?: Snippet
  } = $props()

  const shown = $derived(isTouchDevice ? [] : hints(contexts).filter((entry) => !entry.action || !hide.includes(entry.action)))
</script>

{#if shown.length || children}
  <footer class="label">
    {#each shown as entry, index (index)}
      <span class="hint">{#each entry.caps as cap (cap)}<span class="kbd">{cap}</span>{/each} {entry.hint}</span>
    {/each}
    {@render children?.()}
  </footer>
{/if}

<style>
  footer { display: flex; flex-wrap: wrap; align-items: center; gap: 6px 14px; font-size: 10px; }
  .hint { display: flex; gap: 3px; align-items: center; }

  @media (max-width: 767px) {
    .hint { display: none; }
    footer:not(:has(> :not(.hint))) { display: none; }
  }
</style>
