<script lang="ts" module>
  /* The vim register controller is global, so it is wrapped once per page. */
  let clipboardMirrored = false
</script>

<script lang="ts">
  import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands'
  import { markdown, markdownLanguage } from '@codemirror/lang-markdown'
  import { HighlightStyle, indentOnInput, syntaxHighlighting } from '@codemirror/language'
  import { EditorState } from '@codemirror/state'
  import {
    EditorView,
    drawSelection,
    dropCursor,
    highlightActiveLine,
    keymap,
    placeholder,
  } from '@codemirror/view'
  import { tags } from '@lezer/highlight'
  import { Vim, getCM, vim } from '@replit/codemirror-vim'
  import { onDestroy, onMount } from 'svelte'
  import { commands, followLink, hooks, runCommand } from './commands'
  import { embedSnippet } from './markdown'
  import { linkAt, sourceHeadings } from './util'
  import { workspace } from './store.svelte'

  let host = $state<HTMLDivElement>()
  let view: EditorView | undefined
  const focus = () => view?.focus()

  const markdownHighlight = HighlightStyle.define([
    { tag: tags.heading, color: 'var(--light)', fontWeight: '600' },
    { tag: tags.strong, color: 'var(--light)', fontWeight: '700' },
    { tag: tags.emphasis, color: 'var(--subtle)', fontStyle: 'italic' },
    { tag: tags.link, color: 'var(--light)', textDecoration: 'underline', textDecorationColor: 'var(--overlay)' },
    { tag: tags.url, color: 'var(--light-dim)' },
    { tag: tags.monospace, color: 'var(--light-soft)', backgroundColor: 'var(--surface)' },
    { tag: tags.quote, color: 'var(--subtle)', fontStyle: 'italic' },
    { tag: tags.list, color: 'var(--light-dim)' },
    { tag: tags.contentSeparator, color: 'var(--muted)' },
    { tag: tags.processingInstruction, color: 'var(--muted)' },
  ])

  /*
   * CodeMirror's base theme paints panels and the caret for a light editor
   * (`&light .cm-panels { color: black }`), and those selectors outrank
   * anything the app stylesheet can say. Theme rules do win, so the parts that
   * would otherwise render black on black live here rather than in app.css.
   * `dark: true` also stops the light defaults applying in the first place.
   */
  const editorTheme = EditorView.theme(
    {
      '&': { color: 'var(--fg)', backgroundColor: 'transparent', height: '100%' },
      '.cm-content': { padding: '12px 28px', maxWidth: '90ch' },
      '.cm-line': { padding: '0' },
      '.cm-cursor, .cm-dropCursor': { borderLeft: '2px solid var(--light)' },
      '.cm-panels': {
        backgroundColor: 'var(--hull)',
        color: 'var(--fg)',
        borderColor: 'var(--rim)',
      },
      '.cm-panels-bottom': { borderTop: '1px solid var(--rim)' },
      /*
       * The `:` prompt is a bare text node in the panel; the input follows it.
       * The input is an inline-block whose baseline sits below the text node's,
       * so the two are laid out as flex items instead of aligned by baseline.
       */
      '.cm-vim-panel': {
        padding: '4px 12px',
        color: 'var(--light)',
        display: 'flex',
        alignItems: 'center',
      },
      '.cm-vim-panel input': {
        fontFamily: 'inherit',
        fontSize: 'inherit',
        lineHeight: 'inherit',
        color: 'var(--light)',
        caretColor: 'var(--light)',
        flex: '1',
      },
    },
    { dark: true },
  )

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
    if (clipboardMirrored) return
    const registers = Vim.getRegisterController?.()
    if (!registers) return
    clipboardMirrored = true
    const push = registers.pushText.bind(registers)
    registers.pushText = (name, operator, text, linewise, blockwise) => {
      push(name, operator, text, linewise, blockwise)
      if (!name && text && workspace.clipboard) void workspace.copy(text)
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
    const headings = found.map(({ text, level, line }) => ({ text, level, go: () => gotoLine(line) }))
    return { headings, current }
  }

  function gotoLine(number: number) {
    if (!view) return
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
    const step = (direction: 1 | -1) => {
      if (!view) return
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
      gotoLine(target ?? (direction > 0 ? view.state.doc.lines : 1))
    }
    Vim.defineAction('workspaceNextHeading', () => step(1))
    Vim.defineAction('workspacePrevHeading', () => step(-1))
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
    const follow = (tab: boolean) => {
      if (!view) return
      const head = view.state.selection.main.head
      const line = view.state.doc.lineAt(head)
      const href = linkAt(line.text, head - line.from)
      if (!href) return workspace.say('no link under the cursor', 'error')
      followLink(href, tab)
    }
    Vim.defineAction('workspaceFollowLink', () => follow(false))
    Vim.defineAction('workspaceOpenLink', () => follow(true))
    Vim.mapCommand('gf', 'action', 'workspaceFollowLink', {}, { context: 'normal' })
    Vim.mapCommand('gx', 'action', 'workspaceOpenLink', {}, { context: 'normal' })
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
      const uploaded = await workspace.upload(files)
      insertAtCursor(uploaded.map(embedSnippet).join('\n\n'))
    })
  }

  onMount(() => {
    bridgeExCommands()
    bindHelpKey()
    bindHeadingKeys()
    bindLinkKeys()
    mirrorUnnamedRegisterToClipboard()

    const extensions = [
      ...(workspace.vimEnabled ? [vim({ status: false })] : []),
      history(),
      drawSelection(),
      dropCursor(),
      indentOnInput(),
      highlightActiveLine(),
      EditorState.allowMultipleSelections.of(true),
      markdown({ base: markdownLanguage, codeLanguages: [] }),
      syntaxHighlighting(markdownHighlight),
      EditorView.lineWrapping,
      placeholder('write…'),
      editorTheme,
      keymap.of([
        {
          key: 'Mod-s',
          preventDefault: true,
          run: () => (void workspace.guard(() => workspace.save()), true),
        },
        ...defaultKeymap,
        ...historyKeymap,
        indentWithTab,
      ]),
      EditorView.updateListener.of((update) => {
        if (update.docChanged) {
          workspace.draft.body = update.state.doc.toString()
          workspace.touch()
        }
      }),
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

    view = new EditorView({
      state: EditorState.create({ doc: workspace.draft.body, extensions }),
      parent: host!,
    })

    hooks.focusEditor = focus
    hooks.insertText = insertAtCursor
    hooks.headings = editorHeadings
    view.focus()

    const cm = workspace.vimEnabled ? getCM(view) : null
    if (cm) {
      // Keep the status line's mode indicator in step with the editor.
      const emitter = cm as unknown as {
        on?: (event: string, handler: (e: { mode: string }) => void) => void
      }
      emitter.on?.('vim-mode-change', (event) => {
        workspace.vimMode = event.mode
      })

      // A fresh entry starts in insert mode; an existing one starts in normal
      // mode, the way opening a file in vim does.
      if (workspace.enterInsert) {
        workspace.enterInsert = false
        Vim.handleKey(cm, 'i', 'workspace')
      }
    }
  })

  onDestroy(() => {
    view?.destroy()
    view = undefined
    // Only clear the hooks if no newer editor has taken them already.
    if (hooks.focusEditor === focus) {
      hooks.focusEditor = () => {}
      hooks.insertText = () => {}
      if (hooks.headings === editorHeadings) hooks.headings = () => ({ headings: [], current: 0 })
      workspace.vimMode = 'normal'
    }
  })
</script>

<div class="editor" bind:this={host}></div>

<style>
  .editor {
    height: 100%;
    overflow: hidden;
  }
</style>
