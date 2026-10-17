<script lang="ts">
  import { hooks, runCommand } from './lib/commands'
  import { embedSnippet } from './lib/markdown'
  import Bar from './lib/Bar.svelte'
  import Board from './lib/Board.svelte'
  import CommandLine from './lib/CommandLine.svelte'
  import DocumentPane from './lib/DocumentPane.svelte'
  import Help from './lib/Help.svelte'
  import Login from './lib/Login.svelte'
  import Overview from './lib/Overview.svelte'
  import MediaBrowser from './lib/MediaBrowser.svelte'
  import Rail from './lib/Rail.svelte'
  import Shared from './lib/Shared.svelte'
  import Sidebar from './lib/Sidebar.svelte'
  import StatusBar from './lib/StatusBar.svelte'
  import { workspace } from './lib/store.svelte'

  /*
   * A share link is /s/<token>#<key>. The fragment never leaves the browser, so
   * a crawler that only scraped the path cannot ask the server for the entry.
   */
  const shareToken = location.pathname.startsWith('/s/')
    ? decodeURIComponent(location.pathname.slice(3))
    : null
  const shareKey = shareToken ? decodeURIComponent(location.hash.slice(1)) : ''

  let cmdline = $state<string | null>(null)
  let fileInput = $state<HTMLInputElement>()
  let folderInput = $state<HTMLInputElement>()
  let importInput = $state<HTMLInputElement>()
  let shell = $state<HTMLDivElement>()
  let overlayEl = $state<HTMLDivElement>()
  let pending = $state('')

  if (!shareToken) void workspace.boot()

  $effect(() => {
    folderInput?.setAttribute('webkitdirectory', '')
  })

  $effect(() => {
    hooks.pickFiles = () => fileInput?.click()
    hooks.pickFolder = () => folderInput?.click()
    hooks.pickArchive = () => importInput?.click()
    hooks.focusList = () => shell?.focus()
    hooks.openCommandLine = (initial) => (cmdline = initial)
    hooks.openOverview = () => void openOverview()
  })

  /*
   * The overlay takes focus while it is up, so its keys (q, ?, esc) reach the
   * window handler instead of being swallowed by the editor underneath.
   */
  $effect(() => {
    if (workspace.overlay !== 'none') {
      overlayEl?.focus()
      return () => (workspace.editing ? hooks.focusEditor() : shell?.focus())
    }
  })

  // Guard against losing a half-written entry to a stray refresh.
  $effect(() => {
    if (!workspace.dirty) return
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
      const uploaded = await workspace.upload(files)
      hooks.insertText(uploaded.map(embedSnippet).join('\n\n'))
    })
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
    if (files.length === 0) return
    await workspace.guard(() => workspace.importFolder(files))
  }

  // What the empty pane offers, in the key/value shape of a status readout.
  const offers: [string, string][] = [
    ['o', 'write a new document'],
    ['O', 'make a folder'],
    ['j / k', 'browse, l opens'],
    ['1 … 9', 'switch space, Tab cycles'],
    ['w', 'overview of every space'],
    ['b', "this space's board"],
    ['/', 'search this space'],
    ['?', 'help'],
  ]

  /*
   * Spaces are workspaces: digits jump to one, shifted digits send the node
   * there, Tab walks along the bar. `event.code` rather than `event.key`, so
   * Shift+1 means space 1 whatever symbol the layout puts on that key.
   */
  function spaceKey(event: KeyboardEvent): boolean {
    const digit = /^Digit(\d)$/.exec(event.code)?.[1]
    if (digit !== undefined) {
      const index = digit === '0' ? 9 : Number(digit) - 1
      void workspace.guard(() =>
        event.shiftKey ? workspace.sendToSpace(index) : workspace.switchSpace(index),
      )
      return true
    }
    if (event.key === 'Tab') {
      void workspace.guard(() => workspace.cycleSpace(event.shiftKey ? -1 : 1))
      return true
    }
    if (event.key === 'w') {
      void openOverview()
      return true
    }
    return false
  }

  async function openOverview() {
    await workspace.guard(() => workspace.loadOverview())
    workspace.overviewCursor = Math.max(
      workspace.overview.findIndex((entry) => entry.space.id === workspace.spaceId),
      0,
    )
    workspace.overlay = 'overview'
  }

  /** The overview's own keys: hjkl across the grid, a digit or Enter to go. */
  function overviewKey(event: KeyboardEvent): boolean {
    const count = workspace.overview.length
    const columns = hooks.overviewColumns()
    const go = (index: number) => {
      const entry = workspace.overview[index]
      if (!entry) return
      workspace.overlay = 'none'
      void workspace.guard(() => workspace.enterSpace(entry.space.id))
    }
    const digit = /^Digit(\d)$/.exec(event.code)?.[1]
    if (digit !== undefined) {
      go(digit === '0' ? 9 : Number(digit) - 1)
      return true
    }
    const step: Record<string, number> = { h: -1, ArrowLeft: -1, l: 1, ArrowRight: 1, k: -columns, ArrowUp: -columns, j: columns, ArrowDown: columns }
    if (event.key in step) {
      workspace.overviewCursor = Math.min(Math.max(workspace.overviewCursor + step[event.key], 0), count - 1)
      return true
    }
    if (event.key === 'Enter') {
      go(workspace.overviewCursor)
      return true
    }
    if (event.key === 'w' || event.key === 'q') {
      workspace.overlay = 'none'
      return true
    }
    return false
  }

  /** Alt+h / Alt+l: focus moves between the panes, out of the editor too. */
  function focusPane(pane: 'list' | 'document') {
    if (pane === 'document' && !workspace.open) return
    workspace.pane = pane
    if (pane === 'document' && workspace.editing) hooks.focusEditor()
    else shell?.focus()
  }

  /*
   * The board reads the same keys as the list, one dimension wider: h/l walk
   * the lists, j/k the cards, and the shifted pair takes the card along. Only
   * `:` and `?` fall through to the keys below, because everything else would
   * act on a tree that is not on screen.
   */
  function boardKey(key: string): boolean {
    switch (key) {
      case 'h':
      case 'ArrowLeft':
        workspace.moveList(-1)
        return true
      case 'l':
      case 'ArrowRight':
        workspace.moveList(1)
        return true
      case 'j':
      case 'ArrowDown':
        workspace.moveCard(1)
        return true
      case 'k':
      case 'ArrowUp':
        workspace.moveCard(-1)
        return true
      case 'J':
        void workspace.guard(() => workspace.nudgeCard(1))
        return true
      case 'K':
        void workspace.guard(() => workspace.nudgeCard(-1))
        return true
      case 'H':
        void workspace.guard(() => workspace.sendCard(-1))
        return true
      case 'L':
        void workspace.guard(() => workspace.sendCard(1))
        return true
      case 'G':
        workspace.cardCursor = Math.max((workspace.list?.cards.length ?? 0) - 1, 0)
        return true
      case 'Enter':
        void workspace.guard(() => workspace.openCard())
        return true
      case 'o':
        cmdline = ':card '
        return true
      case 'R':
        cmdline = ':name '
        return true
      case 't':
      case ' ':
        void workspace.guard(() => workspace.toggleDone())
        return true
      case 'x':
        void runCommand('delete')
        return true
      case 'r':
        void workspace.guard(() => workspace.reloadBoard())
        return true
      case 'q':
      case 'Escape':
        workspace.closeBoard()
        return true
      default:
        return false
    }
  }

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
    if (event.key === 's' && (event.metaKey || event.ctrlKey)) {
      event.preventDefault()
      if (workspace.open) void workspace.guard(() => workspace.save())
      return
    }

    if (event.altKey && !event.ctrlKey && !event.metaKey && (event.key === 'h' || event.key === 'l')) {
      event.preventDefault()
      focusPane(event.key === 'h' ? 'list' : 'document')
      return
    }

    if (workspace.overlay === 'overview' && !event.ctrlKey && !event.metaKey && !event.altKey) {
      if (overviewKey(event)) {
        event.preventDefault()
        return
      }
    }

    if (event.key === 'Escape') {
      if (workspace.overlay !== 'none') {
        workspace.overlay = 'none'
        return
      }
      if (cmdline !== null) {
        cmdline = null
        shell?.focus()
        return
      }
      if (workspace.fullscreen && !workspace.editing) {
        workspace.fullscreen = false
        return
      }
    }

    if (workspace.overlay !== 'none') {
      if (event.key === 'q' || event.key === '?') {
        event.preventDefault()
        workspace.overlay = 'none'
      }
      return
    }

    if (cmdline !== null || isTypingTarget(event.target)) return

    // Ctrl-D / Ctrl-U page the open document, as they do in a pager.
    if (event.ctrlKey && !event.metaKey && !event.altKey && (event.key === 'd' || event.key === 'u')) {
      if (workspace.open && !workspace.editing) {
        event.preventDefault()
        hooks.scrollReader(event.key === 'd' ? 'halfdown' : 'halfup')
        return
      }
    }

    if (event.ctrlKey || event.metaKey || event.altKey) return

    const key = event.key
    const previous = pending
    pending = ''

    if (spaceKey(event)) {
      event.preventDefault()
      return
    }
    if (key === 'f') {
      event.preventDefault()
      workspace.fullscreen = !workspace.fullscreen
      return
    }

    if (workspace.view === 'board') {
      if (previous === 'g' && key === 'g') {
        event.preventDefault()
        workspace.cardCursor = 0
        return
      }
      if (key === 'g') {
        pending = key
        return
      }
      if (boardKey(key)) {
        event.preventDefault()
        return
      }
    }

    /*
     * The focused pane owns j/k. In the document they scroll what is on
     * screen; `h` moves focus to the list with the document still open, where
     * they walk the rows again, and `l` goes back.
     */
    const reading = !!workspace.open && !workspace.editing && workspace.pane === 'document'

    // Two-key sequences: gg and dd.
    if (previous === 'g' && key === 'g') {
      event.preventDefault()
      if (reading) hooks.scrollReader('top')
      else workspace.cursor = 0
      return
    }
    if (previous === 'd' && key === 'd') {
      event.preventDefault()
      void runCommand('d')
      return
    }
    if (key === 'g' || key === 'd') {
      pending = key
      return
    }

    switch (key) {
      case 'j':
      case 'ArrowDown':
        event.preventDefault()
        if (reading) hooks.scrollReader(1)
        else workspace.move(1)
        break
      case 'k':
      case 'ArrowUp':
        event.preventDefault()
        if (reading) hooks.scrollReader(-1)
        else workspace.move(-1)
        break
      case 'G':
        event.preventDefault()
        if (reading) hooks.scrollReader('bottom')
        else workspace.cursor = Math.max(workspace.nodes.length - 1, 0)
        break
      case 'Enter':
      case 'l':
      case 'ArrowRight':
        event.preventDefault()
        if (workspace.open && workspace.selected?.id === workspace.open.id) focusPane('document')
        else void workspace.guard(() => workspace.openSelected())
        break
      case 'm':
        // Pick a node up here, walk to a folder, put it down with `p`.
        event.preventDefault()
        workspace.mark()
        break
      case 'p':
        event.preventDefault()
        void workspace.guard(() => workspace.drop())
        break
      case 'o':
        event.preventDefault()
        void runCommand('new')
        break
      case 'O':
        // The name has to come from somewhere, and the command line is where
        // names are typed — this only saves typing `:mkdir `.
        event.preventDefault()
        cmdline = ':mkdir '
        break
      case 'i':
      case 'a':
        event.preventDefault()
        void runCommand('edit')
        break
      case 'b':
        // The board is a view of the space, not a row in the list, so it needs
        // a key as plain as the one that opens a document.
        event.preventDefault()
        void runCommand('board')
        break
      case 'R':
        // Renaming needs a name typed, and names are typed on the command line.
        event.preventDefault()
        cmdline = ':name '
        break
      case 'h':
      case 'ArrowLeft':
        // Leftwards at every depth: from the document to the list, from the
        // list up a level.
        event.preventDefault()
        if (workspace.open && workspace.pane === 'document') focusPane('list')
        else void workspace.guard(() => workspace.up())
        break
      case 'q':
      case 'Escape':
        event.preventDefault()
        if (workspace.open) void runCommand('q')
        break
      case 'x':
        event.preventDefault()
        void runCommand('d')
        break
      case 's':
        event.preventDefault()
        void runCommand(workspace.selected?.shared ? 'unshare' : 'share')
        break
      case 'y':
        event.preventDefault()
        void runCommand('link')
        break
      case 'n':
        event.preventDefault()
        if (workspace.query) void workspace.guard(() => workspace.search(''))
        break
      case '/':
        event.preventDefault()
        cmdline = '/'
        break
      case ':':
        event.preventDefault()
        cmdline = ':'
        break
      case '?':
        event.preventDefault()
        void runCommand('help')
        break
    }
  }
