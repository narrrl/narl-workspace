<script lang="ts" module>
  /*
   * What was run before, oldest first, kept across reloads. Commands and
   * searches are walked separately, as in vim: `/` then ↑ finds searches.
   */
  const HISTORY_CAP = 100

  function readHistory(key: string): string[] {
    try {
      const lines = JSON.parse(localStorage.getItem(key) ?? '[]')
      return Array.isArray(lines) ? lines.filter((line) => typeof line === 'string') : []
    } catch {
      return []
    }
  }

  /** The line goes to the end, and any earlier copy of it goes. */
  function remember(key: string, line: string) {
    if (!line.replace(/^[:/]/, '').trim()) return
    const lines = [...readHistory(key).filter((old) => old !== line), line].slice(-HISTORY_CAP)
    try {
      localStorage.setItem(key, JSON.stringify(lines))
    } catch {
      /* a full localStorage costs the history, not the command */
    }
  }
</script>

<script lang="ts">
  import { tick, untrack } from 'svelte'
  import { argCompletions, completions, runCommand, hooks } from './commands'
  import { workspace } from './store.svelte'

  interface Props {
    initial: string
    /** A `/` search across every space rather than this one — Ctrl-P. */
    global?: boolean
    close: () => void
  }

  /** One line of the completion list, and what picking it makes of the line. */
  interface Item {
    value: string
    label: string
    args?: string
    help?: string
  }

  const { initial, global = false, close }: Props = $props()

  let value = $state(untrack(() => initial))
  let input = $state<HTMLInputElement>()
  let list = $state<HTMLUListElement>()
  const isSearch = $derived(value.startsWith('/'))
  const historyKey = $derived(isSearch ? 'diary:history:search' : 'diary:history:cmd')

  /*
   * Where the keys were before the line took them: the line goes away under
   * the focus, which would otherwise be left on nothing at all.
   */
  const returnTo = document.activeElement as HTMLElement | null

  /** Tab's walk through the matches, frozen as they were when it started. */
  let cycle = $state<{ items: Item[]; at: number } | null>(null)
  /** ↑'s walk back through the history, narrowed by what was typed first. */
  let walk: { stem: string; lines: string[]; at: number } | null = null

  function candidates(line: string): Item[] {
    const args = argCompletions(line)
    if (args !== null) {
      const head = line.replace(/^:/, '').split(/\s/)[0]
      return args.map((arg) => ({ value: `:${head} ${arg}`, label: arg }))
    }
    return completions(line).map((spec) => ({
      value: `:${spec.name} `,
      label: `:${spec.name}`,
      args: spec.args,
      help: spec.help,
    }))
  }

  const shown = $derived.by((): Item[] => {
    if (cycle) return cycle.items
    if (isSearch || value.replace(/^:/, '').length === 0) return []
    return candidates(value).slice(0, 8)
  })

  $effect(() => {
    input?.focus()
  })

  // The match Tab is on stays in sight in a list longer than the box.
  $effect(() => {
    if (cycle?.at === undefined) return
    void tick().then(() => list?.querySelector('.current')?.scrollIntoView({ block: 'nearest' }))
  })

  // A search is live: the list filters as you type, like `incsearch`.
  $effect(() => {
    if (!isSearch) return
    const query = value.slice(1)
    const timer = setTimeout(() => void workspace.guard(() => workspace.search(query, global)), 120)
    return () => clearTimeout(timer)
  })

  /**
   * Hand the keys back once the line is gone, unless what ran took them
   * somewhere itself — into the editor, a prompt, an overlay.
   */
  async function giveBackFocus() {
    // The line is taken off the page on the next update; until then it is
    // still what has the focus.
    await tick()
    const active = document.activeElement
    if (active && active !== document.body && active.isConnected) return
    if (returnTo?.isConnected && returnTo !== document.body) returnTo.focus()
    else if (workspace.editing && workspace.pane === 'document') hooks.focusEditor()
    else hooks.focusList()
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault()
    const line = value
    remember(historyKey, line)
    close()
    if (isSearch) {
      // Enter goes to the highlighted result, once the results are for what
      // was typed rather than for the line a keystroke ago.
      const query = line.slice(1)
      await workspace.guard(async () => {
        if (query && workspace.query !== query) await workspace.search(query, global)
        if (query && workspace.selected) await workspace.openSelected()
      })
      hooks.focusList()
      return
    }
    await runCommand(line)
    await giveBackFocus()
  }

  /** Tab and Shift-Tab: the next or previous match, round and round. */
  function complete(delta: 1 | -1) {
    if (cycle) {
      const count = cycle.items.length
      cycle.at = (cycle.at + delta + count) % count
      value = cycle.items[cycle.at].value
      return
    }
    const items = candidates(value)
    if (items.length === 0) return
    // A single match is simply taken, so the next Tab moves on to what follows.
    if (items.length === 1) {
      value = items[0].value
      return
    }
    cycle = { items, at: delta > 0 ? 0 : items.length - 1 }
    value = items[cycle.at].value
  }

  /** ↑ and ↓: the lines run before, newest first, that start as this one does. */
  function recall(delta: 1 | -1) {
    if (!walk) {
      const stem = value
      const lines = readHistory(historyKey)
        .reverse()
        .filter((line) => line.startsWith(stem) && line !== stem)
      walk = { stem, lines, at: -1 }
    }
    const at = Math.min(Math.max(walk.at + delta, -1), walk.lines.length - 1)
    if (at === walk.at) return
    walk.at = at
    value = at < 0 ? walk.stem : walk.lines[at]
    cycle = null
    void tick().then(() => input?.setSelectionRange(value.length, value.length))
  }

  function onkeydown(event: KeyboardEvent) {
    const ctrl = event.ctrlKey && !event.metaKey && !event.altKey
    if (event.key === 'Escape') {
      event.preventDefault()
      event.stopPropagation()
      close()
      void giveBackFocus()
    } else if (event.key === 'Tab' && isSearch) {
      // Nothing to complete in a search: Tab walks the results instead.
      event.preventDefault()
      workspace.move(event.shiftKey ? -1 : 1)
    } else if (event.key === 'Tab') {
      event.preventDefault()
      complete(event.shiftKey ? -1 : 1)
    } else if (ctrl && isSearch && (event.key === 'n' || event.key === 'p')) {
      // The results, walked without leaving the line; Enter opens the one lit.
      event.preventDefault()
      workspace.move(event.key === 'n' ? 1 : -1)
    } else if (event.key === 'ArrowUp' || (ctrl && event.key === 'p')) {
      event.preventDefault()
      recall(1)
    } else if (event.key === 'ArrowDown' || (ctrl && event.key === 'n')) {
      event.preventDefault()
      recall(-1)
    }
  }

  function oninput() {
    cycle = null
    walk = null
  }
