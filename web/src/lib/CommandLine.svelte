<script lang="ts">
  import { untrack } from 'svelte'
  import { completions, runCommand, hooks } from './commands'
  import { workspace } from './store.svelte'

  interface Props {
    initial: string
    close: () => void
  }

  const { initial, close }: Props = $props()

  let value = $state(untrack(() => initial))
  let input = $state<HTMLInputElement>()
  const isSearch = $derived(value.startsWith('/'))
  const matches = $derived(isSearch ? [] : completions(value).slice(0, 8))

  $effect(() => {
    input?.focus()
  })

  // A search is live: the list filters as you type, like `incsearch`.
  $effect(() => {
    if (!isSearch) return
    const query = value.slice(1)
    const timer = setTimeout(() => void workspace.guard(() => workspace.search(query)), 120)
    return () => clearTimeout(timer)
  })

  async function submit(event: SubmitEvent) {
    event.preventDefault()
    const line = value
    close()
    if (isSearch) {
      hooks.focusList()
      return
    }
    await runCommand(line)
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault()
      close()
      hooks.focusList()
    }
    if (event.key === 'Tab' && matches.length > 0) {
      event.preventDefault()
      value = `:${matches[0].name} `
    }
  }
</script>

<div class="cmdline">
  {#if matches.length > 0 && value.replace(/^:/, '').length > 0}
    <ul class="completions">
      {#each matches as spec (spec.name)}
        <li>
          <button onclick={() => (value = `:${spec.name} `, input?.focus())}>
            <span class="name">:{spec.name}</span>
            {#if spec.args}<span class="muted">{spec.args}</span>{/if}
            <span class="help">{spec.help}</span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}

  <form onsubmit={submit}>
    <input bind:this={input} bind:value {onkeydown} spellcheck="false" autocomplete="off" />
  </form>
</div>

<style>
  /*
   * The command line slides up out of the bottom band, the way a popout comes
   * out from behind the desktop's frame rather than fading in over it.
   */
  .cmdline {
    position: absolute;
    left: calc(var(--frame) + 12px);
    right: calc(var(--rail-w) + 12px);
    bottom: calc(var(--status-h) - 1px);
    z-index: 20;
    display: flex;
    flex-direction: column;
    background: color-mix(in oklab, var(--hull) 97%, transparent);
    border: 1px solid var(--surface-alt);
    border-bottom: 0;
    border-radius: var(--radius-l) var(--radius-l) 0 0;
    overflow: hidden;
    animation: rise var(--t-slow) var(--ease-snap) both;
  }

  form { display: flex; padding: 6px; }

  input {
    flex: 1;
    height: 30px;
    background: var(--surface);
    border: 1px solid var(--rim);
    border-radius: var(--radius-s);
    color: var(--light);
    padding: 0 10px;
    font-size: 13px;
  }

  .completions {
    list-style: none;
    margin: 0;
    padding: 6px 6px 0;
    max-height: 40vh;
    overflow-y: auto;
  }

  .completions button {
    display: flex;
    gap: 10px;
    align-items: baseline;
    width: 100%;
    border: 0;
    text-align: left;
    padding: 3px 10px;
    font-size: 12px;
    letter-spacing: 0;
    text-transform: none;
  }
  .completions button:hover { background: var(--surface-alt); }
  .name { color: var(--light); }
  .help { color: var(--subtle); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
