/*
 * What every editor on the page looks like: the frame of it, and the colours
 * of what is written in it, in any language a fence names.
 */
import { syntaxHighlighting } from '@codemirror/language'
import { languages } from '@codemirror/language-data'
import { EditorView } from '@codemirror/view'
import { highlighter } from './highlight'

/** The languages a fence can be written in, each fetched the first time one is. */
export { languages }

export const highlighting = syntaxHighlighting(highlighter)

/*
 * CodeMirror's base theme paints panels and the caret for a light editor
 * (`&light .cm-panels { color: black }`), and those selectors outrank
 * anything the app stylesheet can say. Theme rules do win, so the parts that
 * would otherwise render black on black live here rather than in app.css.
 * `dark: true` also stops the light defaults applying in the first place.
 */
export const editorTheme = EditorView.theme(
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
    '.cm-tooltip': {
      backgroundColor: 'var(--hull)',
      color: 'var(--fg)',
      border: '1px solid var(--rim)',
    },
    '.cm-tooltip-autocomplete > ul > li[aria-selected]': {
      backgroundColor: 'var(--surface-alt)',
      color: 'var(--light)',
    },
    '.cm-completionDetail': { color: 'var(--muted)', fontStyle: 'normal', marginLeft: '1ch' },
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
