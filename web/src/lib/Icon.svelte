<script lang="ts" module>
  /*
   * Line icons for the rail and the phone dock, drawn on a 24-unit grid with
   * one stroke weight, so they read as one set the way the desktop's Nerd Font
   * glyphs do. Paths rather than a font: there are a dozen of them.
   */
  const PATHS = {
    'doc-plus': ['M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z', 'M14 3v5h5', 'M12 11v6', 'M9 14h6'],
    'folder-plus': ['M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z', 'M12 10v6', 'M9 13h6'],
    board: [
      'M4 5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1z',
      'M14 5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v8a1 1 0 0 1-1 1h-4a1 1 0 0 1-1-1z',
    ],
    'column-plus': ['M4 5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1z', 'M17 8v8', 'M13 12h8'],
    list: ['M9 6h11', 'M9 12h11', 'M9 18h11', 'M4.5 6h.01', 'M4.5 12h.01', 'M4.5 18h.01'],
    grid: ['M4 4h6v6H4z', 'M14 4h6v6h-6z', 'M4 14h6v6H4z', 'M14 14h6v6h-6z'],
    expand: ['M4 9V4h5', 'M20 9V4h-5', 'M4 15v5h5', 'M20 15v5h-5'],
    shrink: ['M9 4v5H4', 'M15 4v5h5', 'M9 20v-5H4', 'M15 20v-5h5'],
    search: ['M11 4a7 7 0 1 0 0 14a7 7 0 1 0 0-14z', 'M20 20l-4-4'],
    prompt: ['M5 7l5 5-5 5', 'M12 18h7'],
    help: [
      'M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18z',
      'M9.5 9.5a2.5 2.5 0 1 1 3.5 2.3c-.6.3-1 .9-1 1.6v.6',
      'M12 17h.01',
    ],
    back: ['M15 5l-7 7 7 7'],
  } satisfies Record<string, string[]>

  export type IconName = keyof typeof PATHS
</script>

<script lang="ts">
  const { name, size = 18 }: { name: IconName; size?: number } = $props()
</script>

<svg
  class="icon"
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.5"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  {#each PATHS[name] as d (d)}<path {d} />{/each}
</svg>

<style>
  .icon { display: block; flex: none; }
</style>
