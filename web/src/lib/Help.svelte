<script lang="ts">
  import { commands, groups, type CommandSpec, type Group } from './commands'
  import { workspace } from './store.svelte'

  const keys: [string, string][] = [
    ['1 … 9 · 0', 'switch to that space, like a workspace'],
    ['Shift 1 … 9', 'send the marked nodes, or the selected one, to that space'],
    ['Tab · Shift-Tab', 'next / previous space'],
    ['w', 'overview of every space — hjkl, then Enter'],
    ['f', 'fullscreen the focused pane; again or Esc to leave'],
    ['Alt-h / Alt-l', 'focus the list / the document, from the editor too'],
    ['j / k · ↓ ↑', 'move down / up the list — scroll the document when it has focus'],
    ['gg / G', 'first / last row, or top / bottom of the document'],
    ['Ctrl-D / Ctrl-U', 'half a page down / up the open document'],
    ['Enter · l', 'open a document, or enter a folder'],
    ['h', 'from the document to the list, from the list up a level'],
    ['o', 'new document here, straight into insert mode'],
    ['O', 'new folder here'],
    ['i · a', 'edit the open document'],
    ['R', 'rename what is under the cursor — document, folder or card'],
    ['m / p', 'pick nodes up, then drop them all into the folder you walk to'],
    ['M', 'pick up the whole folder — again puts it all back'],
    ['Ctrl / Shift-click', 'pick a row up · every row up to it — drag them onto a folder'],
    ['t', 'agenda — late, due this week, in hand, on every board'],
    ['T', "today's diary entry, started if there is none"],
    ['Ctrl-P', 'find anything, in every space'],
    ['b', "open this space's board"],
    ['c', "capture a card into this space's inbox, without leaving here"],
    ['Esc · q', 'close the document'],
    ['/', 'full-text search, inside the current space'],
    ['n', 'clear the search'],
    ['s', 'toggle sharing for the document'],
    ['y', 'mint a new share link and copy it'],
    ['x · dd', 'delete what has focus — the row in the list, the document in its pane (asks y/N)'],
    [':', 'command line'],
    ['Ctrl-S', 'save, from anywhere'],
    ['?', 'this screen — in the editor too'],
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
    ['r', 'read the board again, for changes made elsewhere'],
    ['x · dd', 'delete the card (asks y/N)'],
    ['q · Esc', 'back to the tree'],
  ]

  const agendaKeys: [string, string][] = [
    ['j / k', 'down / up the cards'],
    ['Enter', 'to the card, on its own board'],
    ['Space', 'done, where it stands'],
    ['T', "today's diary entry"],
    ['t · q · Esc', 'close the agenda'],
  ]

  const titles: Record<Group, string> = {
    tree: 'spaces & folders',
    board: 'boards',
    documents: 'documents',
    editing: 'writing',
    sharing: 'sharing & export',
    vault: 'vault',
    view: 'session',
  }

  const grouped = groups.map(
    (group) => [group, commands.filter((spec) => spec.group === group)] as [Group, CommandSpec[]],
  )

  /** `:write(w)` — the name first, then whatever else reaches it. */
  const signature = (spec: CommandSpec) =>
    `:${spec.name}${spec.aliases?.length ? `(${spec.aliases.join(' ')})` : ''}`
</script>

<div class="sheet">
  <header>
    <span class="label rule"><span class="marker"></span><span class="title">help</span> everything is a key or a `:` command</span>
    <button onclick={() => (workspace.overlay = 'none')}>esc</button>
  </header>

  <div class="cols">
    <section class="keys">
      <h2>keys</h2>
      <dl>
        {#each keys as [key, description] (key)}
          <dt>{key}</dt>
          <dd>{description}</dd>
        {/each}
      </dl>
    </section>

    <section class="keys">
      <h2>board keys</h2>
      <dl>
        {#each boardKeys as [key, description] (key)}
          <dt>{key}</dt>
          <dd>{description}</dd>
        {/each}
      </dl>
    </section>

    <section class="keys">
      <h2>agenda keys</h2>
      <dl>
        {#each agendaKeys as [key, description] (key)}
          <dt>{key}</dt>
          <dd>{description}</dd>
        {/each}
      </dl>
    </section>

    {#each grouped as [group, specs] (group)}
      <section>
        <h2>{titles[group]}</h2>
        <dl>
          {#each specs as spec (spec.name)}
            <dt>
              {signature(spec)}{#if spec.args}<span class="args muted">{spec.args}</span>{/if}
            </dt>
            <dd>{spec.help}</dd>
            {#if spec.bang}
              <dt>:{spec.name}!</dt>
              <dd>{spec.bang.help}</dd>
            {/if}
          {/each}
        </dl>
      </section>
    {/each}
  </div>

  <footer>
    inside the editor the full vim keymap is live — hjkl, w/b, dd, ciw, visual mode, macros, and
    <span class="key">:w</span> to write. drop or paste an image, video or audio file into the
    editor to attach it. <span class="key">$x^2$</span> and <span class="key">$$…$$</span> are set as math. a file opens in place — pdf, text and code, markdown, csv, and
    certificates and keystores, which ask for their password. <span class="key">:12</span> jumps straight to entry 12. yanks and
    deletes are copied to the system clipboard —
    <span class="key">:set noclipboard</span> keeps them in vim's registers.
  </footer>
</div>

<style>
  /* A free-standing panel, as the desktop draws one: hull, one rim, the frame's radius. */
  .sheet {
    display: flex;
    flex-direction: column;
    gap: 18px;
    max-height: 100%;
    background: color-mix(in oklab, var(--hull) 97%, transparent);
    border: 1px solid var(--surface-alt);
    border-radius: var(--frame-radius);
    padding: 20px 24px;
  }

  header { display: flex; align-items: center; gap: 12px; }
  header .label { flex: 1; }
  header .marker { background: var(--light); }
  header .title { color: var(--light); }
  .sheet { width: min(100%, 112ch); overflow-y: auto; overflow-x: hidden; }

  /* Newspaper columns, so the short groups fill the gaps under the long ones. */
  .cols { columns: 44ch; column-gap: 36px; }

  section { break-inside: avoid; min-width: 0; margin: 0 0 20px; }

  h2 {
    display: flex;
    align-items: center;
    gap: 12px;
    margin: 0 0 8px;
    font-size: 10px;
    font-weight: 400;
    letter-spacing: 0.22em;
    text-transform: uppercase;
    color: var(--muted);
  }
  h2::after { content: ''; height: 1px; flex: 1; background: var(--overlay); }

  /* The key column is capped, so one long signature cannot squeeze every
     description in its group down to a letter a line; arguments wrap
     underneath the name instead. */
  dl {
    margin: 0;
    display: grid;
    grid-template-columns: fit-content(20ch) minmax(0, 1fr);
    gap: 7px 16px;
    font-size: 13px;
    line-height: 1.45;
  }
  dt { color: var(--light); white-space: nowrap; }
  .args { display: block; white-space: normal; font-size: 11px; }
  dd { margin: 0; color: var(--fg); overflow-wrap: break-word; }

  footer {
    font-size: 12px;
    color: var(--muted);
    border-top: 1px solid var(--overlay);
    padding-top: 12px;
    line-height: 1.7;
  }
  .key { color: var(--light); }
</style>
