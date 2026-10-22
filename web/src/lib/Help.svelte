<script lang="ts" module>
  /*
   * The help is a sheet the size of the porthole, laid out in newspaper
   * columns that run on to the right, so all of it is in view at once on a
   * wide screen and a key away on a narrow one. It comes in two depths: the
   * keys and commands to start with, then, on a second `?`, every one of
   * them. Its keys live in keymap.ts with the others; these are what they
   * move.
   */
  import { workspace } from './store.svelte'

  let pages: HTMLElement | undefined = $state()
  let field: HTMLInputElement | undefined = $state()
  /** `/` in the help: only the rows whose key or words hold this. */
  export const helpFilter = $state({ text: '' })
  /** Every key and command, rather than the ones to start with. */
  export const helpDepth = $state({ all: false })

  export function openHelp(all = false) {
    helpDepth.all = all
    workspace.sheets.open('help')
  }

  /** `?` in the help: everything, and the help away once it is all on show. */
  export function deeperHelp() {
    if (helpDepth.all) workspace.sheets.close()
    else helpDepth.all = true
  }

  /** A column on or back, a screen on or back, or to either end. */
  export function scrollHelp(by: 1 | -1 | 'pagedown' | 'pageup' | 'start' | 'end') {
    if (!pages) return
    // Columns that run off the right edge scroll sideways; on a phone they
    // stack, and scroll down instead.
    const sideways = pages.scrollWidth > pages.clientWidth
    const scroller = pages
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
  import Sheet from './Sheet.svelte'
  import { essentials, rows, sections as keySections } from './keymap'

  const titles: Record<Group, string> = {
    tree: 'spaces & folders',
    board: 'boards',
    documents: 'documents',
    tiles: 'tiles',
    editing: 'writing',
    sharing: 'sharing & export',
    vault: 'vault',
    view: 'settings & session',
  }

  type Row = { key: string; args?: string; description: string }

  /** `:write(w)` — the name first, then whatever else reaches it. */
  const signature = (spec: CommandSpec) =>
    `:${spec.name}${spec.aliases?.length ? `(${spec.aliases.join(' ')})` : ''}`

  const fromSpecs = (specs: CommandSpec[], bangs = true): Row[] =>
    specs.flatMap((spec) => [
      { key: signature(spec), args: spec.args, description: spec.help },
      ...(spec.bang && bangs ? [{ key: `:${spec.name}!`, description: spec.bang.help }] : []),
    ])

  /* The keys come out of the keymap that dispatches them, so the help
     cannot describe a key that does something else. */
  const sections: [string, Row[]][] = [
    ...keySections.map(({ section, title, rule }) => [title, [...(rule ? [{ key: 'rule', description: rule }] : []), ...rows(section)]] as [string, Row[]]),
    ...groups.map((group: Group) => [titles[group], fromSpecs(commands.filter((spec) => spec.group === group))] as [string, Row[]]),
  ]

  /* The first page: what a day in the workspace needs, and where the rest is. */
  const first: [string, Row[]][] = [
    ['to start with', essentials()],
    ['commands to start with', fromSpecs(commands.filter((spec) => spec.essential), false)],
  ]

  const shown = $derived.by(() => {
    const words = helpFilter.text.trim().toLowerCase().split(/\s+/).filter(Boolean)
    // A filter looks through all of it: what is looked for is rarely on the first page.
    if (words.length === 0) return helpDepth.all ? sections : first
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

<Sheet
  title="help"
  detail={helpDepth.all || filtering ? `${count} keys and commands` : `the ${count} to start with — ? again for all of them`}
  width={0}
  fill
  keys={['help', 'sheet']}
>
  {#snippet actions()}
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
  {/snippet}

  <div class="cols" bind:this={pages}>
    {#each shown as [title, rows] (title)}
      <section style="--key: {keyWidth(rows)}ch">
        <h2>{title}</h2>
        <dl>
          {#each rows as row (row.key + row.description)}
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
</Sheet>

<style>
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

  /* A phone stacks the columns and scrolls them down. */
  @media (max-width: 767px) {
    .cols { columns: auto; overflow-y: auto; }
    .filter { width: 14ch; }
  }
</style>
