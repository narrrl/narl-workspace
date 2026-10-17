<script lang="ts">
  /*
   * The top of the frame, laid out like the desktop bar it is copied from: the
   * mark, the spaces as workspace pills, where in the space the list is, and
   * the clock. Spaces and crumbs are clickable, because a tree is the one thing
   * that is genuinely faster to walk with a pointer than with `:cd`.
   */
  import Clock from './Clock.svelte'
  import { hooks } from './commands'
  import { workspace } from './store.svelte'

  // The space is the first crumb, and it is already named by its pill.
  const crumbs = $derived(workspace.path.slice(1))

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
        {#if index < 10}<span class="n">{(index + 1) % 10}</span>{/if}<span class="slug">{space.slug}</span>
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
  <Clock />
</header>

<style>
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

  .n { color: var(--muted); font-size: 10px; }
  .pill:hover .n { color: var(--light-dim); }
  .pill[aria-current='true'] .n { color: var(--ink); opacity: 0.55; }
  .pill.add { color: var(--muted); }
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

  /*
   * On a phone the other spaces are only their number, like the desktop's
   * workspace dots, so all of them fit; the current one keeps its name.
   */
  @media (max-width: 767px) {
    .crumbs, .divider.path, .bar .user, .divider.wide { display: none; }
    .spaces { max-width: none; flex: 0 1 auto; gap: 2px; }
    .pill:not([aria-current='true']) .slug { display: none; }
    .pill:not([aria-current='true']) { min-width: 28px; height: 24px; }
    .pill:not([aria-current='true']) .n { font-size: 11px; color: var(--subtle); }
  }
</style>
