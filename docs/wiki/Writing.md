# Writing

## The editor

`i` or `a` on an open document edits it. The editor is CodeMirror 6 with the
vim keymap: motions, operators, text objects, visual mode, registers, macros
and `.` all work, and `:w`, `:wq` and `:q!` do what they do in vim.

![The editor](../images/editor.png)

- `:name` on its own moves up into the title; `Enter`, `Esc` or `↓` goes back
  down into the text. `:name <text>` renames without leaving the text.
- `]]`, `[[` and `gO` walk the headings, the same as in the reader.
- `gf` follows the link under the cursor, saving first, and `gx` opens it in
  a new browser tab.
- `Ctrl-S` saves from anywhere, and `?` shows the help.
- Yanks and deletes go to the system clipboard too. `:set noclipboard` keeps
  them in vim's registers only.
- `:set novim` turns the vim keys off for a plain editor; `:set vim` turns them
  back on. On a touch screen they start off.

Drag, drop or paste an image, a video or a sound file into the text and it is
uploaded and embedded where the cursor is. `:upload` picks files to embed, and
`:media` lists every upload: `Enter` embeds the one under the cursor, `y`
copies its address.

## Saving

There is nothing to remember. Each change goes to the browser's storage as it is
typed, and to the server a couple of seconds after typing stops.

- When the server cannot be reached, the draft stays in the browser and the
  save is tried again, less often the longer it fails, and at once when the
  browser says it is back online.
- When a session runs out in the middle of a sentence, the login appears, and
  logging in again carries on where it stopped, draft and all.
- When a tab dies in the middle of a document, opening the document again
  restores the draft and says so. `:e!` throws the draft away.
- When the document was saved somewhere else since you opened it (another tab,
  the phone, the SFTP mount), `:w` says so instead of overwriting it. `:e!`
  takes the other version, and `:w!` writes yours anyway.

Each save keeps the text it replaces, so nothing written is lost. See
[Not losing things](Not-Losing-Things.md#history).

## Markdown

Documents are GitHub-flavoured markdown: headings, lists, task lists, tables,
quotes, code blocks and links. A single line break is kept as a line break.

Math is set by KaTeX:

- `$x^2$` inline;
- `$$ … $$` on lines of its own, or a ` ```math ` block, for a display formula.

![Math in a document](../images/math.png)

## Links

A document links to another in two ways:

- by its address: `[the plan](/n/work/notes/plan)`;
- by its title: `[[Plan]]`, or `[[Plan|the plan]]` with a label of its own.

A title is looked for in the space the link is written in first, then in every
other. It matches a name in any case, or the slug of that name. Typing `[[` in
the editor offers the titles of documents as the search finds them. A title
that no document has yet is drawn dimmed, and following it offers to write that
document.

Links keep working when what they point at moves. Renaming a document, or a
folder above it, or moving either, rewrites the links to it in every document
that has one, in the same save: the path to the new one, a `[[title]]` to the
new name, the label left as it was. A link to something in the trash follows it
when it is restored; once it is deleted for good the link text stays and leads
nowhere.

`gb` or `:backlinks` lists the documents that link to the open one, each with
the line it does so on.

A link written inside a card, or a bare address in prose rather than a markdown
link, is not counted or rewritten.

## Templates

Any space can have a folder called `templates` at its root. Every document in
it is a template.

- `:new meeting` starts a document from `templates/meeting`. `Tab` completes the
  template names.
- `:new meeting 2026-10-20 Budget` dates it and names it as well.

In a template these placeholders are filled in, on the clock of
`WORKSPACE_TIMEZONE` at the date the document is for:

| placeholder | becomes |
| --- | --- |
| `{{date}}` | `2026-10-20` |
| `{{weekday}}` | `Tuesday` |
| `{{time}}` | `14:05` |
| `{{title}}` | the new document's name |

A new diary entry starts from the diary's `templates/day` when there is one.
See [The diary](Diary.md).
