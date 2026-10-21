<script lang="ts">
  import { cycleTile, focusTile, followLink, hooks, runCommand } from './lib/commands'
  import { embedSnippet } from './lib/markdown'
  import Agenda from './lib/Agenda.svelte'
  import Calendar from './lib/Calendar.svelte'
  import Bar from './lib/Bar.svelte'
  import Board from './lib/Board.svelte'
  import CommandLine from './lib/CommandLine.svelte'
  import DocumentPane from './lib/DocumentPane.svelte'
  import FilePane from './lib/FilePane.svelte'
  import Headings from './lib/Headings.svelte'
  import Help, { focusHelpFilter, scrollHelp } from './lib/Help.svelte'
  import Hints from './lib/Hints.svelte'
  import Verify from './lib/Verify.svelte'
  import Login from './lib/Login.svelte'
  import Overview from './lib/Overview.svelte'
  import MediaBrowser, { copyMedia, insertMedia, removeMedia } from './lib/MediaBrowser.svelte'
  import Trash, { purgeTrashed, restoreTrashed } from './lib/Trash.svelte'
  import Backlinks, { openBacklink } from './lib/Backlinks.svelte'
  import CardSheet from './lib/CardSheet.svelte'
  import History, { moveHistory, restoreShown } from './lib/History.svelte'
  import Actions from './lib/Actions.svelte'
  import Prompt from './lib/Prompt.svelte'
  import SecretPrompt from './lib/SecretPrompt.svelte'
  import Rail from './lib/Rail.svelte'
  import Shared from './lib/Shared.svelte'
  import Sidebar from './lib/Sidebar.svelte'
  import StatusBar from './lib/StatusBar.svelte'
  import { workspace, type SpecialTile } from './lib/store.svelte'
  import { dwindle, formatDay, neighbour, type Direction } from './lib/util'

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
  let boardEl = $state<HTMLDivElement>()
  /** The first half of `gg`, `dd`, `]]` …, shown until the second comes. */
  let pending = $state('')
  /** `F`: the pane whose links and buttons are wearing hints right now. */
  let hintRoot = $state<HTMLElement | null>(null)

  if (!shareToken) void workspace.boot()

  /* Where each tile sits, in the order of `workspace.tiles`. */
  const layout = $derived(dwindle(workspace.tiles.length))
  /*
   * The tiles are drawn in the order they were made, whatever order they are
   * in now, so moving one never moves its element — which would cost it its
   * scroll position, and a PDF its page.
   */
  const drawn = $derived([...workspace.tiles].sort((a, b) => a.id - b.id))

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

  // A first half left alone is forgotten, as vim's `timeoutlen` does.
  $effect(() => {
    if (!pending) return
    const timer = setTimeout(() => (pending = ''), 1000)
    return () => clearTimeout(timer)
  })

  // Guard against losing a half-written entry to a stray refresh.
  $effect(() => {
    if (!workspace.tiles.some((tile) => tile.dirty)) return
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
    [['j', 'k'], 'browse, l opens, L opens beside'],
    [['1…9'], 'switch space, Tab cycles'],
    [['w'], 'overview of every space'],
    [['S'], 'board, agenda and calendar'],
    [['b', 't', 'C'], 'straight to one of them'],
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

  /**
   * The agenda's keys: j/k down the list, Enter to the card, Space finishes it.
   * Past the cards the cursor walks on into what was written on this day, and
   * past that into the calendar under it; `h` is the board to its left.
   */
  function agendaKey(event: KeyboardEvent): boolean {
    const cards = workspace.agendaCards
    const card = cards[workspace.agendaCursor]
    const past = workspace.agenda.on_this_day[workspace.agendaCursor - cards.length]
    switch (event.key) {
      case 'j':
      case 'ArrowDown': {
        const last = Math.max(cards.length + workspace.agenda.on_this_day.length - 1, 0)
        if (workspace.agendaCursor >= last) workspace.specialFocus = 'calendar'
        else workspace.agendaCursor += 1
        return true
      }
      case 'k':
      case 'ArrowUp':
        workspace.agendaCursor = Math.max(workspace.agendaCursor - 1, 0)
        return true
      case 'Enter':
        if (card) void workspace.guard(() => workspace.goToCard(card))
        // Opened from the special workspace, so `q` on it comes back here.
        else if (past) void workspace.guard(() => workspace.openPath(past.path))
        return true
      case ' ':
        if (card) void workspace.guard(() => workspace.finishCard(card))
        return true
      case 'D':
        // The agenda stays up behind the line, so `:due` knows which card.
        if (card) {
          cmdGlobal = false
          cmdline = ':due '
        }
        return true
      case ':':
        // And so does `:repeat`, and any other command that asks for it.
        cmdGlobal = false
        cmdline = ':'
        return true
      case 'H':
      case 'L':
        if (card) void workspace.guard(() => workspace.sendAgendaCard(card, event.key === 'H' ? -1 : 1))
        return true
      case 'T':
        void runCommand('today')
        return true
      case 'h':
      case 'ArrowLeft':
        workspace.specialFocus = 'board'
        return true
      case 'q':
        workspace.closeSpecial()
        return true
      default:
        return false
    }
  }

  /** The calendar's keys: h/l a day, j/k a week, H/L a month, Enter to the day. */
  function calendarKey(event: KeyboardEvent): boolean {
    const step: Record<string, [number, number]> = {
      h: [-1, 0], ArrowLeft: [-1, 0], l: [1, 0], ArrowRight: [1, 0],
      k: [-7, 0], ArrowUp: [-7, 0], j: [7, 0], ArrowDown: [7, 0],
      H: [0, -1], L: [0, 1],
    }
    if (event.key in step) {
      const [days, months] = step[event.key]
      void workspace.guard(() => workspace.moveCalendar(days, months))
      return true
    }
    switch (event.key) {
      case '.':
        workspace.calendarDay = formatDay(Math.floor(Date.now() / 1000))
        void workspace.guard(() => workspace.loadCalendar())
        return true
      case 'Enter':
        void workspace.guard(() => workspace.openCalendarDay()).then(() => hooks.focusEditor())
        return true
      case 'q':
        workspace.closeSpecial()
        return true
      default:
        return false
    }
  }

  /**
   * The help's keys: its columns run on to the right, so h/l and j/k move a
   * column, Ctrl-D/U a screen, g/G to either end, and `/` narrows it down.
   */
  function helpKey(event: KeyboardEvent): boolean {
    if (event.ctrlKey) {
      if (event.key !== 'd' && event.key !== 'u') return false
      scrollHelp(event.key === 'd' ? 'pagedown' : 'pageup')
      return true
    }
    switch (event.key) {
      case 'l':
      case 'j':
      case 'ArrowRight':
      case 'ArrowDown':
        scrollHelp(1)
        return true
      case 'h':
      case 'k':
      case 'ArrowLeft':
      case 'ArrowUp':
        scrollHelp(-1)
        return true
      case ' ':
        scrollHelp(event.shiftKey ? 'pageup' : 'pagedown')
        return true
      case 'g':
        scrollHelp('start')
        return true
      case 'G':
        scrollHelp('end')
        return true
      case '/':
        focusHelpFilter()
        return true
      default:
        return false
    }
  }

  /** `gO`'s list: j/k down the headings, Enter to the one picked. */
  function outlineKey(event: KeyboardEvent): boolean {
    const last = Math.max(workspace.outline.length - 1, 0)
    switch (event.key) {
      case 'j':
      case 'ArrowDown':
        workspace.outlineCursor = Math.min(workspace.outlineCursor + 1, last)
        return true
      case 'k':
      case 'ArrowUp':
        workspace.outlineCursor = Math.max(workspace.outlineCursor - 1, 0)
        return true
      case 'G':
        workspace.outlineCursor = last
        return true
      case 'Enter':
      case 'l': {
        const heading = workspace.outline[workspace.outlineCursor]
        workspace.overlay = 'none'
        heading?.go()
        return true
      }
      default:
        return false
    }
  }

  /** The media list's keys: j/k down the files, i inserts, y copies, x deletes. */
  function mediaKey(event: KeyboardEvent): boolean {
    const files = workspace.media
    const last = Math.max(files.length - 1, 0)
    const file = files[workspace.mediaCursor]
    switch (event.key) {
      case 'j':
      case 'ArrowDown':
        workspace.mediaCursor = Math.min(workspace.mediaCursor + 1, last)
        return true
      case 'k':
      case 'ArrowUp':
        workspace.mediaCursor = Math.max(workspace.mediaCursor - 1, 0)
        return true
      case 'G':
        workspace.mediaCursor = last
        return true
      case 'Enter':
      case 'i':
        if (file) insertMedia(file)
        return true
      case 'y':
        if (file) void copyMedia(file)
        return true
      case 'x':
        if (file) void removeMedia(file)
        return true
      default:
        return false
    }
  }

  /** The trash's keys: j/k down what was deleted, Enter or r restores, x deletes for good. */
  function trashKey(event: KeyboardEvent): boolean {
    const items = workspace.trash
    const last = Math.max(items.length - 1, 0)
    const item = items[workspace.trashCursor]
    switch (event.key) {
      case 'j':
      case 'ArrowDown':
        workspace.trashCursor = Math.min(workspace.trashCursor + 1, last)
        return true
      case 'k':
      case 'ArrowUp':
        workspace.trashCursor = Math.max(workspace.trashCursor - 1, 0)
        return true
      case 'G':
        workspace.trashCursor = last
        return true
      case 'Enter':
      case 'r':
        if (item) void restoreTrashed(item)
        return true
      case 'x':
        if (item) void purgeTrashed(item)
        return true
      default:
        return false
    }
  }

  /** The backlinks' keys: j/k through the documents that link here, Enter opens one. */
  function backlinksKey(event: KeyboardEvent): boolean {
    const last = Math.max(workspace.backlinks.length - 1, 0)
    switch (event.key) {
      case 'j':
      case 'ArrowDown':
        workspace.backlinksCursor = Math.min(workspace.backlinksCursor + 1, last)
        return true
      case 'k':
      case 'ArrowUp':
        workspace.backlinksCursor = Math.max(workspace.backlinksCursor - 1, 0)
        return true
      case 'G':
        workspace.backlinksCursor = last
        return true
      case 'Enter': {
        const link = workspace.backlinks[workspace.backlinksCursor]
        if (link) void openBacklink(link)
        return true
      }
      default:
        return false
    }
  }

  /** The history's keys: j/k through the versions, Enter brings the one shown back. */
  function historyKey(event: KeyboardEvent): boolean {
    switch (event.key) {
      case 'j':
      case 'ArrowDown':
        moveHistory(1)
        return true
      case 'k':
      case 'ArrowUp':
        moveHistory(-1)
        return true
      case 'G':
        moveHistory(workspace.history.length)
        return true
      case 'Enter':
        void restoreShown()
        return true
      default:
        return false
    }
  }

  /*
   * `F`: the hints are pinned where the targets are, so a pane still sliding
   * in — a space switch, the board just opened — is let settle first.
   */
  async function showHints(root: HTMLElement) {
    // What moves the root may be an animation on a pane around it. A blinking
    // cursor never finishes, so only what ends is waited for.
    const settling = document.getAnimations().filter((animation) => {
      const target = (animation.effect as KeyframeEffect | null)?.target
      if (!target || animation.effect?.getComputedTiming().endTime === Infinity) return false
      return target.contains(root) || root.contains(target)
    })
    await Promise.all(settling.map((animation) => animation.finished.catch(() => {})))
    if (root.isConnected) hintRoot = root
  }

  /** `gx`: the file as the browser shows it, in a tab of its own. */
  function openInTab() {
    const file = workspace.target?.file
    if (!file) return workspace.say('gx opens a file — put the cursor on one', 'error')
    // A tab opened after the vault's question would be stopped as a popup, so
    // a sealed file asks for the vault to be opened first instead.
    if (file.encrypted && !workspace.vault?.unlocked) return workspace.say('sealed — :unlock first, then gx', 'error')
    followLink(file.url, true)
  }

  /*
   * Alt + h/j/k/l: the focus goes to the tile that way, out of the editor
   * too; left of the leftmost tile is the list. With Shift the focused tile
   * goes that way instead, swapping places with the one it lands on.
   */
  /*
   * The special workspace is three tiles: the board on the left, the agenda
   * over the calendar on the right. Alt-h/j/k/l moves between them.
   */
  const specialNeighbours: Record<SpecialTile, Partial<Record<Direction, SpecialTile>>> = {
    board: { l: 'agenda' },
    agenda: { h: 'board', j: 'calendar' },
    calendar: { h: 'board', k: 'agenda' },
  }

  function tileKey(direction: Direction, shift: boolean) {
    if (workspace.view === 'special') {
      const next = specialNeighbours[workspace.specialFocus][direction]
      if (next && !shift) workspace.specialFocus = next
      return
    }
    if (workspace.pane === 'list' || !workspace.open) {
      if (direction === 'l' && !shift) focusPane('document')
      return
    }
    const next = neighbour(layout, workspace.focused, direction)
    if (next < 0) {
      if (direction === 'h' && !shift) focusPane('list')
      return
    }
    if (!shift) return void focusTile(next)
    const tiles = [...workspace.tiles]
    ;[tiles[workspace.focused], tiles[next]] = [tiles[next], tiles[workspace.focused]]
    workspace.tiles = tiles
    workspace.focused = next
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
        // Right of the last list is the agenda, the tile beside the board.
        if (workspace.listCursor >= (workspace.board?.lists.length ?? 0) - 1) workspace.specialFocus = 'agenda'
        else workspace.moveList(1)
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
        workspace.openCardSheet()
        return true
      case 'o':
      case 'c':
        cmdline = ':card '
        return true
      case 'R':
        // On an empty list there is no card to rename, so it is the list.
        // The name is there to be edited, not typed again from nothing.
        cmdline = workspace.card ? `:name ${workspace.card.title}` : `:name! ${workspace.list?.name ?? ''}`
        return true
      case 'D':
        cmdline = ':due '
        return true
      case 'z':
        workspace.showOldDone = !workspace.showOldDone
        return true
      case 'F':
        // A letter over every card in view: typing it puts the cursor there.
        if (boardEl) void showHints(boardEl)
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
        workspace.closeSpecial()
        return true
      default:
        return false
    }
  }

  /*
   * The card sheet reads the list keys over its checklist, and the board's
   * card keys for the card itself — `D`, `H`/`L`, `R` act on it as they do
   * on the board under it. While the description is being written the keys
   * are the editor's.
   */
  function cardKey(event: KeyboardEvent): boolean {
    const key = event.key
    const previous = pending
    pending = ''
    if (previous === 'g' && key === 'g') return void workspace.moveTask(-Infinity), true
    if (previous === 'g' && key === 'f') {
      workspace.overlay = 'none'
      void workspace.guard(() => workspace.openCard())
      return true
    }
    if (previous === 'd' && key === 'd') return void workspace.guard(() => workspace.deleteTask()), true
    switch (key) {
      case 'g':
      case 'd':
        pending = key
        return true
      case 'j':
      case 'ArrowDown':
        void workspace.moveTask(1)
        return true
      case 'k':
      case 'ArrowUp':
        void workspace.moveTask(-1)
        return true
      case 'G':
        void workspace.moveTask(Infinity)
        return true
      case 'J':
        void workspace.guard(() => workspace.nudgeTask(1))
        return true
      case 'K':
        void workspace.guard(() => workspace.nudgeTask(-1))
        return true
      case ' ':
      case 'x':
        void workspace.guard(() => workspace.toggleTask())
        return true
      case 'o':
        cmdline = ':task '
        return true
      case 'r': {
        const task = workspace.card?.checklist[workspace.taskCursor]
        if (task) cmdline = `:task! ${task.text}`
        return true
      }
      case 'e':
      case 'i':
      case 'Enter':
        workspace.describe()
        return true
      case 'a':
        cmdline = ':label '
        return true
      case 'R':
        cmdline = `:name ${workspace.card?.title ?? ''}`
        return true
      case 'D':
        cmdline = ':due '
        return true
      case 'H':
        void workspace.guard(() => workspace.sendCard(-1))
        return true
      case 'L':
        void workspace.guard(() => workspace.sendCard(1))
        return true
      case ':':
        cmdGlobal = false
        cmdline = ':'
        return true
      case 'q':
      case 'Escape':
        workspace.closeCardSheet()
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
      if (workspace.describing) void workspace.guard(() => workspace.saveDescription())
      else if (workspace.open) void workspace.guard(() => workspace.save())
      return
    }

    // `event.code`, so Alt-Shift-h is the same key on every layout, and on a
    // Mac, where Alt turns h into ˙.
    const alt = event.altKey && !event.ctrlKey && !event.metaKey ? /^Key([HJKLQ])$/.exec(event.code)?.[1].toLowerCase() : undefined
    if (alt === 'q' && !event.shiftKey) {
      event.preventDefault()
      void runCommand('close')
      return
    }
    if (alt && alt !== 'q') {
      event.preventDefault()
      tileKey(alt as Direction, event.shiftKey)
      return
    }

    // The command line over an overlay — `D` in the agenda — keeps its keys.
    const listening = cmdline === null && !isTypingTarget(event.target)
    if (workspace.overlay === 'card' && !workspace.describing && listening && !event.ctrlKey && !event.metaKey && !event.altKey) {
      if (cardKey(event)) {
        event.preventDefault()
        return
      }
    }
    if (workspace.overlay === 'help' && listening && !event.metaKey && !event.altKey) {
      if (helpKey(event)) {
        event.preventDefault()
        return
      }
    }
    if (workspace.overlay === 'overview' && listening && !event.ctrlKey && !event.metaKey && !event.altKey) {
      if (overviewKey(event)) {
        event.preventDefault()
        return
      }
    }
    // The special workspace's agenda and calendar read their own keys while
    // they have focus; what they leave falls through to the rest below.
    const special = workspace.overlay === 'none' && workspace.view === 'special' && listening && !event.ctrlKey && !event.metaKey && !event.altKey
    if (special && workspace.specialFocus === 'agenda') {
      if (agendaKey(event)) {
        event.preventDefault()
        return
      }
    }
    if (workspace.overlay === 'media' && listening && !event.ctrlKey && !event.metaKey && !event.altKey) {
      if (mediaKey(event)) {
        event.preventDefault()
        return
      }
    }
    if (workspace.overlay === 'history' && listening && !event.ctrlKey && !event.metaKey && !event.altKey) {
      if (historyKey(event)) {
        event.preventDefault()
        return
      }
    }
    if (workspace.overlay === 'backlinks' && listening && !event.ctrlKey && !event.metaKey && !event.altKey) {
      if (backlinksKey(event)) {
        event.preventDefault()
        return
      }
    }
    if (workspace.overlay === 'trash' && listening && !event.ctrlKey && !event.metaKey && !event.altKey) {
      if (trashKey(event)) {
        event.preventDefault()
        return
      }
    }
    if (special && workspace.specialFocus === 'calendar') {
      if (calendarKey(event)) {
        event.preventDefault()
        return
      }
    }
    if (workspace.overlay === 'outline' && listening && !event.ctrlKey && !event.metaKey && !event.altKey) {
      if (outlineKey(event)) {
        event.preventDefault()
        return
      }
    }

    if (event.key === 'Escape') {
      // The description's editor has had its Esc; the sheet stays up under it.
      if (workspace.describing) return
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

    if (workspace.overlay !== 'none' && listening) {
      if (event.key === 'q' || event.key === '?') {
        event.preventDefault()
        workspace.overlay = 'none'
      }
      return
    }

    if (cmdline !== null || isTypingTarget(event.target)) return

    // While writing, the status line says the editor is listening. A key that
    // lands anywhere else — the focus left behind by a prompt, a click on the
    // header — hands the focus back to it instead of walking the list unseen.
    if (workspace.editing && workspace.pane === 'document' && !event.ctrlKey && !event.metaKey && !event.altKey) {
      event.preventDefault()
      hooks.focusEditor()
      return
    }

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

    // In the special workspace Tab walks its tiles, as it walks the spaces
    // everywhere else; the digits still switch space, and the board with it.
    if (key === 'Tab' && workspace.view === 'special') {
      event.preventDefault()
      workspace.cycleSpecial(event.shiftKey ? -1 : 1)
      return
    }
    if (spaceKey(event)) {
      event.preventDefault()
      return
    }
    // `gt` / `gT` walk the tiles, as they walk vim's tab pages — before `t`
    // and `T` on their own are taken for the agenda and today's entry.
    if (previous === 'g' && (key === 't' || key === 'T') && workspace.view !== 'special') {
      event.preventDefault()
      void cycleTile(key === 't' ? 1 : -1)
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
    if (key === 'C') {
      event.preventDefault()
      void runCommand('cal')
      return
    }
    // `S`, as the desktop's special workspace is bound: the board, the
    // agenda and the calendar, put up and away again. `b`, `t` and `C` only
    // ever go to their tile.
    if (key === 'S') {
      event.preventDefault()
      void runCommand('special')
      return
    }

    if (workspace.view === 'special') {
      if (workspace.specialFocus !== 'board') {
        // The agenda and the calendar have had their keys; nothing reaches
        // the board's from them, since it is not the tile with focus.
        if (key === 'b') void runCommand('board')
        else if (key === 'Escape') workspace.closeSpecial()
        else if (key === ':') cmdline = ':'
        else if (key === '?') void runCommand('help')
        else return
        event.preventDefault()
        return
      }
      if (key === 'b') {
        event.preventDefault()
        return
      }
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

    // Two-key sequences: gg and dd; gO, ]] and [[ for the headings of what
    // is being read, as in vim.
    if (previous === 'g' && key === 'g') {
      event.preventDefault()
      if (reading) hooks.scrollReader('top')
      else workspace.cursor = 0
      return
    }
    if (previous === 'g' && key === 'O') {
      event.preventDefault()
      void runCommand('outline')
      return
    }
    if (previous === 'g' && key === 'h') {
      event.preventDefault()
      void runCommand('history')
      return
    }
    if (previous === 'g' && key === 'b') {
      event.preventDefault()
      void runCommand('backlinks')
      return
    }
    if (previous === 'g' && key === 'x') {
      event.preventDefault()
      openInTab()
      return
    }

    if (previous === 'd' && key === 'd') {
      event.preventDefault()
      void runCommand('d')
      return
    }
    if (previous === 'z' && ['h', 'l', 'H', 'L'].includes(key)) {
      event.preventDefault()
      const amount = ({ h: -1, l: 1, H: 'halfleft', L: 'halfright' } as const)[key as 'h' | 'l' | 'H' | 'L']
      if (!reading || !hooks.scrollWide(amount)) workspace.say('nothing wider than the column in view', 'error')
      return
    }
    // `[d` / `]d`: the diary entry of the day before or after, by its date.
    if ((previous === '[' || previous === ']') && key === 'd') {
      event.preventDefault()
      void workspace.guard(() => workspace.stepEntry(previous === '[' ? -1 : 1))
      return
    }
    if ((previous === ']' && key === ']') || (previous === '[' && key === '[')) {
      event.preventDefault()
      // Past the last heading there is still the end, as in vim.
      if (reading && !hooks.jumpHeading(key === ']' ? 1 : -1)) hooks.scrollReader(key === ']' ? 'bottom' : 'top')
      return
    }
    if (key === 'g' || key === 'd' || key === ']' || key === '[' || key === 'z') {
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
      case 'ArrowRight': {
        event.preventDefault()
        if (key === 'Enter' && reading && hooks.fileAction()) break
        // Shift-Enter, like `L`: beside what is open, in a tile of its own.
        if (key === 'Enter' && event.shiftKey && !reading) {
          void workspace.guard(() => workspace.openBeside())
          break
        }
        const shown = workspace.tiles.findIndex((tile) => tile.open && tile.open.id === workspace.selected?.id)
        if (shown >= 0) void focusTile(shown)
        else void workspace.guard(() => workspace.openSelected())
        break
      }
      case 'L':
        event.preventDefault()
        if (!reading) void workspace.guard(() => workspace.openBeside())
        break
      case 'F': {
        // `f` already fills the screen, so the hints vimium puts on `f` live
        // on its other key.
        event.preventDefault()
        const root = reading ? hooks.hintRoot() : null
        if (root) void showHints(root)
        break
      }
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
      case 'R': {
        // Renaming needs a name typed, and names are typed on the command
        // line — starting from the one it has, so a typo is a fix, not a retype.
        event.preventDefault()
        const target = workspace.target
        const current = target && target.id === workspace.open?.id ? workspace.draft.name : (target?.name ?? '')
        cmdline = `:name ${current.trim()}`
        break
      }
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
      case 'u':
        // A delete goes to the trash, so it can be undone the way vim undoes.
        event.preventDefault()
        void workspace.guard(() => workspace.undoDelete())
        break
      case 's':
        // Sharing is a key away; taking a link back from its readers asks first.
        event.preventDefault()
        void (async () => {
          const target = workspace.target
          if (target?.shared && !(await workspace.ask(`stop sharing "${target.name.trim() || target.slug}"? the link stops working`))) return
          void runCommand(target?.shared ? 'unshare' : 'share')
        })()
        break
      case 'y':
        // A vim hand presses y to yank; here it retires the link readers have,
        // so it asks first.
        event.preventDefault()
        void (async () => {
          const target = workspace.target
          if (!target?.shared) return void runCommand('link')
          if (await workspace.ask(`new share link for "${target.name.trim() || target.slug}"? the current one stops working`)) void runCommand('link')
        })()
        break
      case 'n':
      case 'N':
        // Through the results, as vim's `n` goes through the matches; the
        // search itself stays until `:noh`.
        event.preventDefault()
        void workspace.guard(() => workspace.nextResult(key === 'n' ? 1 : -1))
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
      {#if workspace.view === 'special'}
        <!-- The special workspace: the board, and the agenda over the
             calendar beside it. `f` gives the focused one the whole porthole. -->
        <div class="panes special switch" class:full={workspace.fullscreen} data-focus={workspace.specialFocus} style="--dir: {workspace.switchDirection}">
          {#each ['board', 'agenda', 'calendar'] as const as tile (tile)}
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <div
              class="window {tile}"
              class:focused={workspace.specialFocus === tile}
              onpointerdown={() => (workspace.specialFocus = tile)}
            >
              {#if tile === 'board'}
                <div class="fill" bind:this={boardEl}><Board /></div>
              {:else if tile === 'agenda'}
                <Agenda />
              {:else}
                <Calendar />
              {/if}
            </div>
          {/each}
        </div>
      {:else}
        <div
          class="panes switch"
          class:reading={workspace.tiles.some((tile) => tile.open)}
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
          <div class="tiles" class:only={workspace.tiles.length === 1}>
            {#each drawn as tile (tile.id)}
              {@const index = workspace.tiles.indexOf(tile)}
              {@const rect = layout[index]}
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div
                class="window entry"
                class:current={index === workspace.focused}
                class:focused={!!tile.open && index === workspace.focused && workspace.pane === 'document'}
                style="--x: {rect.x}; --y: {rect.y}; --w: {rect.w}; --h: {rect.h}"
                onpointerdown={() => tile.open && workspace.focusTile(index)}
                onfocusin={() => tile.open && index !== workspace.focused && workspace.focusTile(index)}
              >
                {#if tile.open?.kind === 'file'}
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
      {/if}
      {/key}
    </main>

    <StatusBar {pending} />

    {#if cmdline !== null}
      <CommandLine initial={cmdline} global={cmdGlobal} close={() => ((cmdline = null), (cmdGlobal = false))} />
    {/if}

    <Prompt />
    <SecretPrompt />

    {#if hintRoot}
      <Hints root={hintRoot} close={() => (hintRoot = null)} />
    {/if}

    {#if workspace.overlay !== 'none'}
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions, a11y_no_noninteractive_tabindex -->
      <div class="overlay" tabindex="-1" bind:this={overlayEl} onclick={(e) => e.target === e.currentTarget && (workspace.overlay === 'card' ? workspace.closeCardSheet() : (workspace.overlay = 'none'))}
      >
        {#if workspace.overlay === 'help'}<Help />{:else if workspace.overlay === 'overview'}<Overview />{:else if workspace.overlay === 'actions'}<Actions />{:else if workspace.overlay === 'verify'}<Verify />{:else if workspace.overlay === 'outline'}<Headings />{:else if workspace.overlay === 'trash'}<Trash />{:else if workspace.overlay === 'history'}<History />{:else if workspace.overlay === 'backlinks'}<Backlinks />{:else if workspace.overlay === 'card'}<CardSheet />{:else}<MediaBrowser />{/if}
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

  /*
   * The special workspace: the board takes what the right column leaves,
   * the agenda takes what the calendar leaves under it.
   */
  .panes.special {
    grid-template-columns: minmax(0, 1fr) minmax(44ch, 30vw);
    grid-template-rows: minmax(0, 1fr) auto;
    grid-template-areas: 'board agenda' 'board calendar';
  }
  .special > .board { grid-area: board; }
  .special > .agenda { grid-area: agenda; }
  .special > .calendar { grid-area: calendar; }
  .special .fill { height: 100%; }
  /* The focused tile's key is lit, as its border is. */
  .special > .window.focused :global(.tile-key) { background: var(--light); border-color: var(--light); color: var(--ink); }

  /* One cursor is lit at a time: the other tiles keep theirs, dimmed. */
  .special > .window:not(.focused) :global(:is(.card.active, .row.at, .day.at)) {
    background: var(--surface-alt);
    color: var(--light);
  }
  .special > .window:not(.focused) :global(.row.at :is(.where, .due)) { color: var(--muted); }
  .special > .window:not(.focused) :global(.card.active :is(.meta, .doc)) { color: var(--muted); }
  .special > .window:not(.focused) :global(:is(.card.active .today, .today .row.at .due)) { color: var(--light); }
  .special > .window:not(.focused) :global(:is(.card.active .overdue, .overdue .row.at .due)) { color: var(--alarm); }
  .special > .window:not(.focused) :global(.card.active .tick) { border-color: var(--outline); }
  .special > .window:not(.focused) :global(.day.at :is(.dot)) { background: var(--light); }
  .special > .window:not(.focused) :global(.day.at .many) { color: var(--light); }
  .panes.special.full { grid-template-columns: 1fr; grid-template-rows: 1fr; grid-template-areas: none; }
  .panes.special.full > .window:not(.focused) { display: none; }
  .panes.special.full > .window { grid-area: auto; }

  /* The agenda and the calendar are sheets of their own as overlays; as tiles
     the window is their sheet. */
  .special > .window :global(.sheet) {
    width: 100%;
    height: 100%;
    max-height: none;
    border: 0;
    border-radius: 0;
    background: none;
  }
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
    /* A phone shows the special workspace's focused tile alone. */
    .panes.special { grid-template-columns: 1fr; grid-template-rows: 1fr; grid-template-areas: none; }
    .panes.special > .window { grid-area: auto; }
    .panes.special > .window:not(.focused) { display: none; }
  }
</style>
