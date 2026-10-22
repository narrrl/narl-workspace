<script lang="ts">
  /*
   * Proton's human check, when a login from `:backup login` runs into one:
   * Proton's own page in a frame, which hands back a token once it is solved.
   * Only messages from that page are listened to. Closing the sheet — Escape,
   * or a click beside it — abandons the login.
   */
  import { onDestroy } from 'svelte'
  import Sheet from './Sheet.svelte'
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
      workspace.closeSheet()
    } else if (data?.type === 'RESIZE' && typeof payload?.height === 'number') {
      height = Math.min(Math.max(payload.height, 320), 900)
    } else if (data?.type === 'CLOSE') {
      workspace.closeSheet()
    }
  }

  onDestroy(() => asked?.resolve(null))
</script>

<svelte:window {onmessage} />

<Sheet title="proton" detail="prove you are human to log in" width={64}>
  {#if asked}
    <div class="frame">
      <iframe
        title="Proton human verification"
        src={asked.url}
        style:height="{height}px"
        sandbox="allow-scripts allow-same-origin allow-forms allow-popups"
        referrerpolicy="no-referrer"
      ></iframe>
    </div>
  {/if}
</Sheet>

<style>
  .frame { min-height: 0; overflow-y: auto; }
  iframe { display: block; width: 100%; border: 0; border-radius: 6px; background: #fff; }
</style>
