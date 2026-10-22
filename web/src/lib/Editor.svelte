<script lang="ts" module>
  import { EditorView } from '@codemirror/view'
  import { Vim, getCM, vim } from '@replit/codemirror-vim'
  import { commands, cycleTile, followLink, runCommand } from './commands'
  import { linkAt, sourceHeadings } from './util'
  import { workspace as ws } from './store.svelte'

  /*
   * Vim's actions, ex commands and registers are global: one definition
   * serves every editor on the page. Each action is handed the editor the key
   * was pressed in, so with several documents open it acts on that one.
   */
  let vimBound = false

  /** What vim hands an action: CodeMirror 5's face over the editor. */
  type Adapter = { cm6: EditorView }

  /** Route every `:command` typed inside the editor through the shared handler. */
  function bridgeExCommands() {
    for (const spec of commands) {
      const names = [spec.name, ...(spec.aliases ?? [])]
      for (const name of names) {
        Vim.defineEx(name, name, (_cm: unknown, params: { argString?: string }) => {
          const arg = (params.argString ?? '').trim()
          // vim parses `:q!` as command `q` with the bang left in the arguments,
          // so put it back on the name before handing the line over.
          const bang = arg.startsWith('!')
          void runCommand(`${spec.name}${bang ? '!' : ''} ${bang ? arg.slice(1).trim() : arg}`)
        })
      }
    }
  }

  /**
   * `set clipboard=unnamed`, in effect: every yank or delete that lands in the
   * unnamed register is copied out to the system clipboard too. An explicit
   * register (`"ay`) is left alone, and `"+p` still pastes the clipboard back.
   */
  function mirrorUnnamedRegisterToClipboard() {
    const registers = Vim.getRegisterController?.()
    if (!registers) return
    const push = registers.pushText.bind(registers)
    registers.pushText = (name, operator, text, linewise, blockwise) => {
      push(name, operator, text, linewise, blockwise)
      if (!name && text && ws.clipboard) void ws.copy(text)
    }
  }

  /**
   * `?` is vim's search-backwards, but the workspace advertises it as the help key
   * everywhere else, so it opens the reference here too. `/` still searches.
   */
  function bindHelpKey() {
    Vim.defineAction('workspaceHelp', () => void runCommand('help'))
    for (const context of ['normal', 'visual'] as const) {
      Vim.mapCommand('?', 'action', 'workspaceHelp', {}, { context })
    }
  }

  function gotoLine(view: EditorView, number: number) {
    const line = view.state.doc.line(number)
    view.dispatch({
      selection: { anchor: line.from },
      effects: EditorView.scrollIntoView(line.from, { y: 'start', yMargin: 24 }),
    })
    view.focus()
  }

  /*
   * vim's `]]` and `[[` walk sections, which in a markdown file are the
   * headings — the same jump the reader makes, where the text is. `gO` lists
   * them, as it does in vim's own help files.
   */
  function bindHeadingKeys() {
    const step = (view: EditorView, direction: 1 | -1) => {
      const at = view.state.doc.lineAt(view.state.selection.main.head).number
      const lines = sourceHeadings(view.state.doc.toString()).map((heading) => heading.line)
      let target: number | undefined
      for (const line of lines) {
        if (direction > 0 && line > at) {
          target = line
          break
        }
        if (direction < 0 && line < at) target = line
      }
      gotoLine(view, target ?? (direction > 0 ? view.state.doc.lines : 1))
    }
    Vim.defineAction('workspaceNextHeading', (cm) => step((cm as unknown as Adapter).cm6, 1))
    Vim.defineAction('workspacePrevHeading', (cm) => step((cm as unknown as Adapter).cm6, -1))
    Vim.defineAction('workspaceOutline', () => void runCommand('outline'))
    Vim.mapCommand(']]', 'action', 'workspaceNextHeading', {}, { context: 'normal' })
    Vim.mapCommand('[[', 'action', 'workspacePrevHeading', {}, { context: 'normal' })
    Vim.mapCommand('gO', 'action', 'workspaceOutline', {}, { context: 'normal' })
  }

  /*
   * `gf` follows the link under the cursor — a workspace link opens that
   * document, saving this one first, anything else gets a tab — and `gx`
   * opens it in a tab whatever it is, as vim hands it to the browser.
   */
  function bindLinkKeys() {
    const follow = (view: EditorView, tab: boolean) => {
      const head = view.state.selection.main.head
      const line = view.state.doc.lineAt(head)
      const href = linkAt(line.text, head - line.from)
      if (!href) return ws.say('no link under the cursor', 'error')
      followLink(href, tab)
    }
    Vim.defineAction('workspaceFollowLink', (cm) => follow((cm as unknown as Adapter).cm6, false))
    Vim.defineAction('workspaceOpenLink', (cm) => follow((cm as unknown as Adapter).cm6, true))
    Vim.mapCommand('gf', 'action', 'workspaceFollowLink', {}, { context: 'normal' })
    Vim.mapCommand('gx', 'action', 'workspaceOpenLink', {}, { context: 'normal' })
  }

  /* `gt` / `gT` walk the tiles from inside one, as they walk vim's tab pages. */
  function bindTileKeys() {
    Vim.defineAction('workspaceNextTile', () => void cycleTile(1))
    Vim.defineAction('workspacePrevTile', () => void cycleTile(-1))
    Vim.mapCommand('gt', 'action', 'workspaceNextTile', {}, { context: 'normal' })
    Vim.mapCommand('gT', 'action', 'workspacePrevTile', {}, { context: 'normal' })
  }

  export function bindVim() {
    if (vimBound) return
    vimBound = true
    bridgeExCommands()
    bindHelpKey()
    bindHeadingKeys()
    bindLinkKeys()
    bindTileKeys()
    mirrorUnnamedRegisterToClipboard()
  }
