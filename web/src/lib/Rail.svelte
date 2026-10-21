<script lang="ts">
  /*
   * The right edge of the frame, built like the desktop's rail: a glyph per
   * door, groups split by a hairline, the always-there things pinned to the
   * bottom. The name and the key that does the same thing live in a tooltip,
   * so the rail stays narrow and still teaches the keymap. `dock` lays the
   * same doors out in a row for the bottom band on a phone, without the ones
   * that only make sense with a keyboard or two panes.
   */
  import type { NodeSummary } from './api'
  import { hooks, runCommand } from './commands'
  import Icon, { type IconName } from './Icon.svelte'
  import { workspace } from './store.svelte'

  const { dock = false }: { dock?: boolean } = $props()

  const dropping = (nodes: NodeSummary[]) => (nodes.length === 1 ? nodes[0].name.trim() || nodes[0].slug : `${nodes.length} nodes`)

  type Door = {
    icon: IconName
    label: string
    keys: string[]
    run: () => unknown
    on?: boolean
    dim?: boolean
    desk?: boolean // only on the desktop rail
  }

  const groups = $derived.by((): Door[][] => {
    const cmd: Door = { icon: 'prompt', label: 'Command line', keys: [':'], run: () => hooks.openCommandLine(':') }
    const help: Door = { icon: 'help', label: 'Help', keys: ['?'], run: () => runCommand('help') }

    if (workspace.view === 'special') {
      // The tiles of the special workspace are doors, so a phone — which
      // shows one at a time — can walk between them.
      const tiles: Door[] = [
        { icon: 'board', label: 'Board', keys: ['b'], run: () => (workspace.specialFocus = 'board'), on: workspace.specialFocus === 'board' },
        { icon: 'agenda', label: 'Agenda — late, due, in hand', keys: ['t'], run: () => (workspace.specialFocus = 'agenda'), on: workspace.specialFocus === 'agenda' },
        { icon: 'calendar', label: 'Calendar', keys: ['C'], run: () => (workspace.specialFocus = 'calendar'), on: workspace.specialFocus === 'calendar' },
        { icon: 'list', label: 'Back to the tree', keys: ['S', 'q'], run: () => workspace.closeSpecial() },
      ]
      if (workspace.specialFocus !== 'board') return [tiles, [cmd, help]]
      // The card verbs are doors too, so a phone — which has no H, L or
      // Space — can still move work along.
      const card = !!workspace.card
      return [
        tiles,
        [
          { icon: 'doc-plus', label: 'New card on this list', keys: ['o'], run: () => hooks.openCommandLine(':card ') },
          { icon: 'column-plus', label: 'New list', keys: [':list'], run: () => hooks.openCommandLine(':list ') },
        ],
        [
          { icon: 'left', label: 'Send the card left', keys: ['H'], run: () => workspace.guard(() => workspace.sendCard(-1)), dim: !card },
          { icon: 'check', label: 'Done, or back to the inbox', keys: ['Space'], run: () => workspace.guard(() => workspace.toggleDone()), dim: !card },
          { icon: 'right', label: 'Send the card right', keys: ['L'], run: () => workspace.guard(() => workspace.sendCard(1)), dim: !card },
          { icon: 'calendar', label: 'Due date', keys: ['D'], run: () => hooks.openCommandLine(':due '), dim: !card },
          { icon: 'repeat', label: 'Repeat once done', keys: [':repeat'], run: () => hooks.openCommandLine(':repeat '), dim: !card },
          { icon: 'doc', label: "The card's notes", keys: ['Enter'], run: () => workspace.guard(() => workspace.openCard()), dim: !card },
        ],
        [
          { icon: 'up', label: 'Card up its list', keys: ['K'], run: () => workspace.guard(() => workspace.nudgeCard(-1)), dim: !card },
          { icon: 'down', label: 'Card down its list', keys: ['J'], run: () => workspace.guard(() => workspace.nudgeCard(1)), dim: !card },
          {
            icon: 'rename',
            label: card ? 'Rename the card' : 'Rename the list',
            keys: ['R'],
            run: () => hooks.openCommandLine(card ? ':name ' : ':name! '),
          },
          {
            icon: 'trash',
            label: card ? 'Delete the card' : 'Delete the list',
            keys: card ? ['x'] : [':list!'],
            run: () => runCommand(card ? 'delete' : 'list!'),
          },
        ],
        [cmd, help],
      ]
    }

    const board: Door = workspace.space?.has_board
      ? { icon: 'board', label: 'Board', keys: ['b'], run: () => runCommand('board') }
      : { icon: 'board', label: 'Give this space a board', keys: [':set board'], run: () => runCommand('set board'), dim: true }
    return [
      [
        { icon: 'doc-plus', label: 'New document', keys: ['o'], run: () => runCommand('new') },
        { icon: 'folder-plus', label: 'New folder', keys: ['⇧ O'], run: () => hooks.openCommandLine(':mkdir ') },
        ...(workspace.space ? [board] : []),
      ],
      [
        { icon: 'agenda', label: 'Agenda — late, due, in hand', keys: ['t'], run: () => runCommand('agenda') },
        { icon: 'calendar', label: 'Calendar — the diary by month', keys: ['C'], run: () => runCommand('cal') },
        { icon: 'today', label: "Today's diary entry", keys: ['T'], run: () => runCommand('today') },
        { icon: 'capture', label: 'Capture a card to the inbox', keys: ['c'], run: () => hooks.openCommandLine(':card ') },
      ],
      [
        { icon: 'upload', label: 'Upload files into this folder', keys: [':upload'], run: () => hooks.pickTreeFiles(false) },
        { icon: 'trash', label: 'The trash — what was deleted', keys: [':trash'], run: () => runCommand('trash') },
        ...(workspace.marked.length
          ? [{ icon: 'drop' as const, label: `Drop ${dropping(workspace.marked)} here`, keys: ['p'], run: () => workspace.guard(() => workspace.drop()), on: true }]
          : []),
      ],
      [
        { icon: 'grid', label: 'Every space', keys: ['w'], run: () => hooks.openOverview() },
        {
          icon: workspace.fullscreen ? 'shrink' : 'expand',
          label: workspace.fullscreen ? 'Leave fullscreen' : 'Fullscreen the focused pane',
          keys: ['f'],
          run: () => (workspace.fullscreen = !workspace.fullscreen),
          on: workspace.fullscreen,
          desk: true,
        },
        { icon: 'search', label: 'Search this space', keys: ['/'], run: () => hooks.openCommandLine('/') },
        { icon: 'search-all', label: 'Search every space', keys: ['Ctrl-P'], run: () => hooks.openGlobalSearch() },
      ],
      [cmd, help],
    ]
  })

  const shown = $derived(dock ? [groups.flat().filter((door) => !door.desk)] : groups)
