<script lang="ts" module>
  /*
   * The help is a sheet the size of the porthole, laid out in newspaper
   * columns that run on to the right, so all of it is in view at once on a
   * wide screen and a key away on a narrow one. Its keys live in App.svelte
   * with the others; these are what they move.
   */
  let pages: HTMLElement | undefined = $state()
  let field: HTMLInputElement | undefined = $state()
  /** `/` in the help: only the rows whose key or words hold this. */
  export const helpFilter = $state({ text: '' })

  /** A column on or back, a screen on or back, or to either end. */
  export function scrollHelp(by: 1 | -1 | 'pagedown' | 'pageup' | 'start' | 'end') {
    if (!pages) return
    // Columns that run off the right edge scroll sideways; on a phone they
    // stack, and the sheet around them scrolls down instead.
    const sideways = pages.scrollWidth > pages.clientWidth
    const scroller = sideways ? pages : pages.parentElement
    if (!scroller) return
    const axis = sideways ? 'left' : 'top'
    if (by === 'start' || by === 'end') {
      scroller.scrollTo({ [axis]: by === 'start' ? 0 : 1e9, behavior: 'smooth' })
      return
    }
    const page = sideways ? scroller.clientWidth : scroller.clientHeight
    // A row is never broken over columns, so it is as wide as one of them.
    const column = sideways ? (pages.querySelector<HTMLElement>('.row')?.offsetWidth ?? 0) + COLUMN_GAP : page / 2
    const distance = typeof by === 'number' ? by * column : (by === 'pagedown' ? 1 : -1) * page
    scroller.scrollBy({ [axis]: distance, behavior: 'smooth' })
  }

  export function focusHelpFilter() {
    field?.focus()
    field?.select()
  }

  const COLUMN_GAP = 28
</script>

