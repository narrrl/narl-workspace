<script lang="ts">
  import Rail from './Rail.svelte'
  import { workspace } from './store.svelte'
  import { isTouchDevice } from './util'

  const mode = $derived(
    workspace.editing ? (workspace.vimEnabled ? workspace.vimMode.toUpperCase() : 'EDIT') : 'BROWSE',
  )

  // The path is the file name here: work/einarbeiten/01-was-ist-eebus.
  const file = $derived(
    workspace.open ? `${workspace.here}/${workspace.open.slug}` : `${workspace.here || '~'}/`,
  )

  const lines = $derived(workspace.editing ? workspace.draft.body.split('\n').length : null)
</script>

<footer>
  <span
    class="mode"
    class:insert={mode === 'INSERT' || mode === 'EDIT'}
    class:visual={mode.startsWith('VISUAL')}
  >
    {mode}
  </span>
  <span class="file">{file}{#if workspace.dirty}<span class="dirty"> [+]</span>{/if}</span>

  <span class="msg" class:alarm={workspace.flash?.kind === 'error'}>
    {workspace.flash?.text ?? ''}
  </span>

  {#if workspace.fullscreen}<span class="flag">full</span>{/if}
  {#if lines !== null}<span class="readout">{lines}<span class="unit">L</span></span>{/if}
  <span class="hint">
    {isTouchDevice ? 'tap : for commands' : '? help · : command · / search'}
  </span>

  <div class="tools"><Rail compact /></div>
</footer>

<style>
  footer {
    position: absolute;
    inset: auto 0 0 0;
    height: var(--status-h);
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 0 calc(var(--rail-w) + 4px) 0 calc(var(--frame) + 4px);
    font-size: 11px;
    color: var(--subtle);
    white-space: nowrap;
    overflow: hidden;
  }

  .mode {
    display: flex;
    align-items: center;
    height: 18px;
    padding: 0 8px;
    border-radius: 4px;
    flex: none;
    font-weight: 600;
    letter-spacing: 0.18em;
    background: var(--surface-alt);
    color: var(--fg);
    transition: background-color var(--t-base) var(--ease-glide), color var(--t-base) var(--ease-glide);
  }
  .mode.insert { background: var(--light); color: var(--ink); }
  .mode.visual { background: var(--light-dim); color: var(--ink); }

  .file { flex: none; color: var(--subtle); }
  .dirty { color: var(--light); }
  .msg { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; color: var(--fg); }
  .msg.alarm { color: var(--alarm); }
  .readout { color: var(--fg); font-variant-numeric: tabular-nums; }
  .unit { color: var(--muted); margin-left: 1px; }
  .hint { color: var(--muted); letter-spacing: 0.06em; }
  .flag { color: var(--light); letter-spacing: 0.18em; text-transform: uppercase; }

  .tools { display: none; }

  @media (max-width: 767px) {
    footer { padding-right: 6px; gap: 10px; }
    .hint, .file, .readout { display: none; }
    .tools { display: flex; flex: none; }
  }
</style>
