<script lang="ts">
  /*
   * A card's description, written in place on its sheet: the same editor as
   * a document's — vim, markdown, the colours — over `workspace.description`
   * instead of a tile. `:w` writes it to the card, `:q` leaves, and Esc in
   * normal mode leaves too when nothing is unsaved, since there is no tree
   * underneath to go back to.
   */
  import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands'
  import { markdown, markdownLanguage } from '@codemirror/lang-markdown'
  import { indentOnInput } from '@codemirror/language'
  import { EditorState } from '@codemirror/state'
  import { drawSelection, EditorView, highlightActiveLine, keymap, placeholder } from '@codemirror/view'
  import { getCM, Vim, vim } from '@replit/codemirror-vim'
  import { onDestroy, onMount } from 'svelte'
  import { bindVim } from './Editor.svelte'
  import { editorTheme, highlighting, languages } from './editorTheme'
  import { workspace } from './store.svelte'

  let host = $state<HTMLDivElement>()
  let view: EditorView | undefined
  let mode = $state('normal')

  /* Esc, before vim sees it: in normal mode it is the way out of the editor. */
  function onkeydown(event: KeyboardEvent) {
    if (event.key !== 'Escape' || (workspace.vimEnabled && mode !== 'normal')) return
    event.preventDefault()
    event.stopPropagation()
    workspace.stopDescribing()
  }

  onMount(() => {
    bindVim()
    view = new EditorView({
      parent: host!,
      state: EditorState.create({
        doc: workspace.description,
        extensions: [
          ...(workspace.vimEnabled ? [vim({ status: false })] : []),
          history(),
          drawSelection(),
          indentOnInput(),
          highlightActiveLine(),
          markdown({ base: markdownLanguage, codeLanguages: languages }),
          highlighting,
          EditorView.lineWrapping,
          placeholder('what this card is about…'),
          editorTheme,
          EditorView.theme({ '.cm-content': { padding: '10px 14px', maxWidth: 'none' } }),
          keymap.of([
            {
              key: 'Mod-s',
              preventDefault: true,
              run: () => (void workspace.guard(() => workspace.saveDescription()), true),
            },
            ...defaultKeymap,
            ...historyKeymap,
            indentWithTab,
          ]),
          EditorView.updateListener.of((update) => {
            if (!update.docChanged) return
            workspace.description = update.state.doc.toString()
            workspace.descriptionDirty = true
          }),
        ],
      }),
    })
    view.focus()

    const cm = workspace.vimEnabled ? getCM(view) : null
    if (cm) {
      const emitter = cm as unknown as { on?: (event: string, handler: (e: { mode: string }) => void) => void }
      emitter.on?.('vim-mode-change', (event) => (mode = event.mode))
      // An empty description is there to be written, so it starts in insert mode.
      if (!workspace.description.trim()) Vim.handleKey(cm, 'i', 'workspace')
    } else mode = 'insert'
  })

  onDestroy(() => {
    view?.destroy()
    view = undefined
  })
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="writing" onkeydowncapture={onkeydown}>
  <div class="editor" bind:this={host}></div>
  <div class="modeline label">
    <span class="mode" class:insert={mode !== 'normal'}>{mode}</span>
    {#if workspace.descriptionDirty}<span class="dirty">+</span>{/if}
    <span class="hint"><span class="kbd">:w</span> write · <span class="kbd">:q</span> leave · <span class="kbd">:q!</span> discard</span>
  </div>
</div>

<style>
  .writing {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--light-dim);
    border-radius: var(--radius-m);
    background: var(--base);
    overflow: hidden;
  }
  .editor { min-height: 10em; max-height: 50vh; overflow: auto; }
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