</script>

<svelte:window {onkeydown} />

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
      {#key workspace.spaceId}
      {#if workspace.view === 'board'}
        <!-- The board takes the whole width: its columns are the two panes. -->
        <div class="panes board switch" style="--dir: {workspace.switchDirection}">
          <div class="window focused"><Board /></div>
        </div>
      {:else}
        <div
          class="panes switch"
          class:reading={!!workspace.open}
          class:full={workspace.fullscreen}
          class:focus-doc={!!workspace.open && workspace.pane === 'document'}
          style="--dir: {workspace.switchDirection}"
        >
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div
            class="window list"
            class:focused={!workspace.open || workspace.pane === 'list'}
            onpointerdown={() => (workspace.pane = 'list')}
          ><Sidebar /></div>
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div
            class="window entry"
            class:focused={!!workspace.open && workspace.pane === 'document'}
            onpointerdown={() => workspace.open && (workspace.pane = 'document')}
          >
            {#if workspace.open}
              <DocumentPane />
            {:else}
              <div class="placeholder">
                <div class="standby resolve">
                  <h2 class="label rule">nothing open</h2>
                  <dl>
                    {#each offers as [key, what] (key)}
                      <dt>{key}</dt>
                      <dd>{what}</dd>
                    {/each}
                    <dt>status</dt>
                    <dd class="light">standing by<span class="cursor">_</span></dd>
                  </dl>
                </div>
              </div>
            {/if}
          </div>
        </div>
      {/if}
      {/key}
    </main>

    <StatusBar />

    {#if cmdline !== null}
      <CommandLine initial={cmdline} close={() => (cmdline = null)} />
    {/if}

    {#if workspace.overlay !== 'none'}
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions, a11y_no_noninteractive_tabindex -->
      <div class="overlay" tabindex="-1" bind:this={overlayEl} onclick={(e) => e.target === e.currentTarget && (workspace.overlay = 'none')}
      >
        {#if workspace.overlay === 'help'}<Help />{:else if workspace.overlay === 'overview'}<Overview />{:else}<MediaBrowser />{/if}
      </div>
    {/if}
  </div>
{/if}

<input class="hidden" type="file" multiple bind:this={fileInput} onchange={attach} />
<!-- `webkitdirectory` is set from script: it is the one attribute browsers
     agree on for picking a folder, and no typed attribute list carries it. -->
<input class="hidden" type="file" multiple bind:this={folderInput} onchange={importFolder} />
<!-- The same import, one file at a time: a .zip is expanded, anything else
     lands as the document or media it is. Picking a folder is the only thing
     the folder input above can do, so single files need an input of their own. -->
<input class="hidden" type="file" multiple bind:this={importInput} onchange={importFolder} />

<style>
  .boot { display: grid; place-items: center; height: 100%; }

  .shell {
    position: relative;
    height: 100%;
    outline: none;
    background: var(--hull);
  }

  /*
   * The porthole: a rounded opening in the hull, outlined by a single rim. The
   * rim is drawn on top by ::after, so the panes scroll underneath it the way
   * windows sit under the frame on the desktop.
   */
  .porthole {
    position: absolute;
    top: var(--bar-h);
    right: var(--rail-w);
    bottom: var(--status-h);
    left: var(--frame);
    overflow: hidden;
    border-radius: var(--frame-radius);
    background:
      radial-gradient(120% 70% at 30% 0%, #141713 0%, transparent 60%),
      var(--base);
  }
  .porthole::after {
    content: '';
    position: absolute;
    inset: 0;
    border-radius: inherit;
    box-shadow: inset 0 0 0 1px var(--rim);
    pointer-events: none;
    z-index: 5;
  }

  /*
   * The panes tile the porthole like windows tile a workspace: a gap round
   * and between them, each rounded to sit concentric with the frame.
   */
  .panes {
    --gap: 7px;
    height: 100%;
    display: grid;
    grid-template-columns: minmax(32ch, 24vw) 1fr;
    gap: var(--gap);
    padding: var(--gap);
  }

  .panes.board { grid-template-columns: 1fr; }

  /*
   * A window: opaque, a hairline border in overlay. The focused one gets the
   * desktop's active border — a light gradient at 135° — and its shadow.
   */
  .window {
    --pane-bg: color-mix(in oklab, var(--surface) 40%, var(--base));
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    border: 1px solid transparent;
    border-radius: calc(var(--frame-radius) - var(--gap));
    background:
      linear-gradient(var(--pane-bg), var(--pane-bg)) padding-box,
      linear-gradient(var(--overlay), var(--overlay)) border-box;
    transition: box-shadow var(--t-base) var(--ease-glide);
  }
  .window.list { --pane-bg: color-mix(in oklab, var(--surface) 60%, var(--base)); }
  .window.focused {
    background:
      linear-gradient(var(--pane-bg), var(--pane-bg)) padding-box,
      linear-gradient(135deg, var(--light), var(--light-dim)) border-box;
    box-shadow: 0 6px 24px rgb(0 0 0 / 0.45);
  }
  /* Only one pane on screen: nothing to tell apart, so no active border. */
  .panes:not(.reading) .window.list,
  .panes.board .window {
    background:
      linear-gradient(var(--pane-bg), var(--pane-bg)) padding-box,
      linear-gradient(var(--overlay), var(--overlay)) border-box;
    box-shadow: none;
  }
  .panes:not(.reading) .window.entry { border-color: transparent; background: transparent; }

  /* `f`: the focused pane fills the porthole. */
  .panes.full { grid-template-columns: 1fr; }
  .panes.full .window:not(.focused) { display: none; }
  .panes.full:not(.reading) .window.entry { display: none; }

  /* hypr: workspaces, slidefadevert 18%, 550ms on the drift curve. */
  .switch { animation: slidefadevert 550ms var(--ease-drift) both; }
  @keyframes slidefadevert {
    from { opacity: 0; transform: translateY(calc(var(--dir, 1) * 18%)); }
  }

  .placeholder { display: grid; place-items: center; height: 100%; padding: 24px; }
  .standby { width: min(100%, 44ch); }
  .standby h2 { margin: 0 0 18px; font-weight: 400; }
  .standby dl {
    display: grid;
    grid-template-columns: 7rem 1fr;
    gap: 6px 0;
    margin: 0;
    font-size: 12px;
  }
  /* Keys keep their case: `o` and `O` are different keys. */
  .standby dt { color: var(--light); letter-spacing: 0.08em; }
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

  @media (max-width: 760px) {
    .panes { grid-template-columns: 1fr; --gap: 4px; }
    .panes .entry { display: none; }
    .panes.reading .list { display: none; }
    .panes.reading .entry { display: block; }
    .overlay { padding: 12px; }
  }
</style>
