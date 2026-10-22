/*
 * Every key the app reads, in one table: what is pressed, where, what it
 * does, and the words the help, the footers, the rail and the wiki show for
 * it. The keyboard is dispatched from here (modes.ts), so a key the help
 * describes is a key that exists.
 *
 * This file is data only — no imports — so scripts/wiki.ts can read it as it
 * is. What each action does lives in actions.ts, under the same names.
 */

/**
 * Where a key is heard. modes.ts stacks them, the topmost first:
 * always > a sheet > the drawer > workspace > writing > what has the focus
 * (the tree, or a view: board, agenda, calendar, card) > views > global.
 */
export type Context =
  | 'always'
  | 'sheet'
  | 'help'
  | 'overview'
  | 'info'
  | 'outline'
  | 'files'
  | 'trash'
  | 'backlinks'
  | 'cards'
  | 'history'
  | 'share'
  | 'settings'
  | 'card'
  | 'drawer'
  | 'workspace'
  | 'writing'
  | 'tree'
  | 'board'
  | 'agenda'
  | 'calendar'
  | 'views'
  | 'global'
  | 'editor'

/** The help's columns of keys, and the wiki's. */
export type Section = 'keys' | 'board' | 'card' | 'scratch' | 'agenda' | 'editor'

export interface Binding {
  context: Context
  /**
   * What is pressed, as `keyName` spells it: `j`, `G`, `Enter`, `Space`,
   * `Shift-Tab`, `Ctrl-d`, `Ctrl-H`, `1`, `Shift-1`, a two-key `gg`, or `*`
   * for any key without Ctrl, Alt or Meta. A binding without keys only
   * describes — a mouse gesture, the editor's own keymap.
   */
  keys?: readonly string[]
  /** The name of what it does, in actions.ts under the same context. */
  action?: string
  /** Heard while a field or the editor has the keyboard. */
  typing?: boolean
  /** The keys as the help writes them; `keys` joined when not given. */
  label?: string
  /** A line in the help; a binding without one is not listed there. */
  help?: string
  /** The help column it is listed in, when not its context's own. */
  section?: Section
  /** A word or two for the footer of its context. */
  hint?: string
  /** The keycaps the footer and the rail show; from `keys` when not given. */
  caps?: readonly string[]
  /** On the help's first page, among the keys to start with. */
  essential?: boolean
}

const digits = ['1', '2', '3', '4', '5', '6', '7', '8', '9', '0'] as const

