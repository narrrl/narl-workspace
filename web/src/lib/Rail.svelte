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
  import { keysOf } from './keymap'
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
    const cmd: Door = { icon: 'prompt', label: 'Command line', keys: keysOf('global', 'line'), run: () => hooks.openCommandLine(':') }
    const help: Door = { icon: 'help', label: 'Help', keys: keysOf('global', 'help'), run: () => runCommand('help') }
    const settings: Door = { icon: 'settings', label: 'Settings', keys: keysOf('global', 'settings'), run: () => runCommand('settings') }

    const focus = workspace.focus
    const board: Door = workspace.tree.space?.has_board || focus === 'board'
      ? { icon: 'board', label: 'Board', keys: keysOf('global', 'board'), run: () => runCommand('board'), on: focus === 'board' }
      : { icon: 'board', label: 'Give this space a board', keys: [':set board'], run: () => runCommand('set board'), dim: true }
    const agenda: Door = { icon: 'agenda', label: 'Agenda — late, due, in hand', keys: keysOf('global', 'agenda'), run: () => runCommand('agenda'), on: focus === 'agenda' }
    const calendar: Door = { icon: 'calendar', label: 'Calendar — the diary by month', keys: keysOf('global', 'calendar'), run: () => runCommand('cal'), on: focus === 'calendar' }
    // A phone shows the focused tile alone, so a view needs a door out of it.
    const close: Door[] =
      focus !== 'list' && focus !== 'node' ? [{ icon: 'list', label: 'Close this view', keys: keysOf('views', 'close'), run: () => runCommand('close') }] : []

    if (focus === 'board' || focus === 'card') {
      // The card verbs are doors too, so a phone — which has no H, L or
      // Space — can still move work along.
      const card = !!workspace.cards.card
      return [
        [board, agenda, calendar, ...close],
        [
          { icon: 'doc-plus', label: 'New card on this list', keys: keysOf('board', 'new'), run: () => hooks.openCommandLine(':card ') },
          { icon: 'column-plus', label: 'New list', keys: [':list'], run: () => hooks.openCommandLine(':list ') },
        ],
        [
          { icon: 'left', label: 'Send the card left', keys: keysOf('board', 'sendLeft'), run: () => workspace.guard(() => workspace.cards.sendCard(-1)), dim: !card },
          { icon: 'check', label: 'Done, or back to the inbox', keys: keysOf('board', 'done'), run: () => workspace.guard(() => workspace.cards.toggleDone()), dim: !card },
          { icon: 'right', label: 'Send the card right', keys: keysOf('board', 'sendRight'), run: () => workspace.guard(() => workspace.cards.sendCard(1)), dim: !card },
          { icon: 'calendar', label: 'Due date', keys: keysOf('board', 'due'), run: () => hooks.openCommandLine(':due '), dim: !card },
          { icon: 'repeat', label: 'Repeat once done', keys: [':repeat'], run: () => hooks.openCommandLine(':repeat '), dim: !card },
          { icon: 'doc', label: "The card's notes", keys: keysOf('board', 'notes'), run: () => workspace.guard(() => workspace.cards.openCard()), dim: !card },
        ],
        [
          { icon: 'up', label: 'Card up its list', keys: keysOf('board', 'nudgeUp'), run: () => workspace.guard(() => workspace.cards.nudgeCard(-1)), dim: !card },
          { icon: 'down', label: 'Card down its list', keys: keysOf('board', 'nudgeDown'), run: () => workspace.guard(() => workspace.cards.nudgeCard(1)), dim: !card },
          {
            icon: 'rename',
            label: card ? 'Rename the card' : 'Rename the list',
            keys: keysOf('board', 'rename'),
            run: () => hooks.openCommandLine(card ? ':name ' : ':list name '),
          },
          {
            icon: 'trash',
            label: card ? 'Delete the card' : 'Delete the list',
            keys: card ? keysOf('board', 'delete') : [':list rm'],
            run: () => runCommand(card ? 'delete' : 'list rm'),
          },
        ],
        [cmd, settings, help],
      ]
    }

    return [
      [
        { icon: 'doc-plus', label: 'New document', keys: keysOf('tree', 'new'), run: () => runCommand('new') },
        { icon: 'folder-plus', label: 'New folder', keys: keysOf('tree', 'mkdir'), run: () => hooks.openCommandLine(':mkdir ') },
        ...(workspace.tree.space ? [board] : []),
      ],
      [
        agenda,
        calendar,
        { icon: 'today', label: "Today's diary entry", keys: keysOf('global', 'today'), run: () => runCommand('today') },
        { icon: 'capture', label: 'Capture a card to the inbox', keys: keysOf('tree', 'capture'), run: () => hooks.openCommandLine(':card ') },
        ...close,
      ],
      [
        { icon: 'upload', label: 'Upload files into this folder', keys: [':upload'], run: () => hooks.pickTreeFiles(false) },
        { icon: 'trash', label: 'The trash — what was deleted', keys: [':trash'], run: () => runCommand('trash') },
        ...(workspace.tree.marked.length
          ? [{ icon: 'drop' as const, label: `Drop ${dropping(workspace.tree.marked)} here`, keys: keysOf('tree', 'drop'), run: () => workspace.guard(() => workspace.tree.drop()), on: true }]
          : []),
      ],
      [
        { icon: 'grid', label: 'Every space', keys: keysOf('global', 'overview'), run: () => hooks.openOverview() },
        {
          icon: workspace.fullscreen ? 'shrink' : 'expand',
          label: workspace.fullscreen ? 'Leave fullscreen' : 'Fullscreen the focused pane',
          keys: keysOf('global', 'fullscreen'),
          run: () => (workspace.fullscreen = !workspace.fullscreen),
          on: workspace.fullscreen,
          desk: true,
        },
        { icon: 'search', label: 'Search this space', keys: keysOf('tree', 'search'), run: () => hooks.openCommandLine('/') },
        { icon: 'search-all', label: 'Search every space', keys: keysOf('global', 'find'), run: () => hooks.openGlobalSearch() },
      ],
      [cmd, settings, help],
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
