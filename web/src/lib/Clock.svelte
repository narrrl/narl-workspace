<script lang="ts">
  // The bar's clock: hours and minutes, ticked often enough to turn on time.
  let now = $state(new Date())
  $effect(() => {
    const timer = setInterval(() => (now = new Date()), 10_000)
    return () => clearInterval(timer)
  })
  const clock = $derived(
    now.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hour12: false }),
  )
</script>

<time class="clock">{clock}</time>

<style>
  .clock {
    color: var(--light);
    letter-spacing: 0.12em;
    font-variant-numeric: tabular-nums;
  }
</style>
