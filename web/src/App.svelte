<script lang="ts">
  import { hooks, runCommand } from './lib/commands'
  import { embedSnippet } from './lib/markdown'
  import Agenda from './lib/Agenda.svelte'
  import Bar from './lib/Bar.svelte'
  import Board from './lib/Board.svelte'
  import CommandLine from './lib/CommandLine.svelte'
  import DocumentPane from './lib/DocumentPane.svelte'
  import FilePane from './lib/FilePane.svelte'
  import Help from './lib/Help.svelte'
  import Verify from './lib/Verify.svelte'
  import Login from './lib/Login.svelte'
  import Overview from './lib/Overview.svelte'
  import MediaBrowser from './lib/MediaBrowser.svelte'
  import Actions from './lib/Actions.svelte'
  import Prompt from './lib/Prompt.svelte'
  import SecretPrompt from './lib/SecretPrompt.svelte'
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
  let cmdGlobal = $state(false)
  let fileInput = $state<HTMLInputElement>()
  let treeInput = $state<HTMLInputElement>()
  /** Whether the files being picked go into the vault — `:upload!`. */
  let sealUpload = false
  let folderInput = $state<HTMLInputElement>()
  let importInput = $state<HTMLInputElement>()
  // Whether the next archive picked is poured in at the root, `:import! /`.
  let importRoot = false
  let shell = $state<HTMLDivElement>()
  let overlayEl = $state<HTMLDivElement>()
  let pending = $state('')

  if (!shareToken) void workspace.boot()

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
      cmdGlobal = false
      cmdline = initial
    }
    hooks.openOverview = () => void openOverview()
    hooks.openGlobalSearch = () => {
      cmdGlobal = true
      cmdline = '/'
    }
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

  /*
   * A pending `[y/N]` takes the next key before anything else sees it — the
   * editor included, where `y` would otherwise start a yank. Only `y` agrees;
   * every other key, Escape among them, declines.
   */
  $effect(() => {
    const answer = (event: KeyboardEvent) => {
      const prompt = workspace.prompt
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
      void workspace.loadAgenda().then(() => workspace.openAgendaIfDue())
      // The vault locks itself when left alone; the bar should say so.
      void workspace.loadVault()
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
    const vault = workspace.vault
    if (!vault?.unlocked || vault.locks_in === null) return
    const timer = setTimeout(() => void workspace.loadVault(), (vault.locks_in + 2) * 1000)
    return () => clearTimeout(timer)
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

  async function put(event: Event) {
    const input = event.target as HTMLInputElement
    const files = Array.from(input.files ?? [])
    input.value = ''
    if (files.length === 0) return
    await workspace.guard(() => workspace.putFiles(files, sealUpload))
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
    await workspace.guard(() => workspace.importFolder(files, root))
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
    [['o'], 'write a new document'],
    [['⇧ O'], 'make a folder'],
    [['j', 'k'], 'browse, l opens'],
    [['1…9'], 'switch space, Tab cycles'],
    [['w'], 'overview of every space'],
    [['b'], "this space's board"],
    [['t'], 'agenda — late, due, in hand'],
    [['c'], 'capture a card to the inbox'],
    [['⌃ P'], 'find in every space'],
    [['/'], 'search this space'],
    [['?'], 'help'],
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

  /** The agenda's keys: j/k down the list, Enter to the card, Space finishes it. */
  function agendaKey(event: KeyboardEvent): boolean {
    const cards = workspace.agendaCards
    const card = cards[workspace.agendaCursor]
    switch (event.key) {
      case 'j':
      case 'ArrowDown':
        workspace.agendaCursor = Math.min(workspace.agendaCursor + 1, Math.max(cards.length - 1, 0))
        return true
      case 'k':
      case 'ArrowUp':
        workspace.agendaCursor = Math.max(workspace.agendaCursor - 1, 0)
        return true
      case 'Enter':
        if (card) void workspace.guard(() => workspace.goToCard(card))
        return true
      case ' ':
        if (card) void workspace.guard(() => workspace.finishCard(card))
        return true
      case 'T':
        workspace.overlay = 'none'
        void runCommand('today')
        return true
      case 't':
      case 'q':
        workspace.overlay = 'none'
        return true
      default:
        return false
    }
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
   * the lists, j/k the cards, and the shifted pair takes the card along.
   * Nothing falls through to the tree keys, because they would act on a tree
   * that is not on screen.
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
      case 'c':
        cmdline = ':card '
        return true
      case 'R':
        // On an empty list there is no card to rename, so it is the list.
        cmdline = workspace.card ? ':name ' : ':name! '
        return true
      case 'D':
        cmdline = ':due '
        return true
      case 'z':
        workspace.showOldDone = !workspace.showOldDone
        return true
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
    if (workspace.overlay === 'agenda' && !event.ctrlKey && !event.metaKey && !event.altKey) {
      if (agendaKey(event)) {
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

    // Ctrl-P: find anything, in any space — the one search that is not
    // confined to where you are.
    if (event.ctrlKey && !event.metaKey && !event.altKey && event.key === 'p') {
      event.preventDefault()
      cmdGlobal = true
      cmdline = '/'
      return
    }

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
    // The agenda and today's entry are a key away from anywhere, the board
    // included: they are the two questions asked most.
    if (key === 't') {
      event.preventDefault()
      void runCommand('agenda')
      return
    }
    if (key === 'T') {
      event.preventDefault()
      void runCommand('today')
      return
    }

    if (workspace.view === 'board') {
      if (previous === 'g' && key === 'g') {
        event.preventDefault()
        workspace.cardCursor = 0
        return
      }
      if (previous === 'd' && key === 'd') {
        event.preventDefault()
        void runCommand('delete')
        return
      }
      if (key === 'g' || key === 'd') {
        pending = key
        return
      }
      if (boardKey(key)) {
        event.preventDefault()
        return
      }
      if (key === ':' || key === '?') {
        event.preventDefault()
        if (key === ':') {
          cmdGlobal = false
          cmdline = ':'
        }
        else void runCommand('help')
      }
      return
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
        // Pick nodes up here, walk to a folder, put them down with `p`.
        event.preventDefault()
        workspace.mark()
        break
      case 'M':
        event.preventDefault()
        workspace.markAll()
        break
      case 'p':
        event.preventDefault()
        void workspace.guard(() => workspace.drop())
        break
      case 'o':
        event.preventDefault()
        void runCommand('new')
        break
      case 'c':
        // Capture: a thought goes on this space's inbox without leaving here.
        event.preventDefault()
        cmdline = ':card '
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
        else workspace.unmarkAll()
        break
      case 'x':
        event.preventDefault()
        void runCommand('d')
        break
      case 's':
        event.preventDefault()
        void runCommand(workspace.target?.shared ? 'unshare' : 'share')
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
        cmdGlobal = false
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
            {#if workspace.open?.kind === 'file'}
              <FilePane />
            {:else if workspace.open}
              <DocumentPane />
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
        </div>
      {/if}
      {/key}
    </main>

    <StatusBar />

    {#if cmdline !== null}
      <CommandLine initial={cmdline} global={cmdGlobal} close={() => ((cmdline = null), (cmdGlobal = false))} />
    {/if}

    <Prompt />
    <SecretPrompt />

    {#if workspace.overlay !== 'none'}
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions, a11y_no_noninteractive_tabindex -->
      <div class="overlay" tabindex="-1" bind:this={overlayEl} onclick={(e) => e.target === e.currentTarget && (workspace.overlay = 'none')}
      >
        {#if workspace.overlay === 'help'}<Help />{:else if workspace.overlay === 'overview'}<Overview />{:else if workspace.overlay === 'agenda'}<Agenda />{:else if workspace.overlay === 'actions'}<Actions />{:else if workspace.overlay === 'verify'}<Verify />{:else}<MediaBrowser />{/if}
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

  .panes.board { grid-template-columns: 1fr; }
  .window.list { --pane-bg: color-mix(in oklab, var(--surface) 60%, var(--base)); }

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

  @media (max-width: 767px) {
    .panes { grid-template-columns: 1fr; }
    .panes .entry { display: none; }
    .panes.reading .list { display: none; }
    .panes.reading .entry { display: block; }
    .overlay { padding: 12px; }
  }
</style>
