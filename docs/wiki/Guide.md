# The guide: a day with the workspace

This page follows one ordinary day: the agenda in the morning, documents, notes
and cards at work, a lecture in the afternoon, a thought on the way home, and
the diary in the evening. Each part of the workspace is explained where it first
comes up, with a link to the page that has the rest of it.

The keys are the desktop's. On a phone, every one of these has a button; see
[On a phone](#on-a-phone) at the end.

## Four things, two screens

Everything you keep is one of four things:

| | what it is for | where it lives |
| --- | --- | --- |
| **document** | the thorough record: a project, a runbook, lecture notes | the tree of a space, in folders |
| **card** | the short, actionable summary of work, with a due date and a checklist; it usually points at a document | the board of a space |
| **note** | a thought jotted down in passing, not sorted yet | the scratchpad, one for the whole workspace |
| **diary entry** | a document about one day | the diary space, by date |

A **file** — a PDF, an image, a log — is never on its own: it sits in a folder
beside the documents, or belongs to the one document it was put into.

They are shown on two screens. The **documents** screen has the list on the
left and the documents open in tiles beside it. The **plan** screen has the
board, the agenda and the calendar, and a card turned over. `S` goes from one to
the other, with every view on the plan, and the documents keep what was on them.
`b`, `t` and `C` open the plan on their own view alone, or add it beside what is
there; `q` closes one view at a time. The scratchpad comes down over either.

## Morning: what needs you

The first time you open the workspace each day, the **agenda** opens by itself
when anything is late or due. Any other time, press `t`. It is on the plan
screen, with the board and the calendar; `Ctrl-h`/`j`/`k`/`l` walk between
them.

![The agenda](../images/agenda.png)

It gathers the unfinished cards of every board into four groups:

- **late**: past their due date;
- **today**: due today;
- **this week**: due in the next seven days;
- **in hand**: in a list between the inbox and done, which means work has
  started on them.

Under the cards it says whether the diary has an entry for today, and lists
what you wrote on this day in earlier years.

Walk it with `j` and `k`. On a card:

- `Space` finishes it, where it stands;
- `D` gives it a due date: `fri`, `tm`, `+3d`, `10-31`;
- `H` and `L` move it to the list left or right on its board;
- `Enter` goes to the card on its own board, turned over in the tile beside.

`q` goes back to the documents. The bar keeps showing the `late` and `today`
counts while either is above zero. More in [Boards and the agenda](Boards-and-Agenda.md).

## At work: documents

Press `2` for the `work` space. A **space** is one area of your life, with its
own folders, its own board and its own search. Make more with
`:space new <name>`; `w` shows all of them at once, with what was last written in
each.

![Every space at a glance](../images/overview.png)

The list on the left shows the folder you are in. `j` and `k` move, `l` or
`Enter` opens what is under the cursor, `h` goes up a level. Folders come first,
then documents by name; only the diary lists them by day.

Open the project notes with `Enter`. To read a second document beside it, move
to it and press `L` instead: it opens in a **tile** of its own.

![Two documents in tiles](../images/tiles.png)

`h` and `l` move between the tiles and back to the list, and `Ctrl-h` and
`Ctrl-l` do the same from the editor. `q` closes a tile, and `f` makes the
focused one fill the screen. Each space remembers its tiles, so switching away
and back finds them as they were. More in
[Spaces and documents](Spaces-and-Documents.md).

### Writing

`i` edits the open document. The editor is CodeMirror with real vim keys:
`ciw`, `dd`, visual mode, macros, `.` and the rest. `Esc` leaves insert mode,
and `Esc` again or `:q` leaves the editor.

![The editor](../images/editor.png)

The document is markdown. Tables, code blocks, task lists and
math (`$x^2$`, or `$$…$$` on its own) are drawn when you read it. Link to
another document by its title, `[[Runbook]]`; typing `[[` offers the titles to
pick from. When the other document is renamed or moved, the link is rewritten
to follow it.

You do not need to save. Every keystroke goes to the browser's storage at
once, and to the server a couple of seconds after you stop typing. If the
network is gone, the draft waits in the browser and is sent when it comes back.
The same is true of a card's description and of a note: there is one editor,
and it works the same way everywhere. More in [Writing](Writing.md).

### Everything about a document

`gi` opens the document's **info sheet**: its headings, its earlier versions,
the documents that link to it, the cards about it, its files, and whether it is
shared. `Tab` goes from one part to the next.

![The cards about a document, in its info sheet](../images/info.png)

## At work: notes

Halfway through a paragraph a thought comes that belongs somewhere else.
`Ctrl-Q`, even in insert mode, brings the **scratchpad** down with the last
note open at its end. Write the thought, and `Ctrl-Q` again puts you back in
the document where you were, still writing. `:scratch <text>` takes a note from
the command line without the drawer.

`Q` brings the scratchpad down to sort it. Each note leaves in one of three
ways:

- `K` keeps it as a document, in the folder you are browsing;
- `C` makes it a card: the first line is the title, the rest the description;
- `A` puts it at the end of a document, picked by title.

![The scratchpad](../images/scratch.png)

The notes are found by `/` like everything else. More in
[Writing](Writing.md#the-scratchpad).

## At work: cards

In a meeting someone asks you to renew a certificate. Press `c`, type
`Renew the gateway certificate`, and `Enter`. The card is in this space's
inbox, and you have not left what you were reading. When that was a document,
the card is about it: it shows in the document's info sheet, and the
document's meta line says what is due. `Tab` on the same line makes a note
instead, and `Tab` again a timed line in today's diary entry.

Press `b` to see the **board**, on the plan screen. A digit there shows the
board of another space.

![The board](../images/board.png)

The first list is the inbox and the last is done; the ones between are work in
hand. `h`/`l` move between lists and `j`/`k` between cards. `H`/`L` move a
card across, `Space` finishes it, and `D` sets its due date. `:repeat weekly`
makes a card come back in the inbox each time it is finished.

`Enter` turns a card over to its labels, description and checklist, in a tile
beside, and `q` turns it back. Keep the description short: the long version
belongs in a document. `gf` opens the card's document, made the first time you
ask, among the documents; `S` comes back to the board.

The other way round works too. Reading a document, `gc` lists the cards about
it, and `:due`, `:label`, `Space`, `H` and `L` act on the one picked, without
going to the board.

### Finding things

`/` searches the space you are in, as you type, and `Tab` on the line takes
it to every space. The search reads the documents and the text files in the tree, then the cards
and the notes: a log file is found by a line inside it, and the result shows
the words around the match.

![A search that found a log line](../images/search.png)

`Ctrl-N`/`Ctrl-P` walk the results without leaving the line, `Enter` opens
one, and `n`/`N` step through them afterwards. When you know what it is called,
`Ctrl-P` goes straight there from anywhere: type part of the name, with a word
of the folder before it to narrow it, `proj ingest`. More in
[Files and search](Files-and-Search.md).

## Afternoon: a lecture

Press `3` for `uni`. The lecture notes have formulas in them, written as
LaTeX between dollar signs and drawn by KaTeX:

![Math in a document](../images/math.png)

The slides came as a PDF. Drop it onto the list, or `:upload`, and it is a
file in the folder, beside the notes about it. An image pasted into a document
while writing belongs to that document instead: it is in the document's info
sheet, `za` folds it open under the document in the list, and it moves and goes
to the trash with it.

`Enter` opens a PDF, an image, a video, a CSV table, a text or source file, or
a certificate in place. Anything else downloads with `:export`.

![A CSV file as a table](../images/file-table.png)

A scan of something private, like an ID or a contract, goes in with
`:upload seal` instead. That seals it in the **vault**, under a password that is
never stored. See [The vault](Vault.md).

## On the way home: capture

A thought arrives when no tab is open. `:token add phone` makes a token. With
it, an iPhone Shortcut, an Android HTTP Shortcut or a single `curl` puts a card
in an inbox, a note on the scratchpad, or a line in today's diary entry:

```sh
curl -H "Authorization: Bearer nwc_…" -H 'content-type: application/json' \
  -d '{"text":"call the bank"}' https://your-host/api/capture
```

The token can do this and nothing else. On Android the installed app needs no
token: share to it, and the text is a note on the scratchpad. See
[Capture](Capture.md) and [On a phone](#on-a-phone).

## Evening: the diary

Press `T` for today's entry. If there is none yet it is started, from the
diary's `templates/day` document when there is one, and opens ready to write.

`C` shows the diary by month, on the plan screen. A dot marks a day with an
entry; `h`/`l`, `j`/`k` and `H`/`L` move by day, week and month, and `Enter`
opens the day among the documents, or starts an entry for it. In an entry, `[d`
and `]d` step to the one before and after.

![The diary by month](../images/calendar.png)

If nothing was written in the diary by the evening and mail is set up, a
reminder arrives at `WORKSPACE_REMINDER_AT`. More in [The diary](Diary.md).

## When something goes wrong

- **Deleted the wrong thing?** `u` brings the last delete back. `:trash` lists
  everything deleted in the last 30 days.
- **Overwrote a paragraph?** `gh` lists the document's earlier versions as diffs
  against the text as it is now, and `Enter` brings one back.

  ![Earlier versions of a document](../images/history.png)

- **Saved in two places?** A document saved somewhere else since you opened it
  is not overwritten: `:w` says so, `:e!` takes the other version, and `:w!`
  writes yours anyway.

More in [Not losing things](Not-Losing-Things.md).

## On a phone

On a touch screen the workspace drops the modal keys, and under 768 pixels
wide the list and the open document take the whole screen in turn. A dock along
the bottom has a new document, a new folder, the board, today's entry, an
upload, the trash and a search. The `⋯` on a row, or a long press, has
everything that can be done to it: rename, move, share, seal, delete. The agenda
has a done button on each card.

| | |
| --- | --- |
| ![The tree on a phone](../images/mobile-tree.png) | ![A document on a phone](../images/mobile-document.png) |

`:set vim` turns the vim keys on for a phone with a keyboard.

Over HTTPS the browser offers to install the workspace as an app (Chrome's
menu: *Install app*, or *Add to Home screen*). Installed, it opens from its own
icon, and with no network it still opens, to say the server cannot be reached.
Back lets go of one thing at a time, as `Esc` does — a sheet, the scratchpad,
the editor, the document, then a folder up — and only at the root of a space
leaves the app.

On Android the installed app is in the share sheet. A link or some text shared
to it becomes a note on the scratchpad, the title on its first line; the drawer
comes down with it on top, ready to be kept, made a card or put at the end of a
document. A share made while logged out waits for the login.
