<script lang="ts">
  import { ApiError } from './api'
  import { workspace } from './store.svelte'

  let username = $state('')
  let password = $state('')
  let error = $state('')
  let busy = $state(false)
  let field = $state<HTMLInputElement>()

  $effect(() => {
    field?.focus()
  })

  async function submit(event: SubmitEvent) {
    event.preventDefault()
    busy = true
    error = ''
    try {
      await workspace.login(username, password)
    } catch (e) {
      // The server answers a wrong password with a bare 401; the throttle's
      // "wait Ns" message is worth showing verbatim.
      error =
        e instanceof ApiError && e.status === 401
          ? 'invalid credentials'
          : e instanceof Error
            ? e.message
            : 'login failed'
      password = ''
    } finally {
      busy = false
    }
  }
</script>

<main>
  <form class="resolve" onsubmit={submit}>
    <div class="brand">
      <span class="mark" aria-hidden="true"></span>
      <h1 class="display">workspace</h1>
    </div>

    <p class="label rule">a private workspace · one user</p>

    <label>
      <span class="label">login</span>
      <input bind:this={field} bind:value={username} autocomplete="username" spellcheck="false" />
    </label>

    <label>
      <span class="label">password</span>
      <input type="password" bind:value={password} autocomplete="current-password" />
    </label>

    <div class="row">
      <button class="enter" type="submit" disabled={busy}>
        <span class="dot"></span>
        {busy ? 'authenticating' : 'enter'}
      </button>
      {#if error}<span class="alarm">{error}</span>{/if}
    </div>
  </form>
</main>

<style>
  main {
    display: grid;
    place-items: center;
    height: 100%;
    padding: 24px;
    background:
      radial-gradient(120% 70% at 30% 0%, #141713 0%, transparent 60%),
      var(--base);
  }

  form {
    display: flex;
    flex-direction: column;
    gap: 16px;
    width: min(100%, 44ch);
    background: color-mix(in oklab, var(--hull) 97%, transparent);
    border: 1px solid var(--surface-alt);
    border-radius: var(--frame-radius);
    padding: 28px;
  }

  .brand { display: flex; align-items: center; gap: 14px; }
  h1 { margin: 0; font-size: 26px; text-shadow: 0 2px 24px var(--ink); }

  /* The ring-and-dot mark, larger than in the bar. */
  .mark {
    position: relative;
    width: 18px;
    height: 18px;
    flex: none;
    border: 2px solid var(--light);
    border-radius: 999px;
  }
  .mark::after {
    content: '';
    position: absolute;
    inset: 3px;
    border-radius: 999px;
    background: var(--light);
  }

  p { margin: 0 0 4px; font-size: 10px; }

  label { display: flex; flex-direction: column; gap: 6px; }
  label .label { font-size: 10px; }
  input { height: 36px; }

  .row { display: flex; align-items: center; gap: 14px; margin-top: 4px; }

  /* The desktop's action button: a hairline in light, a dot, a spaced label. */
  .enter {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 44px;
    padding: 0 20px 0 16px;
    font-size: 11px;
    letter-spacing: 0.22em;
    color: var(--light);
    border-color: color-mix(in oklab, var(--light) 55%, transparent);
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 999px;
    background: currentColor;
    opacity: 0.35;
    transition: opacity var(--t-base) var(--ease-glide);
  }
  .enter:hover .dot, .enter:focus-visible .dot { opacity: 1; }
  .alarm { font-size: 12px; }
</style>
