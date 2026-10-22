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
    lock: ['M6 11h12a1 1 0 0 1 1 1v8a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1v-8a1 1 0 0 1 1-1z', 'M8 11V8a4 4 0 0 1 8 0v3'],
    help: [
      'M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18z',
      'M9.5 9.5a2.5 2.5 0 1 1 3.5 2.3c-.6.3-1 .9-1 1.6v.6',
      'M12 17h.01',
    ],
    back: ['M15 5l-7 7 7 7'],
    left: ['M19 12H5', 'M11 6l-6 6 6 6'],
    right: ['M5 12h14', 'M13 6l6 6-6 6'],
    check: ['M5 12.5l4.5 4.5L19 7'],
    calendar: ['M5 6h14a1 1 0 0 1 1 1v12a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1z', 'M4 10h16', 'M8 4v4', 'M16 4v4'],
    doc: ['M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z', 'M14 3v5h5', 'M9 13h6', 'M9 17h4'],
    agenda: ['M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18z', 'M12 7v5l3 2'],
    capture: ['M12 5v14', 'M5 12h14'],
    upload: ['M12 15V4', 'M7 9l5-5 5 5', 'M5 15v3a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-3'],
    'search-all': ['M10 4a6 6 0 1 0 0 12a6 6 0 1 0 0-12z', 'M19 19l-4.5-4.5', 'M17 4h4', 'M19 2v4'],
    today: ['M5 6h14a1 1 0 0 1 1 1v12a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1z', 'M4 10h16', 'M12 13v4', 'M10 15h4'],
    up: ['M12 19V5', 'M6 11l6-6 6 6'],
    down: ['M12 5v14', 'M6 13l6 6 6-6'],
    drop: ['M12 4v11', 'M7 10l5 5 5-5', 'M5 20h14'],
    rename: ['M4 20h4L19 9l-4-4L4 16z', 'M13 7l4 4'],
    repeat: ['M17 3l3 3-3 3', 'M4 11V9a3 3 0 0 1 3-3h13', 'M7 21l-3-3 3-3', 'M20 13v2a3 3 0 0 1-3 3H4'],
    trash: ['M4 7h16', 'M9 7V4h6v3', 'M6 7l1 13h10l1-13'],
    more: ['M5 12h.01', 'M12 12h.01', 'M19 12h.01'],
    settings: ['M4 6h9', 'M17 6h3', 'M15 4v4', 'M4 12h3', 'M11 12h9', 'M9 10v4', 'M4 18h11', 'M19 18h1', 'M17 16v4'],
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