export const bindings = [
  // --- browsing and reading; `always` is heard in the editor too ---------------
  { context: 'global', keys: digits, action: 'space', caps: ['1…9'], label: '1 … 9 · 0', help: 'switch to that space, like a workspace', essential: true },
  { context: 'global', keys: digits.map((digit) => `Shift-${digit}`), action: 'send', label: 'Shift 1 … 9', help: 'send the marked nodes, or the selected one, to that space' },
  { context: 'global', keys: ['Tab', 'Shift-Tab'], action: 'cycle', label: 'Tab · Shift-Tab', help: 'next / previous space' },
  { context: 'global', keys: ['w'], action: 'overview', help: 'overview of every space — hjkl, then Enter' },
  { context: 'global', keys: ['f'], action: 'fullscreen', help: 'fullscreen the focused pane or tile; again or Esc to leave' },
  { context: 'always', keys: ['Ctrl-h', 'Ctrl-j', 'Ctrl-k', 'Ctrl-l'], action: 'focus', typing: true, label: 'Ctrl-h/j/k/l', help: 'focus the tile that way, from the editor too — left of the leftmost is the list; h / l do the same while reading', essential: true },
  { context: 'always', keys: ['Ctrl-H', 'Ctrl-J', 'Ctrl-K', 'Ctrl-L'], action: 'swap', typing: true, label: 'Ctrl-Shift-h/j/k/l', help: 'move the focused tile that way' },
  { context: 'tree', keys: ['L'], action: 'beside', label: 'L · Shift-Enter', help: 'open the row in a new tile beside the others' },
  { context: 'global', keys: ['gt'], action: 'nextTile', label: 'gt / gT', help: 'next / previous tile — a view too' },
  { context: 'global', keys: ['gT'], action: 'previousTile' },
  { context: 'tree', keys: ['j', 'ArrowDown'], action: 'down', label: 'j / k · ↓ ↑', help: 'move down / up the list — scroll the document when it has focus', essential: true },
  { context: 'tree', keys: ['k', 'ArrowUp'], action: 'up' },
  { context: 'tree', keys: ['gg'], action: 'top', label: 'gg / G', help: 'first / last row, or top / bottom of the document — a first g, d, ] or [ waits a second in the status line' },
  { context: 'tree', keys: ['G'], action: 'bottom' },
  { context: 'global', keys: ['Ctrl-d'], action: 'halfDown', label: 'Ctrl-D / Ctrl-U', help: 'half a page down / up the open document' },
  { context: 'global', keys: ['Ctrl-u'], action: 'halfUp' },
  { context: 'tree', keys: ['zh', 'zl', 'zH', 'zL'], action: 'sideways', label: 'zl / zh · zL / zH', help: 'the wide table or code block at the top of the view, sideways — a little, or half its width' },
  { context: 'tree', keys: [']]'], action: 'nextHeading', label: ']] / [[', help: 'next / previous heading of the document — in the editor too' },
  { context: 'tree', keys: ['[['], action: 'previousHeading' },
  { context: 'tree', keys: ['gi'], action: 'info', help: "the info sheet: the open document's headings, earlier versions, backlinks, cards, files and sharing — :share there, Tab between them", essential: true },
  { context: 'tree', keys: ['gO'], action: 'outline', help: 'the headings as a list: j/k, Enter jumps there' },
  { context: 'tree', keys: ['F'], action: 'hints', help: 'hints over the links and buttons in view — type one to follow it' },
  { context: 'tree', label: 'gf / gx', help: 'in the editor: follow the link under the cursor / open it in a tab' },
  { context: 'tree', keys: ['gx'], action: 'inTab', help: 'open the file under the cursor in a tab of its own' },
  { context: 'tree', keys: ['Enter', 'Shift-Enter', 'l', 'ArrowRight'], action: 'open', label: 'Enter · l', help: 'open a document, or enter a folder — on an open keystore or sealed file, Enter asks for its password', essential: true },
  { context: 'tree', keys: ['h', 'ArrowLeft'], action: 'left', label: 'h', help: 'from the document to the list, from the list up a level', essential: true },
  { context: 'tree', keys: ['o'], action: 'new', help: 'new document here, straight into insert mode — when the space has templates, the line offers them first', essential: true },
  { context: 'tree', keys: ['O'], action: 'mkdir', caps: ['⇧ O'], help: 'new folder here' },
  { context: 'tree', keys: ['i', 'a'], action: 'edit', help: 'edit the open document', essential: true },
  { context: 'tree', keys: ['R'], action: 'rename', help: 'rename what is under the cursor — document, folder or card — starting from its name' },
  { context: 'tree', keys: ['m'], action: 'mark', label: 'm / p', help: 'pick nodes up, then drop them all into the folder you walk to', essential: true },
  { context: 'tree', keys: ['p'], action: 'drop' },
  { context: 'tree', keys: ['za'], action: 'files', help: "a document's own files, folded open under it in the list — again folds them away; p drops marked files in there" },
  { context: 'tree', keys: ['M'], action: 'markAll', help: 'pick up the whole folder — again puts it all back' },
  { context: 'tree', label: 'Ctrl / Shift-click', help: 'pick a row up · every row up to it — drag them onto a folder' },
  { context: 'global', keys: ['S'], action: 'plan', help: 'the plan screen: the board, the agenda and the calendar, in place of the documents and the list — S again goes back to the documents as they were', essential: true },
  { context: 'global', keys: ['t'], action: 'agenda', help: 'agenda — late, due this week, in hand, on every board — on the plan screen', essential: true },
  { context: 'global', keys: ['T'], action: 'today', help: "today's diary entry, started if there is none — from templates/day when the diary has one", essential: true },
  { context: 'global', keys: ['C'], action: 'calendar', help: 'the diary by month, on the plan screen: h/l a day, j/k a week, H/L a month, . today, Enter opens or starts the day among the documents' },
  { context: 'tree', keys: ['[d'], action: 'previousEntry', label: '[d / ]d', help: 'the diary entry before / after the one open, by date' },
  { context: 'tree', keys: [']d'], action: 'nextEntry' },
  { context: 'global', keys: ['Ctrl-p'], action: 'find', label: 'Ctrl-P', help: 'go to anything by its name, in every space — words before the last narrow the path, as in proj/ingest; Tab or Ctrl-N/P walk, Enter goes', essential: true },
  { context: 'global', keys: ['b'], action: 'board', help: "the board, on the plan screen: this space's, or the last one shown — 1 … 9 there shows another's" },
  { context: 'tree', keys: ['c'], action: 'capture', help: "capture a card into this space's inbox, without leaving here — reading a document, a card about it; Tab on the line makes it a note, or a line in today's entry", essential: true },
  { context: 'tree', keys: ['q'], action: 'close', label: 'Esc · q', help: 'close the document, and its tile when there are several — Esc first puts down what m picked up, and in the list lets go of a search', essential: true },
  { context: 'tree', keys: ['Escape'], action: 'escape' },
  { context: 'tree', keys: ['/'], action: 'search', help: 'full-text search, inside the current space and its text files — Tab takes it to every space, Ctrl-N/P walk the results, Enter opens one, Esc gives up', essential: true },
  { context: 'tree', keys: ['n'], action: 'next', label: 'n / N', help: 'next / previous search result, opened beside the results — Esc or :noh clears the search' },
  { context: 'tree', keys: ['N'], action: 'previous' },
  { context: 'tree', keys: ['x', 'dd'], action: 'delete', help: 'delete what has focus — the row in the list, the document in its pane — into the trash (asks y/N)', essential: true },
  { context: 'tree', keys: ['u'], action: 'undo', help: 'undo the last delete: it comes back out of the trash — :trash has the rest', essential: true },
  { context: 'tree', keys: ['gh'], action: 'history', help: "the open document's earlier versions, as diffs — Enter brings one back" },
  { context: 'tree', keys: ['gb'], action: 'backlinks', help: 'the documents that link to the open one — [[title]] links by title, [[ completes one while writing' },
  { context: 'tree', keys: ['gc'], action: 'cards', help: 'the cards about the open document — j/k picks the one :due, :label, Space and H/L act on, Enter turns it over on its board' },
  { context: 'global', keys: [':'], action: 'line', help: 'command line — Tab / Shift-Tab cycle the matches, then the argument; ↑ ↓ recall what ran before', essential: true },
  { context: 'always', keys: ['Ctrl-s', 'Meta-s'], action: 'save', typing: true, label: 'Ctrl-S', help: 'save, from anywhere' },
  { context: 'global', keys: ['?'], action: 'help', help: 'this screen — the keys to start with, then on a second ? every key and command; h/l a column, / filters it down', essential: true },
  { context: 'global', keys: [','], action: 'settings', help: 'the settings sheet: vim and the clipboard, the vault, the backup, signed-in browsers, ssh keys, capture tokens', essential: true },

  // --- the workspace around the panes ------------------------------------------
  { context: 'workspace', keys: ['Escape'], action: 'escape', typing: true },
  { context: 'workspace', keys: ['Q'], action: 'scratch', section: 'scratch', help: 'lower the scratchpad over whatever is on screen, or raise it again — :scratch <text> takes a note without it', essential: true },
  { context: 'always', keys: ['Ctrl-q'], action: 'scratch', typing: true, section: 'scratch', label: 'Ctrl-Q', help: 'from anywhere, the editor too: the scratchpad down with the last note being written, and again back to where you were, still writing — Firefox quits on it instead', essential: true },
  // While a document is being written, a key that lands outside the editor
  // hands the focus back to it instead of walking the list unseen.
  { context: 'writing', keys: ['*'], action: 'refocus' },

  // --- what the views share: the board, the agenda, the calendar, a card's back
  { context: 'views', keys: ['q', 'Escape'], action: 'close', label: 'q · Esc', caps: ['q'], hint: 'close', help: "on the plan screen: from a card's back to the board, from the board, the agenda or the calendar back to the documents" },

  // --- on a board ---------------------------------------------------------------
  { context: 'board', keys: ['h', 'ArrowLeft'], action: 'left', label: 'h / l', caps: ['h', 'j', 'k', 'l'], hint: 'move', help: 'to the list left / right — right of the last, to the tile beside' },
  { context: 'board', keys: ['l', 'ArrowRight'], action: 'right' },
  { context: 'board', keys: ['j', 'ArrowDown'], action: 'down', label: 'j / k', help: 'down / up the cards' },
  { context: 'board', keys: ['k', 'ArrowUp'], action: 'up' },
  { context: 'board', keys: ['gg'], action: 'top', label: 'gg / G', help: 'first / last card of the list' },
  { context: 'board', keys: ['G'], action: 'bottom' },
  { context: 'board', keys: ['J'], action: 'nudgeDown', label: 'J / K', help: 'move the card down / up its list' },
  { context: 'board', keys: ['K'], action: 'nudgeUp' },
  { context: 'board', keys: ['H'], action: 'sendLeft', label: 'H / L', caps: ['H', 'L'], hint: 'send', help: 'move the card to the list left / right' },
  { context: 'board', keys: ['L'], action: 'sendRight' },
  { context: 'board', keys: ['Space'], action: 'done', hint: 'done', help: 'into the done list, or back out to the inbox' },
  { context: 'board', keys: ['Enter'], action: 'turn', hint: 'turn over', help: 'turn the card over, in a tile beside: its labels, description and checklist' },
  { context: 'board', keys: ['gf'], action: 'notes', help: "the card's notes — a document, made on first use, opened among the documents; S comes back" },
  { context: 'board', keys: ['o', 'c'], action: 'new', label: 'o · c', caps: ['o'], hint: 'new', help: 'new card on this list' },
  { context: 'board', keys: ['D'], action: 'due', hint: 'due', help: 'due date: fri, tm, +3d, 10-31 …' },
  { context: 'board', keys: ['R'], action: 'rename', hint: 'rename', help: 'rename the card, or the list when it is empty' },
  { context: 'board', keys: ['z'], action: 'old', help: 'show done cards older than a week' },
  { context: 'board', keys: ['F'], action: 'hints', help: 'a letter over every card in view — type it to put the cursor there' },
  { context: 'board', keys: ['r'], action: 'reload', help: 'read the board again, for changes made elsewhere' },
  { context: 'board', keys: ['x', 'dd'], action: 'delete', caps: ['dd'], hint: 'delete', help: 'delete the card (asks y/N)' },
  { context: 'board', caps: ['1…9'], hint: 'board' },

  // --- the back of a card ---------------------------------------------------
  { context: 'card', keys: ['j', 'ArrowDown'], action: 'down', label: 'j / k', caps: ['j', 'k'], hint: 'step', help: 'down / up the checklist' },
  { context: 'card', keys: ['k', 'ArrowUp'], action: 'up' },
  { context: 'card', keys: ['gg'], action: 'top', label: 'gg / G', help: 'first / last step' },
  { context: 'card', keys: ['G'], action: 'bottom' },
  { context: 'card', keys: ['Space', 'x'], action: 'tick', label: 'Space · x', caps: ['␣'], hint: 'tick', help: 'tick the step, or untick it' },
  { context: 'card', keys: ['o'], action: 'add', hint: 'add', help: 'a new step below the cursor — :task on the line' },
  { context: 'card', keys: ['r'], action: 'reword', hint: 'reword', help: 'reword the step' },
  { context: 'card', keys: ['J'], action: 'nudgeDown', label: 'J / K', caps: ['J', 'K'], hint: 'order', help: 'move the step down / up' },
  { context: 'card', keys: ['K'], action: 'nudgeUp' },
  { context: 'card', keys: ['dd'], action: 'drop', hint: 'drop', help: 'drop the step' },
  { context: 'card', keys: ['e', 'i', 'Enter'], action: 'describe', caps: ['e'], hint: 'describe', help: 'write the description, in the editor — it saves itself, :q or Esc in normal mode leaves' },
  { context: 'card', keys: ['a'], action: 'label', hint: 'label', help: "put on a label, or take it off — :label on the line, Tab offers the board's" },
  { context: 'card', keys: ['R'], action: 'rename', label: 'R · D', help: 'rename the card, its due date' },
  { context: 'card', keys: ['D'], action: 'due', hint: 'due' },
  { context: 'card', keys: ['H'], action: 'sendLeft', label: 'H / L', caps: ['H', 'L'], hint: 'send', help: 'send the card to the list left / right' },
  { context: 'card', keys: ['L'], action: 'sendRight' },
  { context: 'card', keys: ['gf'], action: 'notes', hint: 'notes', help: "the card's notes — a document, made on first use, opened among the documents; S comes back" },
  { context: 'card', keys: [':'], action: 'line' },

  // --- the scratchpad, while it is down --------------------------------------------
  { context: 'drawer', keys: ['j', 'ArrowDown'], action: 'down', label: 'j / k', caps: ['j', 'k'], hint: 'note', help: 'down / up the notes, the one touched last on top' },
  { context: 'drawer', keys: ['k', 'ArrowUp'], action: 'up' },
  { context: 'drawer', keys: ['gg'], action: 'top' },
  { context: 'drawer', keys: ['G'], action: 'bottom' },
  { context: 'drawer', keys: ['o', 'O', 'c'], action: 'new', label: 'o', caps: ['o'], hint: 'new', help: 'a new note, written straight away — it saves itself' },
  { context: 'drawer', keys: ['Enter', 'e', 'i', 'a'], action: 'write', label: 'Enter · e · i', caps: ['↵'], hint: 'write', help: 'write the note — Esc in normal mode, or :q, comes back to the notes' },
  { context: 'drawer', keys: ['K'], action: 'keep', hint: 'keep as document', help: 'keep the note: a document in the folder being browsed, titled by its first line — :keep <title> names it' },
  { context: 'drawer', keys: ['C'], action: 'card', hint: 'card', help: 'make the note a card where :card puts one — the first line its title, the rest its description' },
  { context: 'drawer', keys: ['A'], action: 'append', hint: 'append', help: 'put the note at the end of a document — :append, with the one on screen filled in; Tab completes a title' },
  { context: 'drawer', keys: ['dd'], action: 'drop', hint: 'drop', help: 'drop the note (asks y/N)' },
  { context: 'drawer', keys: [':'], action: 'line' },
  { context: 'drawer', keys: ['?'], action: 'help' },
  { context: 'drawer', keys: ['q', 'Escape', 'Q'], action: 'raise', label: 'q · Esc · Q', caps: ['Q', 'q'], hint: 'raise', help: 'raise the scratchpad' },
  // Nothing under the drawer hears a key while it is down.
  { context: 'drawer', keys: ['*'], action: 'nothing' },

  // --- the agenda --------------------------------------------------------------
  { context: 'agenda', keys: ['j', 'ArrowDown'], action: 'down', label: 'j / k', caps: ['j', 'k'], hint: 'move', help: 'down / up the cards, then what was written on this day in earlier years, then on to the tile below' },
  { context: 'agenda', keys: ['k', 'ArrowUp'], action: 'up' },
  { context: 'agenda', keys: ['h', 'ArrowLeft'], action: 'left', label: 'h', help: 'to the tile on the left' },
  { context: 'agenda', keys: ['Enter'], action: 'open', hint: 'open', help: 'to the card, on its own board beside — or to the entry' },
  { context: 'agenda', keys: ['Space'], action: 'finish', hint: 'done', help: 'done, where it stands' },
  { context: 'agenda', keys: ['D'], action: 'due', hint: 'due', help: 'due date: fri, tm, +3d … — :due on the line' },
  // The agenda stays up behind the line, so `:due` and `:repeat` know which card.
  { context: 'agenda', keys: [':'], action: 'line', help: 'the line, with the agenda kept up — :repeat weekly acts on the card' },
  { context: 'agenda', keys: ['H'], action: 'sendLeft', label: 'H / L', caps: ['H', 'L'], hint: 'list', help: 'to the list left / right on its board' },
  { context: 'agenda', keys: ['L'], action: 'sendRight' },
  { context: 'agenda', keys: ['T'], action: 'today', hint: 'diary', help: "today's diary entry" },

  // --- the calendar ------------------------------------------------------------
  { context: 'calendar', keys: ['h', 'ArrowLeft', 'l', 'ArrowRight'], action: 'day', caps: ['h', 'l'], hint: 'day' },
  { context: 'calendar', keys: ['k', 'ArrowUp', 'j', 'ArrowDown'], action: 'week', caps: ['j', 'k'], hint: 'week' },
  { context: 'calendar', keys: ['H', 'L'], action: 'month', hint: 'month' },
  { context: 'calendar', keys: ['.'], action: 'today', hint: 'today' },
  { context: 'calendar', keys: ['Enter'], action: 'open', hint: 'open' },

  // --- the sheets over everything --------------------------------------------------
  { context: 'sheet', keys: ['Escape'], action: 'close', typing: true, caps: ['q', 'esc'], hint: 'close' },
  { context: 'sheet', keys: ['q', '?'], action: 'close' },

  { context: 'help', keys: ['l', 'j', 'ArrowRight', 'ArrowDown'], action: 'next', caps: ['h', 'l'], hint: 'column' },
  { context: 'help', keys: ['h', 'k', 'ArrowLeft', 'ArrowUp'], action: 'previous' },
  { context: 'help', keys: ['Ctrl-d'], action: 'pageDown', caps: ['⌃d', '⌃u'], hint: 'page' },
  { context: 'help', keys: ['Ctrl-u'], action: 'pageUp' },
  { context: 'help', keys: ['Space'], action: 'page' },
  { context: 'help', keys: ['g'], action: 'start', caps: ['g', 'G'], hint: 'start · end' },
  { context: 'help', keys: ['G'], action: 'end' },
  { context: 'help', keys: ['/'], action: 'filter', hint: 'filter, and read the lines whole' },
  { context: 'help', keys: ['?'], action: 'deeper', hint: 'all of it' },

  { context: 'overview', keys: ['h', 'ArrowLeft', 'l', 'ArrowRight', 'k', 'ArrowUp', 'j', 'ArrowDown'], action: 'step', caps: ['h', 'j', 'k', 'l'], hint: 'move' },
  { context: 'overview', keys: digits, action: 'go', caps: ['1…9', '↵'], hint: 'go' },
  { context: 'overview', keys: ['Enter'], action: 'enter' },
  { context: 'overview', keys: ['w', 'q'], action: 'close', caps: ['w', 'esc'], hint: 'close' },

  { context: 'info', keys: ['Tab'], action: 'next', caps: ['tab'], hint: 'next part' },
  { context: 'info', keys: ['Shift-Tab'], action: 'previous' },

  { context: 'outline', keys: ['j', 'ArrowDown'], action: 'down', caps: ['j', 'k'], hint: 'move' },
  { context: 'outline', keys: ['k', 'ArrowUp'], action: 'up' },
  { context: 'outline', keys: ['G'], action: 'bottom' },
  { context: 'outline', keys: ['Enter', 'l'], action: 'go', caps: ['↵'], hint: 'jump there' },

  { context: 'files', keys: ['j', 'ArrowDown'], action: 'down', caps: ['j', 'k'], hint: 'move' },
  { context: 'files', keys: ['k', 'ArrowUp'], action: 'up' },
  { context: 'files', keys: ['G'], action: 'bottom' },
  { context: 'files', keys: ['Enter', 'l'], action: 'open', caps: ['↵'], hint: 'open' },
  { context: 'files', keys: ['i'], action: 'insert', hint: 'embed' },
  { context: 'files', keys: ['y'], action: 'copy', hint: 'copy embed' },

  { context: 'trash', keys: ['j', 'ArrowDown'], action: 'down', caps: ['j', 'k'], hint: 'move' },
  { context: 'trash', keys: ['k', 'ArrowUp'], action: 'up' },
  { context: 'trash', keys: ['G'], action: 'bottom' },
  { context: 'trash', keys: ['Enter', 'r'], action: 'restore', caps: ['↵', 'r'], hint: 'restore' },
  { context: 'trash', keys: ['x'], action: 'purge', hint: 'delete for good' },

  { context: 'backlinks', keys: ['j', 'ArrowDown'], action: 'down', caps: ['j', 'k'], hint: 'move' },
  { context: 'backlinks', keys: ['k', 'ArrowUp'], action: 'up' },
  { context: 'backlinks', keys: ['G'], action: 'bottom' },
  { context: 'backlinks', keys: ['Enter'], action: 'open', hint: 'open' },

  { context: 'cards', keys: ['j', 'ArrowDown'], action: 'down', caps: ['j', 'k'], hint: 'move' },
  { context: 'cards', keys: ['k', 'ArrowUp'], action: 'up' },
  { context: 'cards', keys: ['G'], action: 'bottom' },
  { context: 'cards', keys: ['Enter'], action: 'turn', hint: 'turn over' },
  { context: 'cards', keys: ['Space'], action: 'done', hint: 'done' },
  { context: 'cards', keys: ['H'], action: 'sendLeft', caps: ['H', 'L'], hint: 'send' },
  { context: 'cards', keys: ['L'], action: 'sendRight' },
  { context: 'cards', keys: ['D'], action: 'due', hint: 'due' },
  { context: 'cards', keys: ['o', 'c'], action: 'new', caps: ['o'], hint: 'new' },
  { context: 'cards', keys: ['a'], action: 'label', hint: 'label' },
  // The sheet stays up behind the line, so `:due` and `:repeat` know which card.
  { context: 'cards', keys: [':'], action: 'line' },

  { context: 'history', keys: ['j', 'ArrowDown'], action: 'down', caps: ['j', 'k'], hint: 'earlier / later' },
  { context: 'history', keys: ['k', 'ArrowUp'], action: 'up' },
  { context: 'history', keys: ['G'], action: 'bottom' },
  { context: 'history', keys: ['Enter'], action: 'restore', hint: 'restore' },

  { context: 'share', keys: ['s', 'Enter'], action: 'toggle', caps: ['s'], hint: 'share · stop' },
  { context: 'share', keys: ['y'], action: 'link', hint: 'new link' },

  { context: 'settings', keys: ['Tab'], action: 'next', caps: ['tab'], hint: 'next part' },
  { context: 'settings', keys: ['Shift-Tab'], action: 'previous' },
  { context: 'settings', keys: ['j', 'ArrowDown'], action: 'down', caps: ['j', 'k'], hint: 'move' },
  { context: 'settings', keys: ['k', 'ArrowUp'], action: 'up' },
  { context: 'settings', keys: ['G'], action: 'bottom' },
  { context: 'settings', keys: ['Enter', 'l', 'Space'], action: 'act', caps: ['↵'], hint: 'change' },
  { context: 'settings', keys: ['x'], action: 'remove', hint: 'remove' },

  // --- the editor's own keymap, described only ---------------------------------------
  { context: 'editor', label: 'vim', help: 'the full keymap — hjkl, w/b, dd, ciw, visual mode, macros' },
  { context: 'editor', label: 'autosave', help: 'a document, a description or a note saves itself as the typing stops; :w saves sooner', essential: true },
  { context: 'editor', label: ':q · Esc', help: 'leave the editor, what is waiting written first — Esc in normal mode; :q! lets it go', essential: true },
  { context: 'editor', label: '[[', help: 'link a document by its title, offered as you type' },
  { context: 'editor', label: 'drop · paste', help: 'an image, video or audio file attaches it' },
  { context: 'editor', label: '$x^2$ · $$…$$', help: 'set as math' },
  { context: 'editor', label: '```rust', help: 'a fenced block in a language is coloured as nvim colours it, written and read' },
  { context: 'editor', label: '```typst', help: 'a block typst sets, shown as an image — math, tables, figures; no packages or files' },
  { context: 'editor', label: ':12', help: 'jumps straight to entry 12' },
  { context: 'editor', label: 'y · d', help: 'copied to the system clipboard — :set noclipboard keeps them in the registers' },
  { context: 'editor', label: 'files', help: 'open in place — pdf, text and code, markdown, csv, certificates and keystores' },
] as const satisfies readonly Binding[]

