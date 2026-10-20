<script lang="ts">
  /*
   * Proton's human check, when a login from `:backup login` runs into one:
   * Proton's own page in a frame, which hands back a token once it is solved.
   * Only messages from that page are listened to. Closing the sheet — Escape,
   * or a click beside it — abandons the login.
   */
  import { onDestroy } from 'svelte'
  import { workspace } from './store.svelte'

  const ORIGIN = 'https://verify.proton.me'
  const asked = workspace.verify
  let height = $state(560)

  function onmessage(event: MessageEvent) {
    if (event.origin !== ORIGIN) return
    let data = event.data
    if (typeof data === 'string') {
      try {
        data = JSON.parse(data)
      } catch {
        return
      }
    }
    const payload = data?.payload
    if (data?.type === 'HUMAN_VERIFICATION_SUCCESS' && typeof payload?.token === 'string') {
      asked?.resolve(payload.token)
      workspace.overlay = 'none'
    } else if (data?.type === 'RESIZE' && typeof payload?.height === 'number') {
      height = Math.min(Math.max(payload.height, 320), 900)
    } else if (data?.type === 'CLOSE') {
      workspace.overlay = 'none'
    }
  }

  onDestroy(() => asked?.resolve(null))
</script>

<svelte:window {onmessage} />

<div class="sheet">
  <header>
    <span class="label rule"><span class="marker"></span><span class="title">proton</span> prove you are human to log in</span>
    <button onclick={() => (workspace.overlay = 'none')}>esc</button>
  </header>
  {#if asked}
    <iframe
      title="Proton human verification"
      src={asked.url}
      style:height="{height}px"
      sandbox="allow-scripts allow-same-origin allow-forms allow-popups"
      referrerpolicy="no-referrer"
    ></iframe>
  {/if}
</div>

<style>
  .sheet {
    display: flex;
    flex-direction: column;
    gap: 16px;
    width: min(100%, 64ch);
    max-height: 100%;
    overflow-y: auto;
    background: color-mix(in oklab, var(--hull) 97%, transparent);
    border: 1px solid var(--surface-alt);
    border-radius: var(--frame-radius);
    padding: 20px 24px;
  }
  header { display: flex; align-items: center; gap: 12px; }
  header .label { flex: 1; }
  iframe { width: 100%; border: 0; border-radius: 6px; background: #fff; }
</style>