<script lang="ts">
  import { commands, groups, type CommandSpec, type Group } from './commands'
  import { workspace } from './store.svelte'
  import { isTouchDevice } from './util'

  const keys: [string, string][] = [
    ['1 … 9 · 0', 'switch to that space, like a workspace'],
    ['Shift 1 … 9', 'send the marked nodes, or the selected one, to that space'],
    ['Tab · Shift-Tab', 'next / previous space'],
    ['w', 'overview of every space — hjkl, then Enter'],
    ['f', 'fullscreen the focused pane or tile; again or Esc to leave'],
    ['Alt-h/j/k/l', 'focus the tile that way, from the editor too — left of the leftmost is the list'],
    ['Alt-Shift-h/j/k/l', 'move the focused tile that way'],
    ['L · Shift-Enter', 'open the row in a new tile beside the others'],
    ['gt / gT', 'next / previous tile — in the editor too'],
    ['Alt-q', 'close the focused tile — :close; :only closes the others'],
    ['j / k · ↓ ↑', 'move down / up the list — scroll the document when it has focus'],
    ['gg / G', 'first / last row, or top / bottom of the document — a first g, d, ] or [ waits a second in the status line'],
    ['Ctrl-D / Ctrl-U', 'half a page down / up the open document'],
    ['zl / zh · zL / zH', 'the wide table or code block at the top of the view, sideways — a little, or half its width'],
    [']] / [[', 'next / previous heading of the document — in the editor too'],
    ['gO', 'the headings as a list: j/k, Enter jumps there'],
    ['F', 'hints over the links and buttons in view — type one to follow it'],
    ['gf / gx', 'in the editor: follow the link under the cursor / open it in a tab'],
    ['gx', 'open the file under the cursor in a tab of its own'],
    ['Enter · l', 'open a document, or enter a folder — on an open keystore or sealed file, Enter asks for its password'],
    ['h', 'from the document to the list, from the list up a level'],
    ['o', 'new document here, straight into insert mode'],
    ['O', 'new folder here'],
    ['i · a', 'edit the open document'],
    ['R', 'rename what is under the cursor — document, folder or card — starting from its name'],
    ['m / p', 'pick nodes up, then drop them all into the folder you walk to'],
    ['M', 'pick up the whole folder — again puts it all back'],
    ['Ctrl / Shift-click', 'pick a row up · every row up to it — drag them onto a folder'],
    ['t', 'agenda — late, due this week, in hand, on every board'],
    ['T', "today's diary entry, started if there is none — from templates/day when the diary has one"],
    ['C', 'the diary by month: h/l a day, j/k a week, H/L a month, . today, Enter opens or starts the day'],
    ['[d / ]d', 'the diary entry before / after the one open, by date'],
    ['Ctrl-P', 'find anything, in every space — Enter goes to the result lit'],
    ['b', "open this space's board"],
    ['c', "capture a card into this space's inbox, without leaving here"],
    ['Esc · q', 'close the document, and its tile when there are several'],
    ['/', 'full-text search, inside the current space and its text files — Tab or Ctrl-N/P walk the results, Enter opens one'],
    ['n / N', 'next / previous search result, opened beside the results — :noh clears the search'],
    ['s', 'toggle sharing for the document — asks before taking it back'],
    ['y', 'mint a new share link and copy it — asks, the old one stops working'],
    ['x · dd', 'delete what has focus — the row in the list, the document in its pane — into the trash (asks y/N)'],
    ['u', 'undo the last delete: it comes back out of the trash — :trash has the rest'],
    ['gh', "the open document's earlier versions, as diffs — Enter brings one back"],

    ['gb', 'the documents that link to the open one — [[title]] links by title, [[ completes one while writing'],
    [':', 'command line — Tab / Shift-Tab cycle the matches, then the argument; ↑ ↓ recall what ran before'],
    ['Ctrl-S', 'save, from anywhere'],
    ['?', 'this screen, from the editor too — h/l a column, / filters it down'],
  ]

  /* The board rebinds most keys, so it gets its own column rather than
     footnotes on the list keys. */
  const boardKeys: [string, string][] = [
    ['rule', 'the first list is the inbox, the last list is done'],
    ['h / l', 'to the list left / right'],
    ['j / k', 'down / up the cards'],
    ['gg / G', 'first / last card of the list'],
    ['J / K', 'move the card down / up its list'],
    ['H / L', 'move the card to the list left / right'],
    ['Space', 'into the done list, or back out to the inbox'],
    ['Enter', "the card's notes — a document, made on first use; q comes back"],
    ['o · c', 'new card on this list'],
    ['D', 'due date: fri, tm, +3d, 10-31 …'],
    ['R', 'rename the card, or the list when it is empty'],
    ['z', 'show done cards older than a week'],
    ['F', 'a letter over every card in view — type it to put the cursor there'],
    ['r', 'read the board again, for changes made elsewhere'],
    ['x · dd', 'delete the card (asks y/N)'],
    ['q · Esc', 'back to the tree'],
  ]

  const agendaKeys: [string, string][] = [
    ['j / k', 'down / up the cards, then what was written on this day in earlier years'],
    ['Enter', 'to the card, on its own board — or to the entry'],
    ['Space', 'done, where it stands'],
    ['D', 'due date: fri, tm, +3d … — :due on the line'],
    [':', 'the line, with the agenda kept up — :repeat weekly acts on the card'],
    ['H / L', 'to the list left / right on its board'],
    ['T', "today's diary entry"],
    ['t · q · Esc', 'close the agenda'],
  ]

  const titles: Record<Group, string> = {
    tree: 'spaces & folders',
    board: 'boards',
    documents: 'documents',
    tiles: 'tiles',
    editing: 'writing',
    sharing: 'sharing & export',
    vault: 'vault',
    view: 'session',
  }

  type Row = { key: string; args?: string; description: string }

  const fromPairs = (pairs: [string, string][]): Row[] =>
    pairs.map(([key, description]) => ({ key, description }))

  /** `:write(w)` — the name first, then whatever else reaches it. */
  const signature = (spec: CommandSpec) =>
    `:${spec.name}${spec.aliases?.length ? `(${spec.aliases.join(' ')})` : ''}`

  const fromSpecs = (specs: CommandSpec[]): Row[] =>
    specs.flatMap((spec) => [
      { key: signature(spec), args: spec.args, description: spec.help },
      ...(spec.bang ? [{ key: `:${spec.name}!`, description: spec.bang.help }] : []),
    ])

  const editorKeys: [string, string][] = [
    ['vim', 'the full keymap — hjkl, w/b, dd, ciw, visual mode, macros; :w writes'],
    ['drop · paste', 'an image, video or audio file attaches it'],
    ['$x^2$ · $$…$$', 'set as math'],
    [':12', 'jumps straight to entry 12'],
    ['y · d', 'copied to the system clipboard — :set noclipboard keeps them in the registers'],
    ['files', 'open in place — pdf, text and code, markdown, csv, certificates and keystores'],
  ]

  const sections: [string, Row[]][] = [
    ['keys', fromPairs(keys)],
    ['board keys', fromPairs(boardKeys)],
    ['agenda keys', fromPairs(agendaKeys)],
    ['in the editor', fromPairs(editorKeys)],
    ...groups.map((group: Group) => [titles[group], fromSpecs(commands.filter((spec) => spec.group === group))] as [string, Row[]]),
  ]

  const shown = $derived.by(() => {
    const words = helpFilter.text.trim().toLowerCase().split(/\s+/).filter(Boolean)
    if (words.length === 0) return sections
    const hit = (row: Row) => {
      const text = `${row.key} ${row.args ?? ''} ${row.description}`.toLowerCase()
      return words.every((word) => text.includes(word))
    }
    return sections.map(([title, rows]) => [title, rows.filter(hit)] as [string, Row[]]).filter(([, rows]) => rows.length > 0)
  })
  const count = $derived(shown.reduce((sum, [, rows]) => sum + rows.length, 0))
  const filtering = $derived(helpFilter.text.trim() !== '')

  /* What a row says before its first dash is enough to find it by; the rest
     comes back once a filter has narrowed the sheet down to a few. */
  const brief = (description: string) => description.split(' — ')[0]
  /** The key column of a section, as wide as its longest key, within reason. */
  const keyWidth = (rows: Row[]) => Math.min(Math.max(...rows.map((row) => Math.max(row.key.length, row.args?.length ?? 0))), 14)

  // A filter left over from last time would hide rows without saying so.
  helpFilter.text = ''

  /* The field takes its own Enter and Esc, so they settle the filter
     instead of closing the help underneath it. */
  function filterKey(event: KeyboardEvent) {
    if (event.key !== 'Escape' && event.key !== 'Enter') return
    event.preventDefault()
    event.stopPropagation()
    if (event.key === 'Escape') helpFilter.text = ''
    field?.blur()
    field?.closest<HTMLElement>('.overlay')?.focus()
  }