/** The help's key columns, in order, and the context each draws from. */
export const sections: readonly { section: Section; title: string; contexts: readonly Context[]; rule?: string }[] = [
  { section: 'keys', title: 'keys', contexts: ['always', 'workspace', 'global', 'tree', 'views'] },
  { section: 'board', title: 'board keys', contexts: ['board'], rule: 'the first list is the inbox, the last list is done' },
  { section: 'card', title: 'card keys', contexts: ['card'] },
  { section: 'scratch', title: 'scratchpad keys', contexts: ['drawer'] },
  { section: 'agenda', title: 'agenda keys', contexts: ['agenda'] },
  { section: 'editor', title: 'in the editor', contexts: ['editor'] },
]

const named: Record<string, string> = { Escape: 'Esc' }

/** The keys as the help writes them. */
export const label = (binding: Binding) => binding.label ?? (binding.keys ?? []).map((key) => named[key] ?? key).join(' · ')

/** The help's first page: the keys marked to start with, in the table's order. */
export function essentials(): { key: string; description: string }[] {
  return (bindings as readonly Binding[])
    .filter((binding) => binding.essential && binding.help)
    .map((binding) => ({ key: label(binding), description: binding.help! }))
}

/** The rows of one help column, in the table's order. */
export function rows(section: Section): { key: string; description: string }[] {
  const contexts = sections.find((entry) => entry.section === section)?.contexts ?? []
  return (bindings as readonly Binding[])
    .filter((binding) => binding.help && (binding.section ?? (contexts.includes(binding.context) ? section : undefined)) === section)
    .map((binding) => ({ key: label(binding), description: binding.help! }))
}

