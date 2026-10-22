<script lang="ts">
  import type { Component } from 'svelte'
  import { openOverview } from './lib/actions'
  import { hooks } from './lib/commands'
  import { keysOf } from './lib/keymap'
  import { back, dispatch } from './lib/modes'
  import { embedSnippet } from './lib/markdown'
  import Agenda from './lib/Agenda.svelte'
  import Calendar from './lib/Calendar.svelte'
  import Bar from './lib/Bar.svelte'
  import Board from './lib/Board.svelte'
  import CommandLine from './lib/CommandLine.svelte'
  import DocumentPane from './lib/DocumentPane.svelte'
  import FilePane from './lib/FilePane.svelte'
  import Help from './lib/Help.svelte'
  import Info from './lib/Info.svelte'
  import Hints from './lib/Hints.svelte'
  import Verify from './lib/Verify.svelte'
  import Login from './lib/Login.svelte'
  import Overview from './lib/Overview.svelte'
  import Settings from './lib/Settings.svelte'
  import Trash from './lib/Trash.svelte'
  import CardTile from './lib/CardTile.svelte'
  import Scratch from './lib/Scratch.svelte'
  import Actions from './lib/Actions.svelte'
  import Prompt from './lib/Prompt.svelte'
  import SecretPrompt from './lib/SecretPrompt.svelte'
  import Rail from './lib/Rail.svelte'
  import Shared from './lib/Shared.svelte'
  import Sidebar from './lib/Sidebar.svelte'
  import StatusBar from './lib/StatusBar.svelte'
  import { workspace, type Sheet } from './lib/store.svelte'
  import { dwindle, isTouchDevice } from './lib/util'

  /*
   * A share link is /s/<token>#<key>. The fragment never leaves the browser, so
   * a crawler that only scraped the path cannot ask the server for the entry.
   */
  const shareToken = location.pathname.startsWith('/s/')
    ? decodeURIComponent(location.pathname.slice(3))
    : null
  const shareKey = shareToken ? decodeURIComponent(location.hash.slice(1)) : ''

  let cmdline = $state<string | null>(null)
  let cmdJump = $state(false)
  let fileInput = $state<HTMLInputElement>()
  let treeInput = $state<HTMLInputElement>()
  /** Whether the files being picked go into the vault — `:upload seal`. */
  let sealUpload = false
  let folderInput = $state<HTMLInputElement>()
  let importInput = $state<HTMLInputElement>()
  // Whether the next archive picked is poured in at the root, `:import zip /`.
  let importRoot = false
  let shell = $state<HTMLDivElement>()
  let overlayEl = $state<HTMLDivElement>()

  /** What each sheet is drawn with. */
  const sheets: Record<Sheet, Component> = {
    help: Help,
    overview: Overview,
    actions: Actions,
    verify: Verify,
    info: Info,
    trash: Trash,
    settings: Settings,
  }
  let boardEl = $state<HTMLDivElement>()
  /** The first half of `gg`, `dd`, `]]` …, shown until the second comes. */
  let pending = $state('')
  /** `F`: the pane whose links and buttons are wearing hints right now. */
  let hintRoot = $state<HTMLElement | null>(null)

  if (!shareToken) void workspace.boot()

  /* Where each tile sits, in the order of `workspace.tiles.all`. */
  const layout = $derived(dwindle(workspace.tiles.all.length))
  /*
   * The tiles are drawn in the order they were made, whatever order they are
   * in now, so moving one never moves its element — which would cost it its
   * scroll position, and a PDF its page.
   */
  const drawn = $derived([...workspace.tiles.all].sort((a, b) => a.id - b.id))

  $effect(() => {
    folderInput?.setAttribute('webkitdirectory', '')
  })

  $effect(() => {
    hooks.pickFiles = () => fileInput?.click()
    hooks.pickTreeFiles = (encrypt) => {
      sealUpload = encrypt
      treeInput?.click()
    }
    hooks.pickFolder = () => folderInput?.click()
    hooks.pickArchive = (root) => {
      importRoot = root
      importInput?.click()
    }
    hooks.focusList = () => shell?.focus()
    hooks.openCommandLine = (initial) => {
      cmdJump = false
      cmdline = initial
    }
    hooks.openOverview = () => void openOverview()
    hooks.openJump = () => {
      cmdJump = true
      cmdline = ''
    }
    hooks.closeCommandLine = () => {
      if (cmdline === null) return false
      cmdline = null
      shell?.focus()
      return true
    }
    hooks.showHints = (root) => (hintRoot = root)
    hooks.boardRoot = () => boardEl ?? null
  })

  /*
   * The overlay takes focus while it is up, so its keys (q, ?, esc) reach the
   * window handler instead of being swallowed by the editor underneath.
   */
  $effect(() => {
    if (workspace.sheets.shown !== 'none') {
      overlayEl?.focus()
      return () => (workspace.editing ? hooks.focusEditor() : shell?.focus())
    }
  })

  /*
   * A pending `[y/N]` takes the next key before anything else sees it — the
   * editor included, where `y` would otherwise start a yank. Only `y` agrees;
   * every other key, Escape among them, declines.
   */
  $effect(() => {
    const answer = (event: KeyboardEvent) => {
      const prompt = workspace.prompts.prompt
      if (!prompt || ['Shift', 'Control', 'Alt', 'Meta', 'CapsLock'].includes(event.key)) return
      event.preventDefault()
      event.stopPropagation()
      prompt.resolve(event.key === 'y' || event.key === 'Y')
    }
    window.addEventListener('keydown', answer, true)
    return () => window.removeEventListener('keydown', answer, true)
  })

  /*
   * The agenda is what the bar counts from, so it is kept fresh without being
   * asked: on coming back to the tab, and every few minutes while it is open
   * — which is also when a new day can make it open itself.
   */
  $effect(() => {
    if (!workspace.user) return
    const refresh = () => {
      if (document.visibilityState !== 'visible') return
      void workspace.cards.loadAgenda().then(() => workspace.cards.openAgendaIfDue())
      // The vault locks itself when left alone; the bar should say so.
      void workspace.vault.load()
    }
    const timer = setInterval(refresh, 5 * 60 * 1000)
    document.addEventListener('visibilitychange', refresh)
    return () => {
      clearInterval(timer)
      document.removeEventListener('visibilitychange', refresh)
    }
  })

  // The vault closes itself after a quiet spell; look again just after it
  // would have, so the bar does not go on saying it is open.
  $effect(() => {
    const vault = workspace.vault.status
    if (!vault?.unlocked || vault.locks_in === null) return
    const timer = setTimeout(() => void workspace.vault.load(), (vault.locks_in + 2) * 1000)
    return () => clearTimeout(timer)
  })

  // A first half left alone is forgotten, as vim's `timeoutlen` does.
  $effect(() => {
    if (!pending) return
    const timer = setTimeout(() => (pending = ''), 1000)
    return () => clearTimeout(timer)
  })

  /*
   * On a phone, back lets go of one thing at a time rather than leaving the
   * app. One entry is kept in the history over the page: back takes it off,
   * `back()` lets go of something, and it goes on again. With nothing left to
   * let go of it stays off, so the next back leaves. It is put on at a touch,
   * since a browser skips an entry made without one.
   */
  $effect(() => {
    if (!isTouchDevice || shareToken) return
    let guarded = false
    const guard = () => {
      if (guarded) return
      history.pushState({ guard: true }, '')
      guarded = true
    }
    const popstate = () => {
      guarded = false
      if (back()) guard()
      // The entry under the guard still has the address the page opened on.
      else workspace.tree.syncUrl(workspace.tree.path)
    }
    window.addEventListener('pointerdown', guard, true)
    window.addEventListener('popstate', popstate)
    return () => {
      window.removeEventListener('pointerdown', guard, true)
      window.removeEventListener('popstate', popstate)
    }
  })

  // Guard against losing a half-written entry to a stray refresh.
  $effect(() => {
    if (!workspace.tiles.all.some((tile) => tile.dirty)) return
    const warn = (event: BeforeUnloadEvent) => event.preventDefault()
    window.addEventListener('beforeunload', warn)
    return () => window.removeEventListener('beforeunload', warn)
  })

  async function attach(event: Event) {
    const input = event.target as HTMLInputElement
    const files = Array.from(input.files ?? [])
    input.value = ''
    if (files.length === 0) return
    await workspace.guard(async () => {
      // While writing, they go in at the cursor too; while reading, the
      // document only holds them.
      const writing = workspace.editing
      const created = await workspace.tree.attach(files)
      if (writing && created.length > 0) hooks.insertText(created.map(embedSnippet).join('\n\n'))
    })
  }

  async function put(event: Event) {
    const input = event.target as HTMLInputElement
    const files = Array.from(input.files ?? [])
    input.value = ''
    if (files.length === 0) return
    await workspace.guard(() => workspace.tree.putFiles(files, sealUpload))
  }

  /*
   * A folder upload is the same multipart body as any other, except that each
   * file carries the path it had inside the folder — which is what makes the
   * tree on the other side possible.
   */
  async function importFolder(event: Event) {
    const input = event.target as HTMLInputElement
    const files = Array.from(input.files ?? [])
    input.value = ''
    const root = importRoot
    importRoot = false
    if (files.length === 0) return
    await workspace.guard(() => workspace.tree.importFolder(files, root))
  }

  /*
   * Files let go anywhere but the list or the editor would make the browser
   * open them in place of the app. The places that take files say so
   * themselves; everywhere else refuses the drop, and says where it goes.
   */
  function strayDrop(event: DragEvent) {
    if (event.defaultPrevented || !event.dataTransfer?.types.includes('Files')) return
    event.preventDefault()
    event.dataTransfer.dropEffect = 'none'
    if (event.type === 'drop') workspace.say('drop files on the list to put them in this folder, or into the editor to embed them', 'error')
  }

  // What the empty pane offers, in the key/value shape of a status readout.
  // Keys are keycaps, so `o` and `O` cannot be read as the same key.
  const offers: [string[], string][] = [
    [keysOf('tree', 'new'), 'write a new document'],
    [keysOf('tree', 'mkdir'), 'make a folder'],
    [[...keysOf('tree', 'down'), ...keysOf('tree', 'up')], 'browse, l opens, L opens beside'],
    [keysOf('global', 'space'), 'switch space, Tab cycles'],
    [keysOf('global', 'overview'), 'overview of every space'],
    [keysOf('global', 'plan'), 'the plan: board, agenda, calendar'],
    [[...keysOf('global', 'board'), ...keysOf('global', 'agenda'), ...keysOf('global', 'calendar')], 'one of them, on the plan'],
    [keysOf('tree', 'capture'), 'capture a card to the inbox'],
    [keysOf('global', 'find'), 'go to anything by its name'],
    [keysOf('tree', 'search'), 'search this space, Tab every space'],
    [keysOf('global', 'help'), 'help'],
  ]

  function isTypingTarget(target: EventTarget | null) {
    const node = target as HTMLElement | null
    return (
      !!node &&
      (node.tagName === 'INPUT' ||
        node.tagName === 'TEXTAREA' ||
        node.isContentEditable ||
        !!node.closest?.('.cm-editor'))
    )
  }

  function onkeydown(event: KeyboardEvent) {
    pending = dispatch(event, pending, cmdline !== null, isTypingTarget(event.target))
  }