</script>

<div class="sheet">
  <header>
    <span class="label rule"><span class="marker"></span><span class="title">help</span> {count} keys and commands</span>
    <label class="filter" class:on={helpFilter.text !== ''}>
      <span class="slash">/</span>
      <input
        bind:this={field}
        bind:value={helpFilter.text}
        onkeydown={filterKey}
        placeholder="filter"
        spellcheck="false"
        autocomplete="off"
      />
    </label>
    <button onclick={() => (workspace.overlay = 'none')}>esc</button>
  </header>

  <div class="cols" bind:this={pages}>
    {#each shown as [title, rows] (title)}
      <section style="--key: {keyWidth(rows)}ch">
        <h2>{title}</h2>
        <dl>
          {#each rows as row (row.key)}
            {@const short = filtering ? row.description : brief(row.description)}
            <div class="row">
              <dt>{row.key}{#if row.args}<span class="args">{row.args}</span>{/if}</dt>
              <dd>{short}{#if short !== row.description && !short.endsWith('…')}<span class="more" title={row.description}> …</span>{/if}</dd>
            </div>
          {/each}
        </dl>
      </section>
    {:else}
      <p class="none">nothing matches "{helpFilter.text}"</p>
    {/each}
  </div>

  {#if !isTouchDevice}
    <footer class="label">
      <span class="kbd">h</span><span class="kbd">l</span> column
      <span class="gap"></span>
      <span class="kbd">⌃d</span><span class="kbd">⌃u</span> page
      <span class="gap"></span>
      <span class="kbd">g</span><span class="kbd">G</span> start · end
      <span class="gap"></span>
      <span class="kbd">/</span> filter, and read the lines whole
      <span class="gap"></span>
      <span class="kbd">?</span><span class="kbd">esc</span> close
    </footer>
  {/if}
</div>

<style>
  /* A free-standing panel, as the desktop draws one: hull, one rim, the
     frame's radius — as big as the porthole, so nothing hides below it. */
  .sheet {
    display: flex;
    flex-direction: column;
    gap: 14px;
    width: 100%;
    height: 100%;
    min-height: 0;
    background: color-mix(in oklab, var(--hull) 97%, transparent);
    border: 1px solid var(--surface-alt);
    border-radius: var(--frame-radius);
    padding: 18px 22px 14px;
  }

  header { display: flex; align-items: center; gap: 12px; }
  header .label { flex: 1; }
  header .marker { background: var(--light); }
  header .title { color: var(--light); }

  .filter {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 24ch;
    padding: 0 8px;
    border: 1px solid var(--rim);
    border-radius: var(--radius-s);
    color: var(--muted);
  }
  .filter:focus-within, .filter.on { border-color: var(--outline); color: var(--light); }
  .filter input {
    flex: 1;
    min-width: 0;
    padding: 4px 0;
    border: 0;
    background: none;
    color: var(--light);
    font: inherit;
    font-size: 12px;
    outline: none;
  }

  /*
   * Newspaper columns of a fixed height, so they fill across before they
   * run off the right edge, where h/l reach them. A group may break between
   * columns, but never straight after its heading.
   */
  .cols {
    flex: 1;
    min-height: 0;
    columns: 40ch;
    column-gap: 28px;
    column-fill: auto;
    column-rule: 1px solid var(--surface-alt);
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: none;
  }

  section { margin: 0 0 12px; }

  h2 {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 0 0 6px;
    font-size: 10px;
    font-weight: 400;
    letter-spacing: 0.22em;
    text-transform: uppercase;
    color: var(--muted);
    break-after: avoid;
  }
  h2::after { content: ''; height: 1px; flex: 1; background: var(--overlay); }

  /* One row is never split between columns. The key column is as wide as
     the section's longest key up to a cap, so one long signature cannot
     squeeze every description down; arguments wrap under the name. */
  dl { margin: 0; font-size: 12px; line-height: 1.35; }
  .row {
    display: grid;
    grid-template-columns: var(--key) minmax(0, 1fr);
    gap: 12px;
    padding: 1px 0;
    break-inside: avoid;
  }
  dt { color: var(--light); overflow-wrap: break-word; }
  .args { display: block; font-size: 10px; color: var(--muted); }
  dd { margin: 0; color: var(--subtle); overflow-wrap: break-word; }
  .more { color: var(--outline); }

  .none { margin: 0; color: var(--subtle); font-size: 12px; }

  footer { display: flex; flex-wrap: wrap; align-items: center; gap: 4px; font-size: 10px; }
  footer .kbd { margin-right: 2px; }
  footer .gap { width: 14px; }

  /* A phone stacks the columns and scrolls the sheet down. */
  @media (max-width: 767px) {
    .sheet { overflow-y: auto; }
    .cols { flex: none; columns: auto; overflow: visible; }
    .filter { width: 14ch; }
  }
</style>