const caps: Record<string, string> = { Enter: '↵', Escape: 'esc', Space: '␣', Tab: 'tab' }

/** The keycaps of a binding: what the footers and the rail show. */
export function keycaps(binding: Binding): string[] {
  if (binding.caps) return [...binding.caps]
  return (binding.keys ?? [])
    .filter((key) => !key.startsWith('Arrow') && !key.startsWith('Shift-'))
    .map((key) => caps[key] ?? key.replace(/^Ctrl-(.)$/, (_, letter: string) => `⌃ ${letter.toUpperCase()}`))
}

/** The keycaps of what `action` is bound to in `context`, for a door on the rail. */
export function keysOf(context: Context, action: string): string[] {
  const binding = (bindings as readonly Binding[]).find((entry) => entry.context === context && entry.action === action)
  if (!binding) throw new Error(`no key for ${context}.${action}`)
  return keycaps(binding)
}

/** The footer of a context: each binding with a hint, in the table's order. */
export function hints(contexts: readonly Context[]): { action?: string; caps: string[]; hint: string }[] {
  return contexts.flatMap((context) =>
    (bindings as readonly Binding[])
      .filter((binding) => binding.context === context && binding.hint)
      .map((binding) => ({ action: binding.action, caps: keycaps(binding), hint: binding.hint! })),
  )
}
