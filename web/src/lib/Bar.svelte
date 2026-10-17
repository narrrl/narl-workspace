<script lang="ts">
  /*
   * The top of the frame, laid out like the desktop bar it is copied from: the
   * mark, the spaces as workspace pills, where in the space the list is, and
   * the clock. Spaces and crumbs are clickable, because a tree is the one thing
   * that is genuinely faster to walk with a pointer than with `:cd`.
   */
  import { hooks } from './commands'
  import { workspace } from './store.svelte'

  // The space is the first crumb, and it is already named by its pill.
  const crumbs = $derived(workspace.path.slice(1))

  let now = $state(new Date())
  $effect(() => {
    const timer = setInterval(() => (now = new Date()), 10_000)
    return () => clearInterval(timer)
  })
  const clock = $derived(
    now.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hour12: false }),
  )
</script>

<header class="bar">
  <span class="mark" aria-hidden="true"></span>
  <span class="divider"></span>

  <nav class="spaces" aria-label="spaces">
    {#each workspace.spaces as space, index (space.id)}
      <button
        class="pill"
        aria-current={space.id === workspace.spaceId}
        title="{space.name} ({index < 10 ? (index + 1) % 10 : ''})"
        onclick={() => workspace.guard(() => workspace.enterSpace(space.id))}
      >
        {#if index < 10}<span class="n">{(index + 1) % 10}</span>{/if}{space.slug}
      </button>
    {/each}
    <button class="pill add" title="new space (:space! <name>)" onclick={() => hooks.openCommandLine(':space! ')}>
      +
    </button>
  </nav>

  <span class="divider path"></span>

  <nav class="crumbs" aria-label="path">
    {#if workspace.view === 'board'}
      <span class="here">board</span>
    {:else}
      <button
        class="crumb"
        class:here={crumbs.length === 0}
        onclick={() => workspace.spaceId !== null && workspace.guard(() => workspace.goTo(workspace.spaceId!))}
      >
        ~
      </button>
      {#each crumbs as crumb, index (crumb.id)}
        <span class="sep">/</span>
        <button
          class="crumb"
          class:here={index === crumbs.length - 1}
          onclick={() => workspace.guard(() => workspace.goTo(crumb.id))}
        >
          {crumb.slug}
        </button>
      {/each}
    {/if}
  </nav>

  <span class="spacer"></span>

  <span class="readout user">
    <span class="key">user</span>
    <span class="value">{workspace.user}</span>
  </span>
  <span class="divider wide"></span>
  <time class="clock">{clock}</time>
</header>

<style>
  .bar {
    position: absolute;
    inset: 0 0 auto 0;
    height: var(--bar-h);
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 calc(var(--rail-w) + 4px) 0 calc(var(--frame) + 10px);
    font-size: 11px;
    letter-spacing: 0.18em;
    text-transform: uppercase;
    color: var(--subtle);
    white-space: nowrap;
    overflow: hidden;
  }

  /* A ring with a dot in it: the favicon, and the desktop's launcher. */
  .mark {
    position: relative;
    width: 12px;
    height: 12px;
    flex: none;
    border: 1.5px solid var(--light);
    border-radius: 999px;
  }
  .mark::after {
    content: '';
    position: absolute;
    inset: 2px;
    border-radius: 999px;
    background: var(--light);
  }

  .divider {
    width: 1px;
    height: 14px;
    flex: none;
    background: var(--overlay);
  }

  .spaces, .crumbs {
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .spaces { flex: none; max-width: 50%; }
  .crumbs { flex: 0 1 auto; }

  /* Workspace pills: muted at rest, light on hover, inverted when current. */
  /* The focused pill widens, as the desktop's does, over the slow duration. */
  .pill {
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 22px;
    height: 20px;
    padding: 0 6px;
    transition:
      padding var(--t-slow) var(--ease-snap),
      background-color var(--t-base) var(--ease-glide),
      color var(--t-base) var(--ease-glide);
    border: 0;
    border-radius: 4px;
    font-size: 11px;
    letter-spacing: 0.06em;
    text-transform: none;
    color: var(--muted);
    background: transparent;
  }
  .pill:hover { background: transparent; color: var(--light); }
  .pill[aria-current='true'] { background: var(--light); color: var(--ink); padding: 0 12px; }
  .n { color: var(--overlay); font-size: 10px; }
  .pill:hover .n { color: var(--muted); }
  .pill[aria-current='true'] .n { color: var(--ink); opacity: 0.55; }
  .pill:focus-visible { background: var(--light); color: var(--ink); }
  .pill.add { color: var(--overlay); }
  .pill.add:hover { color: var(--light); }

  /* A crumb is a link along the path; a hairline grows under it on hover. */
  .crumb {
    position: relative;
    border: 0;
    border-radius: 0;
    padding: 2px 0;
    background: transparent;
    font-size: 11px;
    letter-spacing: 0.06em;
    text-transform: none;
    color: var(--subtle);
  }
  .crumb::after {
    content: '';
    position: absolute;
    left: 0;
    right: 0;
    bottom: -1px;
    height: 1px;
    background: var(--light);
    transform: scaleX(0);
    transform-origin: left;
    transition: transform var(--t-base) var(--ease-snap);
  }
  .crumb:hover { background: transparent; color: var(--light); }
  .crumb:hover::after { transform: scaleX(1); }
  .crumb.here, .here { color: var(--light); }
  .sep { color: var(--overlay); }
  span.here { letter-spacing: 0.06em; text-transform: none; }

  .spacer { flex: 1; }

  .readout { display: flex; gap: 8px; font-variant-numeric: tabular-nums; }
  .key { color: var(--muted); }
  .value { color: var(--fg); text-transform: none; letter-spacing: 0.06em; }

  .clock {
    color: var(--light);
    letter-spacing: 0.12em;
    font-variant-numeric: tabular-nums;
  }

  @media (max-width: 767px) {
    .bar { padding-left: 12px; padding-right: 12px; gap: 10px; }
    .crumbs, .divider.path, .user, .divider.wide { display: none; }
    .spaces { max-width: none; flex: 1; }
  }
</style>
