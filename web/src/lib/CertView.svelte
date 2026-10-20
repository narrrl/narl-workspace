<script lang="ts">
  /*
   * A certificate, key or keystore, as the server reads it. A password is
   * asked for only when the file needs one, and goes with that one request.
   */
  import { api, type CertItem, type Inspection } from './api'
  import { workspace } from './store.svelte'
  import { formatDay } from './util'

  let { id, name }: { id: number; name: string } = $props()

  let inspection = $state<Inspection | null>(null)
  let error = $state<string | null>(null)

  async function load(password?: string) {
    try {
      inspection = await api.inspect(id, password)
      error = null
    } catch (e) {
      error = e instanceof Error ? e.message : String(e)
    }
  }

  $effect(() => {
    void load()
  })

  async function unlock() {
    const password = await workspace.askSecret(`password for ${name}`)
    if (password === null) return
    await load(password)
    if (error) workspace.say(error, 'error')
  }

  function expiry(seconds: number): string {
    const days = Math.round((seconds - Date.now() / 1000) / 86400)
    if (days < 0) return `${formatDay(seconds)} — expired`
    return `${formatDay(seconds)} — ${days} day${days === 1 ? '' : 's'} left`
  }

  const expired = (item: CertItem) => item.kind === 'certificate' && item.not_after < Date.now() / 1000

  const title = (item: CertItem) =>
    ({ certificate: 'certificate', request: 'signing request', private_key: 'private key', public_key: 'public key' })[
      item.kind
    ]
</script>

{#if inspection}
  {#if inspection.needs_password}
    <p class="locked">
      protected by a password — <button onclick={unlock}>enter it</button> to read {inspection.items.length ? 'the rest' : 'it'}.
    </p>
  {/if}
  {#each inspection.items as item, i (i)}
    <section class="card" class:expired={expired(item)}>
      <h2 class="label">
        <span class="index">{i + 1}</span>
        {title(item)}
        {#if item.kind === 'certificate' || item.kind === 'private_key'}
          {#if item.label}<span class="muted">{item.label}</span>{/if}
        {/if}
      </h2>
      <dl>
        {#if item.kind === 'certificate'}
          <dt>subject</dt>
          <dd class="light">{item.subject}</dd>
          <dt>issuer</dt>
          <dd>{item.issuer === item.subject ? 'self-signed' : item.issuer}</dd>
          <dt>valid from</dt>
          <dd>{formatDay(item.not_before)}</dd>
          <dt>valid until</dt>
          <dd class:alarm={expired(item)}>{expiry(item.not_after)}</dd>
          <dt>key</dt>
          <dd>{item.key}</dd>
          {#if item.names.length}
            <dt>names</dt>
            <dd>{item.names.join(', ')}</dd>
          {/if}
          {#if item.usage.length}
            <dt>usage</dt>
            <dd>{item.usage.join(', ')}</dd>
          {/if}
          {#if item.ca}
            <dt>authority</dt>
            <dd>can sign other certificates</dd>
          {/if}
          <dt>serial</dt>
          <dd class="mono">{item.serial}</dd>
          <dt>sha-256</dt>
          <dd class="mono">{item.sha256}</dd>
          <dt>sha-1</dt>
          <dd class="mono">{item.sha1}</dd>
        {:else if item.kind === 'request'}
          <dt>subject</dt>
          <dd class="light">{item.subject}</dd>
          <dt>key</dt>
          <dd>{item.key}</dd>
          {#if item.names.length}
            <dt>names</dt>
            <dd>{item.names.join(', ')}</dd>
          {/if}
        {:else if item.kind === 'private_key'}
          <dt>key</dt>
          <dd>{item.key ?? 'encrypted'}</dd>
          {#if item.key}
            <dt>certificate</dt>
            <dd>{item.matches === null ? 'none here matches it' : `belongs to ${item.matches + 1}`}</dd>
          {/if}
        {:else}
          <dt>key</dt>
          <dd>{item.key}</dd>
        {/if}
      </dl>
    </section>
  {/each}
{:else if error}
  <p class="muted">{error}</p>
{:else}
  <p class="muted">reading…</p>
{/if}

<style>
  .card {
    max-width: 52rem;
    margin: 0 0 14px;
    padding: 12px 16px 14px;
    border: 1px solid var(--rim);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .card.expired { border-color: var(--alarm); }
  h2 {
    display: flex;
    gap: 10px;
    align-items: baseline;
    margin: 0 0 10px;
    font-size: 11px;
  }
  .index { color: var(--muted); }
  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 4px 16px;
    margin: 0;
    font-size: 12px;
  }
  dt { color: var(--muted); letter-spacing: 0.18em; text-transform: uppercase; font-size: 11px; }
  dd { margin: 0; color: var(--fg); overflow-wrap: anywhere; }
  .light { color: var(--light); }
  .alarm { color: var(--alarm); }
  .mono { font-family: var(--font-mono); font-size: 11px; }
  .locked { color: var(--subtle); }

  @media (max-width: 767px) {
    dl { grid-template-columns: 1fr; gap: 2px; }
    dd { margin-bottom: 6px; }
  }
</style>
