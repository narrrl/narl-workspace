/*
 * The colours of code, as classes — `tok-keyword`, `tok-string` — painted in
 * app.css from nvim's yorha.lua. The editor (editorTheme.ts) and a rendered
 * code block (code.svelte.ts) are both given these, so a fence reads the same
 * written and read. Only lezer here: the reader loads it with the app, and
 * CodeMirror comes only with an editor.
 */
import { tagHighlighter, tags as t } from '@lezer/highlight'

/** Lezer's tags onto yorha.lua's groups. The most specific tag that matches wins. */
export const highlighter = tagHighlighter([
  { tag: t.comment, class: 'tok-comment' },
  { tag: [t.keyword, t.labelName], class: 'tok-keyword' },
  { tag: [t.string, t.character, t.attributeValue], class: 'tok-string' },
  { tag: [t.escape, t.special(t.string)], class: 'tok-escape' },
  { tag: t.regexp, class: 'tok-regexp' },
  { tag: [t.number, t.bool, t.null, t.unit], class: 'tok-number' },
  { tag: [t.atom, t.constant(t.variableName), t.constant(t.propertyName)], class: 'tok-constant' },
  { tag: [t.self, t.standard(t.variableName)], class: 'tok-builtin' },
  { tag: t.variableName, class: 'tok-variable' },
  { tag: [t.propertyName, t.definition(t.propertyName)], class: 'tok-property' },
  {
    tag: [t.function(t.variableName), t.function(t.propertyName), t.function(t.definition(t.variableName))],
    class: 'tok-function',
  },
  { tag: [t.typeName, t.className], class: 'tok-type' },
  { tag: t.standard(t.typeName), class: 'tok-type-builtin' },
  { tag: t.namespace, class: 'tok-namespace' },
  { tag: [t.macroName, t.meta, t.annotation], class: 'tok-macro' },
  { tag: t.tagName, class: 'tok-tag' },
  { tag: t.attributeName, class: 'tok-attribute' },
  { tag: t.operator, class: 'tok-operator' },
  { tag: [t.punctuation, t.angleBracket], class: 'tok-punctuation' },
  { tag: t.invalid, class: 'tok-invalid' },
  { tag: t.inserted, class: 'tok-inserted' },
  { tag: t.deleted, class: 'tok-deleted' },
  { tag: t.changed, class: 'tok-changed' },

  { tag: [t.heading, t.heading1], class: 'tok-heading1' },
  { tag: t.heading2, class: 'tok-heading2' },
  { tag: t.heading3, class: 'tok-heading3' },
  { tag: t.heading4, class: 'tok-heading4' },
  { tag: t.heading5, class: 'tok-heading5' },
  { tag: t.heading6, class: 'tok-heading6' },
  { tag: t.strong, class: 'tok-strong' },
  { tag: t.emphasis, class: 'tok-emphasis' },
  { tag: t.strikethrough, class: 'tok-strikethrough' },
  { tag: t.link, class: 'tok-link' },
  { tag: t.url, class: 'tok-url' },
  { tag: t.monospace, class: 'tok-raw' },
  { tag: t.quote, class: 'tok-quote' },
  { tag: t.list, class: 'tok-list' },
  { tag: t.processingInstruction, class: 'tok-mark' },
  { tag: t.contentSeparator, class: 'tok-rule' },
])