</script>

<svelte:window {onkeydown} ondragover={strayDrop} ondrop={strayDrop} />

{#if shareToken}
  <Shared token={shareToken} shareKey={shareKey} />
{:else if workspace.booting}
  <div class="boot label">booting ~/workspace<span class="cursor">_</span></div>
{:else if !workspace.user}
  <Login />
{:else}
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div class="shell" bind:this={shell} tabindex="-1">
    <Bar />
    <Rail />

    <!-- The opening in the frame: everything that scrolls is seen through it. -->
    <main class="porthole">
      <!-- Keyed on the space, so a switch slides the new one in, as the desktop
           does between workspaces: from below when moving right, above when left. -->
      {#key workspace.tree.spaceId}
        <div
          class="panes switch"
          class:reading={workspace.tiles.all.some((tile) => !tile.empty)}
          class:full={workspace.fullscreen}
          class:plan={workspace.tiles.planning}
          class:focus-doc={workspace.focus !== 'list'}
          style="--dir: {workspace.switchDirection}"
        >
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div
            class="window list"
            class:focused={workspace.focus === 'list'}
            onpointerdown={() => (workspace.pane = 'list')}
          ><Sidebar /></div>
          <div class="tiles" class:only={workspace.tiles.all.length === 1}>
            {#each drawn as tile (tile.id)}
              {@const index = workspace.tiles.all.indexOf(tile)}
              {@const rect = layout[index]}
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div
                class="window entry"
                class:view={tile.kind !== 'node'}
                data-kind={tile.kind}
                class:current={index === workspace.tiles.focused}
                class:focused={!tile.empty && index === workspace.tiles.focused && workspace.pane === 'document'}
                style="--x: {rect.x}; --y: {rect.y}; --w: {rect.w}; --h: {rect.h}"
                onpointerdown={() => !tile.empty && workspace.tiles.focusOn(index)}
                onfocusin={() => !tile.empty && index !== workspace.tiles.focused && workspace.tiles.focusOn(index)}
              >
                {#if tile.kind === 'board'}
                  <div class="fill" bind:this={boardEl}><Board /></div>
                {:else if tile.kind === 'agenda'}
                  <Agenda />
                {:else if tile.kind === 'calendar'}
                  <Calendar />
                {:else if tile.kind === 'card'}
                  <CardTile />
                {:else if tile.open?.kind === 'file'}
                  <FilePane {tile} />
                {:else if tile.open}
                  <DocumentPane {tile} />
                {:else}
                  <div class="placeholder">
                    <div class="standby resolve">
                      <h2 class="label rule">nothing open</h2>
                      <dl>
                        {#each offers as [keys, what] (what)}
                          <dt>{#each keys as key (key)}<span class="kbd">{key}</span>{/each}</dt>
                          <dd>{what}</dd>
                        {/each}
                        <dt>status</dt>
                        <dd class="light">standing by<span class="cursor">_</span></dd>
                      </dl>
                    </div>
                  </div>
                {/if}
              </div>
            {/each}
          </div>
        </div>
      {/key}
    </main>

    <StatusBar {pending} />

    {#if cmdline !== null}
      <CommandLine initial={cmdline} jump={cmdJump} close={() => ((cmdline = null), (cmdJump = false))} />
    {/if}

    {#if workspace.scratch.open}
      <Scratch />
    {/if}

    <Prompt />
    <SecretPrompt />

    {#if hintRoot}
      <Hints root={hintRoot} close={() => (hintRoot = null)} />
    {/if}

    {#if workspace.sheets.shown !== 'none'}
      {@const Shown = sheets[workspace.sheets.shown]}
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions, a11y_no_noninteractive_tabindex -->
      <div class="overlay" tabindex="-1" bind:this={overlayEl} onclick={(e) => e.target === e.currentTarget && workspace.sheets.close()}>
        <Shown />
      </div>
    {/if}
  </div>
{/if}

<input class="hidden" type="file" multiple bind:this={fileInput} onchange={attach} />
<input class="hidden" type="file" multiple bind:this={treeInput} onchange={put} />
<!-- `webkitdirectory` is set from script: it is the one attribute browsers
     agree on for picking a folder, and no typed attribute list carries it. -->
<input class="hidden" type="file" multiple bind:this={folderInput} onchange={importFolder} />
<!-- The same import, one file at a time: a .zip is expanded, anything else
     lands as the document or media it is. Picking a folder is the only thing
     the folder input above can do, so single files need an input of their own. -->
<input class="hidden" type="file" multiple bind:this={importInput} onchange={importFolder} />

<style>
  .boot { display: grid; place-items: center; height: 100%; }

  .fill { height: 100%; }
  /* The focused view's key is lit, as its border is. */
  .tiles > .window.focused :global(.tile-key) { background: var(--light); border-color: var(--light); color: var(--ink); }

  /* One cursor is lit at a time: the views without the focus keep theirs, dimmed. */
  .view:not(.focused) :global(:is(.card.active, .row.at, .day.at, li.at)) {
    background: var(--surface-alt);
    color: var(--light);
  }
  .view:not(.focused) :global(.row.at :is(.where, .due)) { color: var(--muted); }
  .view:not(.focused) :global(.card.active :is(.meta, .doc)) { color: var(--muted); }
  .view:not(.focused) :global(:is(.card.active .today, .today .row.at .due)) { color: var(--light); }
  .view:not(.focused) :global(:is(.card.active .overdue, .overdue .row.at .due)) { color: var(--alarm); }
  .view:not(.focused) :global(:is(.card.active, li.at) .tick) { border-color: var(--outline); }
  .view:not(.focused) :global(.day.at :is(.dot)) { background: var(--light); }
  .view:not(.focused) :global(.day.at .many) { color: var(--light); }

  .window.list { --pane-bg: color-mix(in oklab, var(--surface) 60%, var(--base)); }

  /* Only one pane on screen: nothing to tell apart, so no active border. */
  .panes:not(.reading) .window.list {
    background:
      linear-gradient(var(--pane-bg), var(--pane-bg)) padding-box,
      linear-gradient(var(--overlay), var(--overlay)) border-box;
    box-shadow: none;
  }
  .panes:not(.reading) .window.entry { border-color: transparent; background: transparent; }

  /*
   * The tiles share the column right of the list, each placed by the dwindle
   * layout. Half a gap is taken off each side of a tile, and the column
   * reaches half a gap out to make up for it at the edges.
   */
  .tiles { position: relative; min-width: 0; min-height: 0; margin: calc(var(--gap) / -2); }
  .tiles > .window {
    position: absolute;
    left: calc(var(--x) * 100% + var(--gap) / 2);
    top: calc(var(--y) * 100% + var(--gap) / 2);
    width: calc(var(--w) * 100% - var(--gap));
    height: calc(var(--h) * 100% - var(--gap));
  }

  /* The plan screen: the views alone, with no list beside them. */
  .panes.plan { grid-template-columns: 1fr; }
  .panes.plan .window.list { display: none; }

  /* `f`: the focused pane fills the porthole — the list, or one tile. */
  .panes.full { grid-template-columns: 1fr; }
  .panes.full.focus-doc .window.list,
  .panes.full:not(.focus-doc) .tiles,
  .panes.full .tiles > .window:not(.current) { display: none; }
  .panes.full .tiles > .window.current { inset: calc(var(--gap) / 2); width: auto; height: auto; }

  .placeholder { display: grid; place-items: center; height: 100%; padding: 24px; }
  .standby { width: min(100%, 44ch); }
  .standby h2 { margin: 0 0 18px; font-weight: 400; }
  .standby dl {
    display: grid;
    grid-template-columns: 7rem 1fr;
    gap: 8px 0;
    align-items: center;
    margin: 0;
    font-size: 12px;
  }
  .standby dt { display: flex; gap: 4px; align-items: center; }
  .standby dt:last-of-type { color: var(--muted); letter-spacing: 0.18em; text-transform: uppercase; }
  .standby dd { margin: 0; color: var(--fg); }
  .standby dd.light { color: var(--light); }

  .overlay {
    position: absolute;
    inset: 0;
    z-index: 30;
    display: grid;
    place-items: center;
    padding: calc(var(--bar-h) + 8px) 24px 24px;
    background: color-mix(in oklab, var(--ink) 72%, transparent);
    animation: fade var(--t-base) var(--ease-glide) both;
  }

  @keyframes fade { from { opacity: 0; } }

  .hidden { display: none; }

  /* A phone shows one window: the list, or the focused tile. */
  @media (max-width: 767px) {
    .panes { grid-template-columns: 1fr; }
    .panes .tiles { display: none; }
    .panes.reading .list { display: none; }
    .panes.reading .tiles { display: block; }
    .tiles > .window:not(.current) { display: none; }
    .tiles > .window.current { inset: calc(var(--gap) / 2); width: auto; height: auto; }
    .overlay { padding: 12px; }
  }
</style>
