<script lang="ts">
  /*
   * The password `workspace.askSecret` asks for, in the same place as the
   * `[y/N]` prompt. A masked field of its own rather than the command line, so
   * a password never shows on screen or lands in the command history.
   *
   * It appears at once rather than rising into place like the `[y/N]` prompt:
   * it is where the next keys go, and a field still sliding and fading in
   * under them reads as a stutter. Password managers are asked to leave the
   * field alone: it is not the login, and their menus redrawn on every key
   * are a flicker of their own.
   */
  import { untrack } from 'svelte'
  import { hooks } from './commands'
  import { workspace } from './store.svelte'

  let value = $state('')
  let input = $state<HTMLInputElement>()
  /*
   * Where the keys went before the question took them. Putting the field away
   * would otherwise leave the focus nowhere, and the next keys — meant for the
   * editor the status line still says is listening — would walk the list.
   */
  let returnTo: HTMLElement | null = null
  let asking = false

  $effect(() => {
    if (workspace.secret) {
      value = ''
      if (!asking) returnTo = document.activeElement as HTMLElement | null
      asking = true
      input?.focus()
    } else if (asking) {
      asking = false
      untrack(() => {
        if (returnTo?.isConnected && returnTo !== document.body) returnTo.focus()
        else if (workspace.editing && workspace.pane === 'document') hooks.focusEditor()
        else hooks.focusList()
      })
      returnTo = null
    }
  })

  function onkeydown(event: KeyboardEvent) {
    event.stopPropagation()
    if (event.key === 'Enter') {
      event.preventDefault()
      workspace.secret?.resolve(value)
      value = ''
    } else if (event.key === 'Escape') {
      event.preventDefault()
      workspace.secret?.resolve(null)
      value = ''
    }
  }
</script>

{#if workspace.secret}
  <div class="prompt" role="dialog" aria-label={workspace.secret.text}>
    <label class="text" for="secret">{workspace.secret.text}</label>
    <input
      id="secret"
      type="password"
      autocomplete="off"
      spellcheck="false"
      data-1p-ignore
      data-lpignore="true"
      data-bwignore
      data-form-type="other"
      bind:this={input}
      bind:value
      {onkeydown}
    />
    <button
      onclick={() => {
        workspace.secret?.resolve(null)
        value = ''
      }}>esc</button
    >
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
    border: 1px solid var(--light-dim);
    border-bottom: 0;
    border-radius: var(--radius-l) var(--radius-l) 0 0;
    font-size: 13px;
  }
  .text { flex: none; color: var(--light); }
  input {
    flex: 1;
    min-width: 0;
    padding: 4px 8px;
    background: var(--surface);
    border: 1px solid var(--rim);
    border-radius: var(--radius-s);
    color: var(--light);
    font-family: var(--font-mono);
  }
  input:focus { border-color: var(--light-dim); outline: none; }

  @media (max-width: 767px) {
    .prompt { left: 8px; right: 8px; flex-wrap: wrap; }
  }
</style>
