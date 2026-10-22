<script lang="ts">
  /*
   * The `[y/N]` question `workspace.prompts.ask` raises, drawn where the command line
   * opens. The keys are read in App.svelte, ahead of everything else, so a
   * stray `y` can never reach the editor or the list underneath.
   */
  import { workspace } from './store.svelte'
</script>

{#if workspace.prompts.prompt}
  <div class="prompt" role="alertdialog" aria-label={workspace.prompts.prompt.text}>
    <span class="text">{workspace.prompts.prompt.text}</span>
    <span class="choice">
      <button class="yes" onclick={() => workspace.prompts.prompt?.resolve(true)}><span class="kbd">y</span> yes</button>
      <button onclick={() => workspace.prompts.prompt?.resolve(false)}>no</button>
    </span>
  </div>
{/if}

<style>
  .prompt {
    position: absolute;
    left: calc(var(--frame) + 12px);
    right: calc(var(--rail-w) + 12px);
    bottom: calc(var(--status-h) - 1px);
    z-index: 40;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 14px;
    background: color-mix(in oklab, var(--hull) 97%, transparent);
    border: 1px solid var(--alarm);
    border-bottom: 0;
    border-radius: var(--radius-l) var(--radius-l) 0 0;
    font-size: 13px;
    animation: rise var(--t-slow) var(--ease-snap) both;
  }
  .text { flex: 1; min-width: 0; color: var(--light); overflow-wrap: anywhere; }
  .choice { flex: none; display: flex; gap: 6px; align-items: center; }
  .yes:hover { background: var(--alarm); border-color: var(--alarm); color: var(--ink); }

  @media (max-width: 767px) {
    .prompt { left: 8px; right: 8px; flex-wrap: wrap; }
  }
</style>