</script>

<div class="cmdline">
  {#if shown.length > 0}
    <ul class="completions" bind:this={list}>
      {#each shown as item, index (item.value)}
        <li>
          <button
            class:current={cycle?.at === index}
            onclick={() => ((value = item.value), (cycle = null), input?.focus())}
          >
            <span class="name">{item.label}</span>
            {#if item.args}<span class="muted">{item.args}</span>{/if}
            {#if item.help}<span class="help">{item.help}</span>{/if}
          </button>
        </li>
      {/each}
    </ul>
  {/if}

  <form onsubmit={submit}>
    {#if isSearch}<span class="scope label">{global ? 'all spaces' : (workspace.space?.slug ?? '')}</span>{/if}
    <input bind:this={input} bind:value {onkeydown} {oninput} spellcheck="false" autocomplete="off" />
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
    /* Above the overlays, so `D` in the agenda can ask for a date. */
    z-index: 35;
    display: flex;
    flex-direction: column;
    background: color-mix(in oklab, var(--hull) 97%, transparent);
    border: 1px solid var(--surface-alt);
    border-bottom: 0;
    border-radius: var(--radius-l) var(--radius-l) 0 0;
    overflow: hidden;
    animation: rise var(--t-slow) var(--ease-snap) both;
  }

  form { display: flex; align-items: center; gap: 8px; padding: 6px; }
  .scope { flex: none; padding-left: 6px; font-size: 10px; color: var(--muted); }

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
  .completions button.current { background: var(--light); color: var(--ink); }
  .completions button.current .name,
  .completions button.current .help { color: var(--ink); }
  .name { color: var(--light); }
  .help { color: var(--subtle); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