</script>

<script lang="ts">
  /*
   * One editor for everything written: a document's draft in its tile, and
   * the short texts written in place — a card's description, a note. Vim,
   * markdown, `[[` completion, the colours, the autosave behind it and the
   * keys out of it are the same in each: `:w` writes sooner, `:q` or Esc in
   * normal mode leaves with what is waiting written, `:q!` lets it go.
   */
  import { autocompletion, completionStatus, type CompletionContext, type CompletionResult } from '@codemirror/autocomplete'
  import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands'
  import { markdown, markdownLanguage } from '@codemirror/lang-markdown'
  import { indentOnInput } from '@codemirror/language'
  import { EditorState } from '@codemirror/state'
  import {
    drawSelection,
    dropCursor,
    highlightActiveLine,
    keymap,
    placeholder,
  } from '@codemirror/view'
  import { onDestroy, onMount } from 'svelte'
  import { api } from './api'
  import { hooks } from './commands'
  import { editorTheme, highlighting, languages } from './editorTheme'
  import { embedSnippet } from './markdown'
  import { workspace, type Tile } from './store.svelte'
  import type { Writing } from './writing.svelte'

  /**
   * What is written: the draft of the document in `tile`, which need not be
   * the one with focus, or the short text in `writing`, framed with a mode
   * line of its own since no status line speaks for it — as tall as it needs,
   * or filling what it is put in.
   */
  const {
    tile,
    writing,
    hint = 'write…',
    fill = false,
  }: { tile?: Tile; writing?: Writing; hint?: string; fill?: boolean } = $props()

  let host = $state<HTMLDivElement>()
  let view: EditorView | undefined
  let mode = $state('normal')
  const focus = () => view?.focus()

  /*
   * Esc, before vim sees it: in normal mode, or with no vim, it leaves, as
   * `:q` does — unless it is closing the completions.
   */
  function onkeydown(event: KeyboardEvent) {
    if (event.key !== 'Escape' || (workspace.vimEnabled && mode !== 'normal')) return
    if (view && completionStatus(view.state) !== null) return
    event.preventDefault()
    event.stopPropagation()
    void runCommand('quit')
  }

  /*
   * `[[` offers the titles of documents as the search finds them, each with
   * where it sits; picking one closes the brackets.
   */
  async function titleCompletions(context: CompletionContext): Promise<CompletionResult | null> {
    const typed = context.matchBefore(/\[\[[^[\]|\n]*/)
    if (!typed) return null
    const query = typed.text.slice(2).trim()
    if (!query) return null
    const found = await api.listNodes({ q: query }).catch(() => [])
    if (context.aborted) return null
    const options = found
      .filter((node) => node.kind === 'document' || node.kind === 'file')
      .filter((node) => node.id !== tile?.open?.id)
      .map((node) => {
        // What a `[[link]]` can hold: a name with no bracket or bar in it, or else the slug.
        const name = node.name.trim()
        const title = name && !/[[\]|\n]/.test(name) ? name : node.slug
        return { label: title, detail: node.location, apply: `${title}]]` }
      })
    return { from: typed.from + 2, options }
  }

  /** The `#` lines being written, as `gO` lists them, and the one the cursor is under. */
  function editorHeadings() {
    if (!view) return { headings: [], current: 0 }
    const editor = view
    const at = editor.state.doc.lineAt(editor.state.selection.main.head).number
    const found = sourceHeadings(editor.state.doc.toString())
    let current = 0
    found.forEach((heading, i) => {
      if (heading.line <= at) current = i
    })
    const headings = found.map(({ text, level, line }) => ({ text, level, go: () => gotoLine(editor, line) }))
    return { headings, current }
  }

  /**
   * Media goes on its own line after the one the cursor is on, so attaching a
   * file never cuts a sentence in half.
   */
  function insertAtCursor(text: string) {
    if (!view) return
    const line = view.state.doc.lineAt(view.state.selection.main.head)
    const blank = line.text.trim() === ''
    const at = blank ? line.from : line.to
    const insert = blank ? `${text}\n` : `\n\n${text}\n`
    view.dispatch({
      changes: { from: at, to: blank ? line.to : at, insert },
      selection: { anchor: at + insert.length },
      scrollIntoView: true,
    })
    view.focus()
  }

  async function uploadFiles(files: File[]) {
    if (files.length === 0) return
    await workspace.guard(async () => {
      const created = await workspace.tree.attach(files, tile)
      if (created.length > 0) insertAtCursor(created.map(embedSnippet).join('\n\n'))
    })
  }

  /** What a change, `Ctrl-S` and the starting text mean for what is written. */
  const changed = (text: string) => {
    if (writing) return writing.change(text)
    tile!.draft.body = text
    workspace.drafts.touch(tile)
  }
  const save = () => void workspace.guard(() => (writing ? workspace.drafts.saveWriting() : workspace.drafts.save({ tile })))

  onMount(() => {
    bindVim()

    const extensions = [
      ...(workspace.vimEnabled ? [vim({ status: false })] : []),
      history(),
      drawSelection(),
      dropCursor(),
      indentOnInput(),
      highlightActiveLine(),
      EditorState.allowMultipleSelections.of(true),
      markdown({ base: markdownLanguage, codeLanguages: languages }),
      autocompletion({ override: [titleCompletions], icons: false }),
      highlighting,
      EditorView.lineWrapping,
      placeholder(hint),
      editorTheme,
      ...(writing ? [EditorView.theme({ '.cm-content': { padding: '10px 14px', maxWidth: 'none' } })] : []),
      keymap.of([
        { key: 'Mod-s', preventDefault: true, run: () => (save(), true) },
        ...defaultKeymap,
        ...historyKeymap,
        indentWithTab,
      ]),
      EditorView.updateListener.of((update) => {
        if (update.docChanged) changed(update.state.doc.toString())
      }),
      // A file goes into a document; a card or a note has nowhere to keep one.
      ...(tile
        ? [
            EditorView.domEventHandlers({
              paste: (event) => {
                const files = Array.from(event.clipboardData?.files ?? [])
                if (files.length === 0) return false
                event.preventDefault()
                void uploadFiles(files)
                return true
              },
              drop: (event) => {
                const files = Array.from(event.dataTransfer?.files ?? [])
                if (files.length === 0) return false
                event.preventDefault()
                void uploadFiles(files)
                return true
              },
            }),
          ]
        : []),
    ]

    const text = writing ? writing.text : tile!.draft.body
    view = new EditorView({
      state: EditorState.create({ doc: text, extensions }),
      parent: host!,
    })

    if (writing || workspace.tile === tile) view.focus()

    const cm = workspace.vimEnabled ? getCM(view) : null
    if (cm) {
      // Keep the mode line in step with the editor.
      const emitter = cm as unknown as {
        on?: (event: string, handler: (e: { mode: string }) => void) => void
      }
      emitter.on?.('vim-mode-change', (event) => {
        mode = event.mode
        if (tile) tile.vimMode = event.mode
      })

      // A fresh entry starts in insert mode, and so does an empty short text,
      // which is there to be written; anything else starts in normal mode,
      // the way opening a file in vim does.
      if (tile?.enterInsert || (writing && !text.trim())) {
        if (tile) tile.enterInsert = false
        Vim.handleKey(cm, 'i', 'workspace')
      }
    } else mode = 'insert'
  })

  /*
   * The keys reach whichever editor is in the focused tile: that one fills
   * the hooks, and the rest stand by until their tile is focused.
   */
  $effect(() => {
    if (!tile || workspace.tile !== tile) return
    hooks.focusEditor = focus
    hooks.insertText = insertAtCursor
    hooks.headings = editorHeadings
    // Only clear the hooks if no other editor has taken them already.
    return () => {
      if (hooks.focusEditor === focus) hooks.focusEditor = () => {}
      if (hooks.insertText === insertAtCursor) hooks.insertText = () => {}
      if (hooks.headings === editorHeadings) hooks.headings = () => ({ headings: [], current: 0 })
    }
  })

  onDestroy(() => {
    view?.destroy()
    view = undefined
    if (tile) tile.vimMode = 'normal'
  })
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="writing" class:framed={writing} class:fill onkeydowncapture={onkeydown}>
  <div class="editor" bind:this={host}></div>
  {#if writing}
    <div class="modeline label">
      <span class="mode" class:insert={mode !== 'normal'}>{mode}</span>
      {#if writing.dirty}<span class="dirty">+</span>{/if}
      <span class="hint">saves itself · <span class="kbd">:q</span> leave · <span class="kbd">:q!</span> drop what is unsaved</span>
    </div>
  {/if}
</div>

<style>
  .writing {
    height: 100%;
    display: flex;
    flex-direction: column;
  }
  .editor {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }

  /* A short text, written in place among other things. */
  .framed {
    height: auto;
    border: 1px solid var(--light-dim);
    border-radius: var(--radius-m);
    background: var(--base);
    overflow: hidden;
  }
  .framed .editor { flex: none; min-height: 10em; max-height: 50vh; overflow: auto; }
  .framed.fill { height: 100%; }
  .framed.fill .editor { flex: 1; max-height: none; }
  .modeline {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 4px 12px;
    border-top: 1px solid var(--rim);
    background: var(--hull);
    font-size: 10px;
  }
  .mode { padding: 0 8px; border-radius: var(--radius-s); background: var(--light-soft); color: var(--ink); }
  .mode.insert { background: var(--light); }
  .dirty { color: var(--light); }
  .hint { margin-left: auto; color: var(--muted); text-transform: none; letter-spacing: 0.04em; }
</style>
