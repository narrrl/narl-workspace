<script lang="ts">
  import Rail from './Rail.svelte'
  import { workspace } from './store.svelte'
  import { isTouchDevice } from './util'

  /** A key waiting for its second half, like vim's `showcmd`. */
  const { pending = '' }: { pending?: string } = $props()

  const mode = $derived(
    workspace.editing ? (workspace.vimEnabled ? workspace.vimMode.toUpperCase() : 'EDIT') : 'BROWSE',
  )

  // The path is the file name here: work/einarbeiten/01-was-ist-eebus.
  const file = $derived(
    workspace.open ? `${workspace.here}/${workspace.open.slug}` : `${workspace.here || '~'}/`,
  )

  const lines = $derived(workspace.editing ? workspace.draft.body.split('\n').length : null)
</script>

<footer class="status">
  <span
    class="mode"
    class:editing={workspace.editing}
    class:insert={mode === 'INSERT' || mode === 'EDIT'}
    class:visual={mode.startsWith('VISUAL')}
  >
    {mode}
  </span>
  <span class="file">{file}{#if workspace.dirty}<span class="dirty"> [+]</span>{/if}</span>

  <span class="msg" class:alarm={workspace.flash?.kind === 'error'}>{workspace.flash?.text ?? ''}</span>

  {#if pending}<span class="pending" aria-live="polite">{pending}</span>{/if}
  {#if workspace.tiles.length > 1}<span class="flag" title=":ls lists the tiles">tile {workspace.focused + 1}/{workspace.tiles.length}</span>{/if}
  {#if workspace.fullscreen}<span class="flag">full</span>{/if}
  {#if workspace.vault?.unlocked}<span class="flag vault" title="the vault is open — :lock closes it">vault open</span>{/if}
  {#if lines !== null}<span class="readout">{lines}<span class="unit">L</span></span>{/if}
  <span class="hint">
    {isTouchDevice ? 'tap : for commands' : '? help · : command · / search'}
  </span>

  <div class="tools"><Rail dock /></div>
</footer>

<style>
  .mode.insert { background: var(--light); color: var(--ink); }
  .mode.visual { background: var(--light-dim); color: var(--ink); }

  .file { flex: none; color: var(--subtle); }
  .dirty { color: var(--light); }
  .msg { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; color: var(--fg); }
  .msg.alarm { color: var(--alarm); }
  .pending { flex: none; color: var(--light); font-family: var(--font-mono); }
  .flag { color: var(--light); letter-spacing: 0.18em; text-transform: uppercase; }

  .tools { display: none; }

  /*
   * On a phone the band is the dock. The mode only shows while writing, and
   * messages float just above the band as a toast, so they never squeeze the
   * doors.
   */
  @media (max-width: 767px) {
    .status { padding: 0 6px; gap: 6px; overflow: visible; }
    .hint, .file, .readout, .flag, .pending { display: none; }
    .mode:not(.editing) { display: none; }
    .tools { display: flex; flex: 1; height: 100%; }

    .msg {
      position: absolute;
      left: 50%;
      bottom: calc(100% + 8px);
      max-width: calc(100% - 24px);
      padding: 6px 12px;
      border: 1px solid var(--overlay);
      border-radius: var(--radius-s);
      background: var(--surface);
      box-shadow: 0 6px 24px rgb(0 0 0 / 0.45);
      transform: translateX(-50%);
      z-index: 20;
    }
    .msg:empty { display: none; }
  }
</style>
