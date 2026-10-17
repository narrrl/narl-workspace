<script lang="ts">
  /*
   * The right edge of the frame: the things a pointer needs a door for. Each
   * door is labelled with the key that does the same thing, so the rail
   * teaches the keymap instead of replacing it. `compact` lays the same doors
   * out in a row, for the bottom band on a phone.
   */
  import { hooks, runCommand } from './commands'
  import { workspace } from './store.svelte'

  const { compact = false }: { compact?: boolean } = $props()

  type Door = { key: string; caption: string; title: string; run: () => unknown; on?: boolean }

  const doors = $derived.by((): Door[] => {
    if (workspace.view === 'board') {
      return [
        { key: 'o', caption: 'card', title: 'new card on this list (o)', run: () => hooks.openCommandLine(':card ') },
        { key: '+', caption: 'list', title: 'new list (:list <name>)', run: () => hooks.openCommandLine(':list ') },
        { key: 'b', caption: 'tree', title: 'back to the tree (b, q)', run: () => runCommand('board'), on: true },
        { key: ':', caption: 'cmd', title: 'command line (:)', run: () => hooks.openCommandLine(':') },
        { key: '?', caption: 'help', title: 'help (?)', run: () => runCommand('help') },
      ]
    }
    const board: Door = workspace.space?.has_board
      ? { key: 'b', caption: 'board', title: 'board (b)', run: () => runCommand('board') }
      : { key: '+b', caption: 'board', title: 'give this space a board (:set board)', run: () => runCommand('set board') }
    return [
      { key: 'o', caption: 'new', title: 'new document (o)', run: () => runCommand('new') },
      { key: 'O', caption: 'dir', title: 'new folder (O)', run: () => hooks.openCommandLine(':mkdir ') },
      ...(workspace.space ? [board] : []),
      { key: 'w', caption: 'spaces', title: 'overview of every space (w)', run: () => hooks.openOverview() },
      { key: 'f', caption: 'full', title: 'fullscreen the focused pane (f)', run: () => (workspace.fullscreen = !workspace.fullscreen), on: workspace.fullscreen },
      { key: '/', caption: 'find', title: 'search (/)', run: () => hooks.openCommandLine('/') },
      { key: ':', caption: 'cmd', title: 'command line (:)', run: () => hooks.openCommandLine(':') },
      { key: '?', caption: 'help', title: 'help (?)', run: () => runCommand('help') },
    ]
  })
</script>

<nav class="rail" class:compact aria-label="tools">
  {#each doors as door (door.caption)}
    <button class="door" class:on={door.on} title={door.title} onclick={door.run}>
      <span class="key">{door.key}</span>
      <span class="caption">{door.caption}</span>
    </button>
  {/each}
</nav>

<style>
  .rail {
    position: absolute;
    top: var(--bar-h);
    right: 0;
    bottom: var(--status-h);
    width: var(--rail-w);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    padding: 6px 0;
  }

  .door {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    width: 40px;
    padding: 7px 0 6px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--muted);
    letter-spacing: 0;
    text-transform: none;
  }

  /* A light bar grows down the left edge on hover, as on the desktop rail. */
  .door::before {
    content: '';
    position: absolute;
    left: 0;
    top: 6px;
    bottom: 6px;
    width: 2px;
    border-radius: 2px;
    background: var(--light);
    transform: scaleY(0);
    transform-origin: top;
    transition: transform var(--t-base) var(--ease-snap);
  }
  .door:hover { background: var(--surface-alt); color: var(--light); }
  .door:hover::before { transform: scaleY(1); }
  .door.on { color: var(--light); }
  .door:focus-visible { background: var(--light); color: var(--ink); }

  .key {
    font-size: 15px;
    line-height: 1;
    transition: transform var(--t-base) var(--ease-snap);
  }
  .door:hover .key { transform: scale(1.08); }
  .caption {
    font-size: 9px;
    font-weight: 600;
    letter-spacing: 0.14em;
    text-transform: uppercase;
  }

  @media (max-width: 767px) {
    .rail:not(.compact) { display: none; }
  }

  .compact {
    position: static;
    width: auto;
    flex-direction: row;
    padding: 0;
    gap: 2px;
  }
  .compact .door { width: 38px; padding: 4px 0; }
  .compact .door::before { display: none; }
  .compact .caption { display: none; }
</style>
