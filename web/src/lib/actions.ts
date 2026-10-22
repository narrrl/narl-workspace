/*
 * What the keys in keymap.ts do, under the names the table gives them, one
 * record per context. The type below holds the two together: a binding
 * whose action is missing here, or an action no binding names, does not
 * compile.
 *
 * An action is handed the key as `keyName` spelled it, for the ones that
 * share an action and differ in direction. Returning `false` lets the key
 * on to the next context down, as if it had not been bound here.
 */
import { cycleTile, focusTile, followLink, hooks, newDocument, openInfo, runCommand } from './commands'
import { bindings, type Context } from './keymap'
import { deeperHelp, focusHelpFilter, scrollHelp } from './Help.svelte'
import { actSetting, moveSettings, removeSetting, turnSettings, openSettings } from './Settings.svelte'
import { purgeTrashed, restoreTrashed } from './Trash.svelte'
import { copyEmbed, embedFile, goHeading, moveHistory, openBacklink, openFile, renewLink, restoreShown, toggleSharing, turnInfo } from './Info.svelte'
import { workspace } from './store.svelte'
import { dwindle, formatDay, neighbour, type Direction } from './util'

export type Run = (key: string, event: KeyboardEvent) => boolean | void

type Bound = (typeof bindings)[number]
type ActionOf<C extends Context> = Extract<Bound, { context: C; action: string }>['action']

const guard = (task: () => Promise<unknown>) => void workspace.guard(task)
const line = (text: string) => hooks.openCommandLine(text)
const shifted = (key: string) => key.startsWith('Shift-')
/** The space a digit key stands for: 1 is the first, 0 the tenth. */
const spaceIndex = (key: string) => {
  const digit = key.slice(-1)
  return digit === '0' ? 9 : Number(digit) - 1
}
/** The document on screen is what j/k scroll, rather than the list. */
const reading = () => !!workspace.open && !workspace.editing && workspace.pane === 'document'

/** A cursor down a sheet's list, kept on it. */
function step(cursor: 'outlineCursor' | 'filesCursor' | 'trashCursor' | 'backlinksCursor' | 'cardsCursor', length: number, by: number) {
  workspace.sheets[cursor] = Math.min(Math.max(workspace.sheets[cursor] + by, 0), Math.max(length - 1, 0))
}

export async function openOverview() {
  await workspace.guard(() => workspace.sheets.loadOverview())
  workspace.sheets.overviewCursor = Math.max(
    workspace.sheets.overview.findIndex((entry) => entry.space.id === workspace.tree.spaceId),
    0,
  )
  workspace.sheets.open('overview')
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
  if (root.isConnected) hooks.showHints(root)
}

/** `gx`: the file as the browser shows it, in a tab of its own. */
function openInTab() {
  const file = workspace.target?.file
  if (!file) return workspace.say('gx opens a file — put the cursor on one', 'error')
  // A tab opened after the vault's question would be stopped as a popup, so
  // a sealed file asks for the vault to be opened first instead.
  if (file.encrypted && !workspace.vault.status?.unlocked) return workspace.say('sealed — :vault unlock first, then gx', 'error')
  followLink(file.url, true)
}

/** Focus moves between the panes, out of the editor too. */
function focusPane(pane: 'list' | 'document') {
  if (pane === 'document' && workspace.tile.empty) return
  workspace.pane = pane
  if (pane === 'document' && workspace.editing) hooks.focusEditor()
  else hooks.focusList()
}

/*
 * Ctrl + h/j/k/l: the focus goes to the tile that way, out of the editor
 * too; left of the leftmost tile is the list. With Shift the focused tile
 * goes that way instead, swapping places with the one it lands on.
 */
function tileKey(direction: Direction, shift: boolean) {
  if (workspace.focus === 'list') {
    if (direction === 'l' && !shift) focusPane('document')
    return
  }
  const next = neighbour(dwindle(workspace.tiles.all.length), workspace.tiles.focused, direction)
  if (next < 0) {
    // The plan screen has no list beside it.
    if (direction === 'h' && !shift && !workspace.tiles.planning) focusPane('list')
    return
  }
  if (!shift) return void focusTile(next)
  const tiles = [...workspace.tiles.all]
  ;[tiles[workspace.tiles.focused], tiles[next]] = [tiles[next], tiles[workspace.tiles.focused]]
  workspace.tiles.all = tiles
  workspace.tiles.focused = next
}