</script>

<nav class="rail" class:dock aria-label="tools">
  {#each shown as group, index (index)}
    <div class="group" class:pinned={index === shown.length - 1 && !dock}>
      {#each group as door (door.label)}
        <button
          class="door"
          class:on={door.on}
          class:dim={door.dim}
          aria-label="{door.label} ({door.keys.join(', ')})"
          onclick={door.run}
        >
          <Icon name={door.icon} />
          {#if !dock}
            <span class="tip" role="tooltip">
              <span class="name">{door.label}</span>
              {#each door.keys as key (key)}<span class="kbd">{key}</span>{/each}
            </span>
          {/if}
        </button>
      {/each}
    </div>
  {/each}
</nav>

<style>
  .rail {
    position: absolute;
    top: var(--bar-h);
    right: 0;
    bottom: var(--status-h);
    width: var(--rail-w);
    z-index: 10;
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 6px 0;
  }

  .group {
    display: flex;
    flex-direction: column;
    align-items: center;
  }
  /* A 16px hairline between groups, as on the desktop rail. */
  .group + .group::before {
    content: '';
    width: 16px;
    height: 1px;
    margin: 8px 0;
    background: color-mix(in oklab, var(--outline) 60%, transparent);
  }
  .group.pinned { margin-top: auto; }
  .group.pinned::before { display: none; }

  .door {
    position: relative;
    display: grid;
    place-items: center;
    width: var(--rail-w);
    height: 34px;
    padding: 0;
    border: 0;
    border-radius: 0;
    background: transparent;
    color: var(--light-soft);
  }

  /* The wipe down the leading edge on hover, held off the edge as on the desktop. */
  .door::before {
    content: '';
    position: absolute;
    left: 5px;
    top: 50%;
    width: 2px;
    height: 16px;
    margin-top: -8px;
    border-radius: 1px;
    background: var(--light);
    transform: scaleY(0);
    transition: transform var(--t-base) var(--ease-snap);
  }
  .door:hover { background: transparent; color: var(--light); }
  .door:hover::before, .door:focus-visible::before { transform: scaleY(1); }
  .door:active { transform: none; }
  .door :global(.icon) { transition: transform var(--t-base) var(--ease-snap); }
  .door:hover :global(.icon) { transform: scale(1.08); }
  .door.on { color: var(--light); }
  .door.on::before { transform: scaleY(1); background: var(--light-dim); }
  .door.dim { color: var(--muted); }
  .door:focus-visible { background: transparent; color: var(--light); }

  /*
   * The tooltip opens into the porthole, to the left of the door: a small
   * card with the name in the reading face and the key as a keycap. A short
   * delay on the way in, so running the pointer down the rail does not flash
   * every one of them.
   */
  .tip {
    position: absolute;
    right: calc(100% + 4px);
    top: 50%;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px 6px 10px;
    border: 1px solid var(--overlay);
    border-radius: var(--radius-s);
    background: var(--surface);
    box-shadow: 0 6px 24px rgb(0 0 0 / 0.45);
    white-space: nowrap;
    pointer-events: none;
    opacity: 0;
    transform: translate(4px, -50%);
    transition:
      opacity var(--t-fast) var(--ease-glide),
      transform var(--t-fast) var(--ease-glide);
  }
  .door:hover .tip, .door:focus-visible .tip {
    opacity: 1;
    transform: translate(0, -50%);
    transition-delay: 250ms;
    transition-duration: var(--t-base);
  }
  .name {
    margin-right: 4px;
    font-family: var(--font-sans);
    font-size: 12px;
    letter-spacing: 0;
    text-transform: none;
    color: var(--fg);
  }

  @media (max-width: 767px) {
    .rail:not(.dock) { display: none; }
  }

  /* The phone dock: the same doors in a row, each a thumb wide. */
  .dock {
    position: static;
    flex: 1;
    width: auto;
    height: 100%;
    flex-direction: row;
    padding: 0;
  }
  .dock .group {
    flex: 1;
    flex-direction: row;
    justify-content: space-around;
  }
  /* More doors than a narrow phone is wide: the row scrolls rather than squeezes. */
  .dock { overflow-x: auto; scrollbar-width: none; }
  .dock .door { flex: none; }
  .dock .door { width: 44px; height: 40px; border-radius: var(--radius-s); }
  .dock .door::before { display: none; }
  .dock .door:active { background: var(--surface-alt); }
</style>