/** The drawer up, and the keys back where they were: in the editor, when a document was being written. */
export async function raise() {
  await workspace.scratch.close()
  if (!workspace.scratch.open && workspace.editing && workspace.pane === 'document') hooks.focusEditor()
}

export const actions: { [C in Context]: Record<ActionOf<C>, Run> } = {
  always: {
    save: () => {
      if (workspace.drafts.writing) guard(() => workspace.drafts.saveWriting())
      else if (workspace.open) guard(() => workspace.drafts.save())
    },
    // Under the drawer the tiles are out of sight, so the keys are the note's.
    focus: (key) => (workspace.scratch.open ? false : tileKey(key.slice(-1) as Direction, false)),
    swap: (key) => (workspace.scratch.open ? false : tileKey(key.slice(-1).toLowerCase() as Direction, true)),
    // Ctrl-Q: down with the last note being written, and up again back to
    // what was being done — the document still in the mode it was left in.
    scratch: () => guard(() => (workspace.scratch.open ? raise() : workspace.scratch.jot())),
  },

  sheet: {
    // A note being written has had its Esc; the sheet stays up under it.
    close: () => {
      if (workspace.scratch.editing) return
      workspace.sheets.close()
    },
  },

  /*
   * The help's columns run on to the right, so h/l and j/k move a column,
   * Ctrl-D/U a screen, g/G to either end, and `/` narrows it down.
   */
  help: {
    next: () => scrollHelp(1),
    previous: () => scrollHelp(-1),
    pageDown: () => scrollHelp('pagedown'),
    pageUp: () => scrollHelp('pageup'),
    page: (key) => scrollHelp(shifted(key) ? 'pageup' : 'pagedown'),
    start: () => scrollHelp('start'),
    end: () => scrollHelp('end'),
    filter: () => focusHelpFilter(),
    deeper: () => deeperHelp(),
  },

  /** The overview: hjkl across the grid, a digit or Enter to go. */
  overview: {
    step: (key) => {
      const columns = hooks.overviewColumns()
      const by = { h: -1, ArrowLeft: -1, l: 1, ArrowRight: 1, k: -columns, ArrowUp: -columns, j: columns, ArrowDown: columns }[key] ?? 0
      workspace.sheets.overviewCursor = Math.min(Math.max(workspace.sheets.overviewCursor + by, 0), workspace.sheets.overview.length - 1)
    },
    go: (key) => goToSpace(spaceIndex(key)),
    enter: () => goToSpace(workspace.sheets.overviewCursor),
    close: () => workspace.sheets.close(),
  },

  /** The info sheet: Tab from one of its parts to the next. */
  info: {
    next: () => turnInfo(1),
    previous: () => turnInfo(-1),
  },

  /** Its headings: j/k down them, Enter to the one picked. */
  outline: {
    down: () => step('outlineCursor', workspace.sheets.outline.length, 1),
    up: () => step('outlineCursor', workspace.sheets.outline.length, -1),
    bottom: () => step('outlineCursor', workspace.sheets.outline.length, Infinity),
    go: () => goHeading(),
  },

  /** Its files: j/k down them, Enter opens one, i embeds it while writing, y copies how. */
  files: {
    down: () => step('filesCursor', workspace.open?.files.length ?? 0, 1),
    up: () => step('filesCursor', workspace.open?.files.length ?? 0, -1),
    bottom: () => step('filesCursor', workspace.open?.files.length ?? 0, Infinity),
    open: () => {
      const file = workspace.open?.files[workspace.sheets.filesCursor]
      if (file) void openFile(file)
    },
    insert: () => {
      const file = workspace.open?.files[workspace.sheets.filesCursor]
      if (file) embedFile(file)
    },
    copy: () => {
      const file = workspace.open?.files[workspace.sheets.filesCursor]
      if (file) void copyEmbed(file)
    },
  },

  /** The trash: j/k down what was deleted, Enter or r restores, x deletes for good. */
  trash: {
    down: () => step('trashCursor', workspace.sheets.trash.length, 1),
    up: () => step('trashCursor', workspace.sheets.trash.length, -1),
    bottom: () => step('trashCursor', workspace.sheets.trash.length, Infinity),
    restore: () => {
      const item = workspace.sheets.trash[workspace.sheets.trashCursor]
      if (item) void restoreTrashed(item)
    },
    purge: () => {
      const item = workspace.sheets.trash[workspace.sheets.trashCursor]
      if (item) void purgeTrashed(item)
    },
  },

  /** Its backlinks: j/k through the documents that link here, Enter opens one. */
  backlinks: {
    down: () => step('backlinksCursor', workspace.sheets.backlinks.length, 1),
    up: () => step('backlinksCursor', workspace.sheets.backlinks.length, -1),
    bottom: () => step('backlinksCursor', workspace.sheets.backlinks.length, Infinity),
    open: () => {
      const link = workspace.sheets.backlinks[workspace.sheets.backlinksCursor]
      if (link) void openBacklink(link)
    },
  },

  /**
   * Its cards: j/k picks the one the card verbs act on — Space, H/L, `:due`,
   * `:label` — and Enter turns it over on its board.
   */
  cards: {
    down: () => step('cardsCursor', workspace.open?.cards.length ?? 0, 1),
    up: () => step('cardsCursor', workspace.open?.cards.length ?? 0, -1),
    bottom: () => step('cardsCursor', workspace.open?.cards.length ?? 0, Infinity),
    turn: () => {
      const card = workspace.open?.cards[workspace.sheets.cardsCursor]
      if (!card) return
      workspace.sheets.close()
      guard(() => workspace.cards.turnTo(card))
    },
    done: () => guard(() => workspace.cards.toggleDone()),
    sendLeft: () => guard(() => workspace.cards.sendCard(-1)),
    sendRight: () => guard(() => workspace.cards.sendCard(1)),
    due: () => {
      if (workspace.cards.focusedCard) line(':due ')
    },
    new: () => line(':card '),
    label: () => {
      if (workspace.cards.focusedCard) line(':label ')
    },
    line: () => line(':'),
  },

  /** Its history: j/k through the versions, Enter brings the one shown back. */
  history: {
    down: () => moveHistory(1),
    up: () => moveHistory(-1),
    bottom: () => moveHistory(workspace.sheets.history.length),
    restore: () => void restoreShown(),
  },

  /** Its sharing: s shares or stops, y makes a new link. */
  share: {
    toggle: () => void toggleSharing(workspace.open),
    link: () => void renewLink(workspace.open),
  },

  /** The settings sheet: Tab from part to part, j/k down it, Enter changes, x removes. */
  settings: {
    next: () => turnSettings(1),
    previous: () => turnSettings(-1),
    down: () => moveSettings(1),
    up: () => moveSettings(-1),
    bottom: () => moveSettings(Infinity),
    act: () => actSetting(),
    remove: () => removeSetting(),
  },

  /*
   * A card's back reads the list keys over its checklist, and the board's
   * card keys for the card itself — `D`, `H`/`L`, `R` act on it as they do
   * on the board beside it. While the description is being written the keys
   * are the editor's.
   */
  card: {
    down: () => void workspace.cards.moveTask(1),
    up: () => void workspace.cards.moveTask(-1),
    top: () => void workspace.cards.moveTask(-Infinity),
    bottom: () => void workspace.cards.moveTask(Infinity),
    tick: () => guard(() => workspace.cards.toggleTask()),
    // `add` spelled out, so a step that starts with "edit" is added too.
    add: () => line(':task add '),
    reword: () => {
      const task = workspace.cards.card?.checklist[workspace.cards.taskCursor]
      if (task) line(`:task edit ${task.text}`)
    },
    nudgeDown: () => guard(() => workspace.cards.nudgeTask(1)),
    nudgeUp: () => guard(() => workspace.cards.nudgeTask(-1)),
    drop: () => guard(() => workspace.cards.deleteTask()),
    describe: () => workspace.cards.describe(),
    label: () => line(':label '),
    rename: () => line(`:name ${workspace.cards.card?.title ?? ''}`),
    due: () => line(':due '),
    sendLeft: () => guard(() => workspace.cards.sendCard(-1)),
    sendRight: () => guard(() => workspace.cards.sendCard(1)),
    notes: () => guard(() => workspace.cards.openCard()),
    line: () => line(':'),
  },

  /*
   * The scratchpad, while it is down and its note is not being written: the
   * list keys over its notes. It sits over whatever was on screen, so
   * nothing else hears a key until it is raised again.
   */
  drawer: {
    down: () => void (workspace.scratch.cursor = Math.min(workspace.scratch.cursor + 1, Math.max(workspace.scratch.notes.length - 1, 0))),
    up: () => void (workspace.scratch.cursor = Math.max(workspace.scratch.cursor - 1, 0)),
    top: () => void (workspace.scratch.cursor = 0),
    bottom: () => void (workspace.scratch.cursor = Math.max(workspace.scratch.notes.length - 1, 0)),
    new: () => workspace.scratch.newNote(),
    write: () => workspace.scratch.editNote(),
    keep: () => guard(() => workspace.scratch.keepNote()),
    card: () => guard(() => workspace.scratch.cardNote()),
    append: () => {
      const open = [workspace.tile, ...workspace.tiles.all].find((tile) => tile.open?.kind === 'document')?.open
      line(`:append ${open ? open.name.trim() || open.slug : ''}`)
    },
    drop: () => {
      if (!workspace.scratch.note) return
      void (async () => {
        if (await workspace.prompts.ask('drop this scratch note?')) await workspace.guard(() => workspace.scratch.dropNote())
      })()
    },
    line: () => line(':'),
    help: () => void runCommand('help'),
    raise: () => guard(raise),
    nothing: () => {},
  },

  workspace: {
    /** Esc leaves one thing: the line, or fullscreen. */
    escape: () => {
      if (workspace.drafts.writing) return
      if (hooks.closeCommandLine()) return
      if (workspace.fullscreen && !workspace.editing) return void (workspace.fullscreen = false)
      return false
    },
    // `Q`, a quick note: the scratchpad lowered from anywhere a key is not
    // being typed — a letter, since a backtick is a dead key on many layouts.
    scratch: () => {
      if (workspace.scratch.open) return false
      guard(() => workspace.scratch.toggle())
    },
  },

  writing: {
    refocus: () => hooks.focusEditor(),
  },

  /*
   * The focused pane owns j/k. In the document they scroll what is on
   * screen; `h` moves focus to the list with the document still open, where
   * they walk the rows again, and `l` goes back.
   */
  tree: {
    down: () => void (reading() ? hooks.scrollReader(1) : workspace.tree.move(1)),
    up: () => void (reading() ? hooks.scrollReader(-1) : workspace.tree.move(-1)),
    top: () => void (reading() ? hooks.scrollReader('top') : (workspace.tree.cursor = 0)),
    bottom: () => void (reading() ? hooks.scrollReader('bottom') : (workspace.tree.cursor = Math.max(workspace.tree.rows - 1, 0))),
    open: (key) => {
      const enter = key === 'Enter' || key === 'Shift-Enter'
      if (enter && reading() && hooks.fileAction()) return
      // Reading, `l` is the tile to the right, as `h` is the one to the left.
      if (!enter && workspace.pane === 'document' && workspace.open) return tileKey('l', false)
      // Shift-Enter, like `L`: beside what is open, in a tile of its own.
      if (key === 'Shift-Enter' && !reading()) return guard(() => workspace.tiles.openBeside())
      const shown = workspace.tiles.all.findIndex((tile) => tile.open && tile.open.id === workspace.tree.selected?.id)
      if (shown >= 0) void focusTile(shown)
      else guard(() => workspace.tree.openSelected())
    },
    beside: () => {
      if (!reading()) guard(() => workspace.tiles.openBeside())
    },
    // Leftwards at every depth: from a document to the tile left of it, from
    // the leftmost to the list, from the list up a level.
    left: () => {
      if (workspace.open && workspace.pane === 'document') tileKey('h', false)
      else guard(() => workspace.tree.up())
    },
    sideways: (key) => {
      const amount = ({ zh: -1, zl: 1, zH: 'halfleft', zL: 'halfright' } as const)[key as 'zh' | 'zl' | 'zH' | 'zL']
      if (!reading() || !hooks.scrollWide(amount)) workspace.say('nothing wider than the column in view', 'error')
    },
    // Past the last heading there is still the end, as in vim.
    nextHeading: () => {
      if (reading() && !hooks.jumpHeading(1)) hooks.scrollReader('bottom')
    },
    previousHeading: () => {
      if (reading() && !hooks.jumpHeading(-1)) hooks.scrollReader('top')
    },
    info: () => void openInfo(),
    outline: () => void runCommand('outline'),
    history: () => void runCommand('history'),
    backlinks: () => void runCommand('backlinks'),
    cards: () => void runCommand('cards'),
    inTab: () => openInTab(),
    // `f` already fills the screen, so the hints vimium puts on `f` live on
    // its other key.
    hints: () => {
      const root = reading() ? hooks.hintRoot() : null
      if (root) void showHints(root)
    },
    // `[d` / `]d`: the diary entry of the day before or after, by its date.
    previousEntry: () => guard(() => workspace.diary.stepEntry(-1)),
    nextEntry: () => guard(() => workspace.diary.stepEntry(1)),
    // Pick nodes up here, walk to a folder, put them down with `p`.
    mark: () => workspace.tree.mark(),
    markAll: () => workspace.tree.markAll(),
    files: () => guard(() => workspace.tree.toggleFiles()),
    drop: () => guard(() => workspace.tree.drop()),
    new: () => guard(newDocument),
    // Capture: a thought goes on this space's inbox without leaving here.
    // Reading a document, `:card` makes it about that document.
    capture: () => line(':card '),
    // The name has to come from somewhere, and the command line is where
    // names are typed — this only saves typing `:mkdir `.
    mkdir: () => line(':mkdir '),
    edit: () => void runCommand('edit'),
    // Renaming starts from the name it has, so a typo is a fix, not a retype.
    rename: () => {
      const target = workspace.target
      const current = target && target.id === workspace.open?.id ? workspace.draft.name : (target?.name ?? '')
      line(`:name ${current.trim()}`)
    },
    close: () => {
      if (workspace.open) void runCommand('q')
      else workspace.tree.unmarkAll()
    },
    // Esc lets go of one thing at a time: what is carried, then what has the
    // keys — the document read, or the search the list shows — then the rest.
    escape: () => {
      if (workspace.tree.marked.length) return workspace.tree.unmarkAll()
      if (workspace.open && workspace.pane === 'document') return void runCommand('q')
      if (workspace.tree.query) return guard(() => workspace.tree.clearSearch())
      if (workspace.open) void runCommand('q')
    },
    delete: () => void runCommand('d'),
    // A delete goes to the trash, so it can be undone the way vim undoes.
    undo: () => guard(() => workspace.tree.undoDelete()),
    // Through the results, as vim's `n` goes through the matches; the
    // search itself stays until Esc or `:noh`.
    next: () => guard(() => workspace.tree.nextResult(1)),
    previous: () => guard(() => workspace.tree.nextResult(-1)),
    search: () => line('/'),
  },

  /*
   * The board reads the same keys as the list, one dimension wider: h/l walk
   * the lists, j/k the cards, and the shifted pair takes the card along.
   * Nothing falls through to the tree keys, because they would act on a tree
   * that is not on screen.
   */
  board: {
    left: () => workspace.cards.moveList(-1),
    // Right of the last list is the tile beside the board.
    right: () => {
      if (workspace.cards.listCursor >= (workspace.cards.board?.lists.length ?? 0) - 1) tileKey('l', false)
      else workspace.cards.moveList(1)
    },
    down: () => workspace.cards.moveCard(1),
    up: () => workspace.cards.moveCard(-1),
    top: () => void (workspace.cards.cardCursor = 0),
    bottom: () => void (workspace.cards.cardCursor = Math.max((workspace.cards.list?.cards.length ?? 0) - 1, 0)),
    nudgeDown: () => guard(() => workspace.cards.nudgeCard(1)),
    nudgeUp: () => guard(() => workspace.cards.nudgeCard(-1)),
    sendLeft: () => guard(() => workspace.cards.sendCard(-1)),
    sendRight: () => guard(() => workspace.cards.sendCard(1)),
    done: () => guard(() => workspace.cards.toggleDone()),
    turn: () => guard(() => workspace.cards.turnCard()),
    // The card's notes, without turning it over first: `gf` as on its back.
    notes: () => guard(() => workspace.cards.openCard()),
    new: () => line(':card '),
    due: () => line(':due '),
    // On an empty list there is no card to rename, so it is the list. The
    // name is there to be edited, not typed again from nothing.
    rename: () => line(workspace.cards.card ? `:name ${workspace.cards.card.title}` : `:list name ${workspace.cards.list?.name ?? ''}`),
    old: () => void (workspace.cards.showOldDone = !workspace.cards.showOldDone),
    // A letter over every card in view: typing it puts the cursor there.
    hints: () => {
      const root = hooks.boardRoot()
      if (root) void showHints(root)
    },
    reload: () => guard(() => workspace.cards.reloadBoard()),
    delete: () => void runCommand('delete'),
  },

  /*
   * The agenda: j/k down the list, Enter to the card, Space finishes it.
   * Past the cards the cursor walks on into what was written on this day,
   * and past that into the tile under it; `h` is the tile to its left.
   */
  agenda: {
    down: () => {
      const last = Math.max(workspace.cards.agendaCards.length + workspace.cards.agenda.on_this_day.length - 1, 0)
      if (workspace.cards.agendaCursor >= last) tileKey('j', false)
      else workspace.cards.agendaCursor += 1
    },
    up: () => void (workspace.cards.agendaCursor = Math.max(workspace.cards.agendaCursor - 1, 0)),
    left: () => tileKey('h', false),
    open: () => {
      const card = workspace.cards.agendaCards[workspace.cards.agendaCursor]
      const past = workspace.cards.agenda.on_this_day[workspace.cards.agendaCursor - workspace.cards.agendaCards.length]
      if (card) guard(() => workspace.cards.goToCard(card))
      // Beside the agenda, so `q` on it comes back here.
      else if (past) guard(() => workspace.tree.openPath(past.path))
    },
    finish: () => guard(() => workspace.cards.toggleDone()),
    due: () => {
      if (workspace.cards.agendaCards[workspace.cards.agendaCursor]) line(':due ')
    },
    line: () => line(':'),
    sendLeft: () => guard(() => workspace.cards.sendCard(-1)),
    sendRight: () => guard(() => workspace.cards.sendCard(1)),
    today: () => void runCommand('today'),
  },

  /** The calendar: h/l a day, j/k a week, H/L a month, Enter to the day. */
  calendar: {
    day: (key) => guard(() => workspace.diary.moveCalendar(key === 'h' || key === 'ArrowLeft' ? -1 : 1, 0)),
    week: (key) => guard(() => workspace.diary.moveCalendar(key === 'k' || key === 'ArrowUp' ? -7 : 7, 0)),
    month: (key) => guard(() => workspace.diary.moveCalendar(0, key === 'H' ? -1 : 1)),
    today: () => {
      workspace.diary.calendarDay = formatDay(Math.floor(Date.now() / 1000))
      guard(() => workspace.diary.loadCalendar())
    },
    open: () => void workspace.guard(() => workspace.diary.openCalendarDay()).then(() => hooks.focusEditor()),
  },

  /** What every view shares: `q` closes a card's back, and from the others goes back to the documents. */
  views: {
    close: () => void runCommand('close'),
  },

  global: {
    // Ctrl-P: go to anything by its name, in any space.
    find: () => hooks.openJump(),
    // Ctrl-D / Ctrl-U page the open document, as they do in a pager.
    halfDown: () => (workspace.open && !workspace.editing ? void hooks.scrollReader('halfdown') : false),
    halfUp: () => (workspace.open && !workspace.editing ? void hooks.scrollReader('halfup') : false),
    /*
     * Spaces are workspaces: digits jump to one, shifted digits send the node
     * there, Tab walks along the bar. Digits are read from `event.code`, so
     * Shift+1 means space 1 whatever symbol the layout puts on that key.
     */
    space: (key) => guard(() => workspace.tree.switchSpace(spaceIndex(key))),
    send: (key) => guard(() => workspace.tree.sendToSpace(spaceIndex(key))),
    cycle: (key) => guard(() => workspace.tree.cycleSpace(shifted(key) ? -1 : 1)),
    overview: () => void openOverview(),
    fullscreen: () => void (workspace.fullscreen = !workspace.fullscreen),
    // `gt` / `gT` walk the tiles, as they walk vim's tab pages.
    nextTile: () => void cycleTile(1),
    previousTile: () => void cycleTile(-1),
    // The board is a view of the space, not a row in the list, so it needs
    // a key as plain as the one that opens a document.
    board: () => void runCommand('board'),
    // The agenda and today's entry are a key away from anywhere, the board
    // included: they are the two questions asked most.
    agenda: () => void runCommand('agenda'),
    today: () => void runCommand('today'),
    calendar: () => void runCommand('cal'),
    // `S`, as the desktop's special workspace is bound: the plan screen —
    // the board, the agenda and the calendar — and back to the documents.
    // `b`, `t` and `C` go there too, to their own view.
    plan: () => void runCommand('plan'),
    line: () => line(':'),
    help: () => void runCommand('help'),
    settings: () => void openSettings(),
  },

  editor: {},
}

function goToSpace(index: number) {
  const entry = workspace.sheets.overview[index]
  if (!entry) return
  workspace.sheets.close()
  guard(() => workspace.tree.enterSpace(entry.space.id))
}
