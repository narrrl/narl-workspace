# Boards and the agenda

## Views

The board, the agenda, the [calendar](Diary.md#the-calendar) and the back of
a card are views: tiles like the documents' tiles, opened beside what is on
screen and walked the same way. `S` (or `:plan`) puts the board, the agenda
and the calendar up together, ahead of the documents with the board focused —
the board on the left, the agenda over the calendar on the right when nothing
else is open — and puts every view away again.

![The board, the agenda and the calendar](../images/board.png)

| key | what it does |
| --- | --- |
| `S` | the board, the agenda and the calendar up together, or every view away |
| `b`, `t`, `C` | to the board, the agenda or the calendar, opening its tile when it is not up |
| `gt` / `gT` | the next / previous tile |
| `Alt-h` / `Alt-j` / `Alt-k` / `Alt-l` | the tile that way; left of the leftmost is the list |
| `l` past the last list, `h` in the agenda, `j` past its end | on into the tile beside |
| `f` | the focused tile fills the screen; again or `Esc` to leave |
| `1` … `9` | switch space; with a view focused, the views come along |
| `q`, `Esc` | close the view's tile, back to the tile focused before it |

The keys go to the tile with focus, which wears the light border and its key
lit in its head; the other views keep their cursors, dimmed. A view is never
replaced by what is opened from it — a card's notes, a day of the calendar, an
entry of the agenda, a row of the list: that opens in a tile beside it, and
`q` on it comes back to the view. Each view is read again whenever it is gone
to.

Each space keeps its own tiles, views among them. Switching space with a view
focused takes the board, the agenda and the calendar along, the way the
special workspace used to stay up: a digit on the board shows that space's
board, with the agenda still beside it. Switching with a document or the list
focused leaves them in the space, and they are there again on coming back.

## The board

Every space except the diary has a board. `b`, the board button on the rail, or
`:board` shows it in a tile beside what is open, and `q` or `Esc` closes it.
It is the board of the space being browsed, or, in a space without one such as
the diary, the last board shown. The head of the board names every space that
has one, with the digit that goes there.

A new board has three lists: `backlog`, `doing` and `done`. The lists mean
something by where they stand, not by their names:

- the **first** list is the inbox, where new cards land;
- the **last** list is done. A card is finished exactly when it is in the last
  list: moving it there stamps it, moving it out reopens it;
- the lists between are **work in hand**, which the agenda shows.

`:list <name>` adds a list in front of the last one, `:name!` renames the list
under the cursor, and `:list!` deletes it with its cards. Done cards older than
a week fold away until `z` shows them.

| key | what it does |
| --- | --- |
| `h` / `l` | to the list on the left / right |
| `j` / `k` | down / up the cards |
| `J` / `K` | move the card down / up its list |
| `H` / `L` | move the card to the list on the left / right |
| `Space` | into the done list, or back out to the inbox |
| `o`, `c` | a new card on this list |
| `Enter` | turn the card over: labels, description, checklist |
| `gf` | the card's notes, without turning it over |
| `D` | due date |
| `R` | rename the card |
| `z` | show done cards older than a week |
| `F` | a letter over every card; type it to jump there |
| `x`, `dd` | delete the card |
| `q`, `Esc` | close the board's tile |

### The back of a card

`Enter` on a card turns it over: a tile beside the board with its labels, its
description and its checklist. It is the card under the board's cursor, so
walking the board leafs through the backs, and `D`, `R`, `H` and `L` act on it
as they do on the board.

| key | what it does |
| --- | --- |
| `j` / `k`, `gg` / `G` | down / up the checklist, its first / last step |
| `Space`, `x` | tick the step, or untick it |
| `o` | a new step below the cursor (`:task <text>`) |
| `r` | reword the step (`:task! <text>`) |
| `J` / `K` | move the step down / up |
| `dd` | drop the step |
| `e`, `i`, `Enter` | write the description |
| `a` | put a label on, or take it off (`:label <name>`) |
| `gf` | the card's notes, below |
| `q`, `Esc` | close the back, back to the board |

The description is markdown, written in the same editor as a document, vim
and all: `:w` writes it to the card, `:q` or `Esc` in normal mode leaves, and
`:q!` throws away what was not written. Leaving the tile while writing keeps
what is written. A fenced block in a language is
coloured the way nvim colours it, written and read.

Labels are lower case, and each name keeps its colour on every card and every
board, without anything to set: the colour comes from the name. `Tab` on
`:label` offers the ones the board already uses. The board shows a card's
labels over its title, and `☑ 2/5` and `≡` under it when it has a checklist and
a description.

A repeating card comes back with the same labels and the same checklist, every
step open again.

### Capturing a card

`c` in the tree opens `:card ` on the command line. The card goes
into the inbox of the space you are in, or of the first space with a board, and
you stay where you were. From outside the browser, see [Capture](Capture.md).

### Notes

`gf` on a card, or on its back, opens its notes: a document named after the card, made in the
root of the space the first time, in a tile beside the board. Closing it goes
back to the board, on the same card. `:card!` while reading a document makes a card that points at it.

The link is loose on purpose: deleting the document leaves the card, because the
work is still open when its notes are gone.

### Due dates

`D` on a card (or `:due <when>`) takes a date the way it is said:

| you type | it means |
| --- | --- |
| `today`, `td` | today |
| `tm`, `tomorrow` | tomorrow |
| `fri` | the next Friday, never today |
| `+3d`, `+2w` | in three days, in two weeks |
| `10-31` | the next 31 October |
| `2026-10-31` | that day |
| `-` | no due date |

### Cards that come back

`:repeat weekly`, or the ↻ button on the rail, makes a card come back once it
is done. The rules are `daily`, `weekly`, `monthly`, `yearly`, and any `+3d`,
`+2w`, `+6m` or `+1y`. `:repeat -` stops it.

Finishing such a card leaves a copy at the bottom of the inbox, with the same
title, notes and rule, due on the next date. The next date counts on from the
old due date, in `WORKSPACE_TIMEZONE`:

- a card due at noon stays due at noon across a change of the clocks;
- a card due on the 31st comes back on the last day of a shorter month, and on
  the 31st again where there is one;
- a card finished late skips the dates that have passed;
- a card without a due date counts from the moment it was finished.

The finished card gives up its rule, so reopening it and finishing it again
does not make a second copy. The board and the agenda show the rule as
`↻ weekly` beside the date.

## The agenda

`t` (or `:agenda`) gathers what needs you from every board at once, in a tile
of its own.

![The agenda](../images/agenda.png)

- **late**, **today** and **this week**: unfinished cards by due date;
- **in hand**: cards in a list between the inbox and done;
- whether the diary has an entry for today;
- **on this day**: diary entries from the same day in earlier years. A
  29 February is remembered on the 28th in the years that have none.

| key | what it does |
| --- | --- |
| `j` / `k` | down / up, on into the diary entries, then into the tile below |
| `h` | to the tile on the left |
| `Enter` | to the card on its own board, in the tile beside, or open the entry |
| `Space` | finish the card where it stands |
| `D` | due date |
| `H` / `L` | to the list left / right on its board |
| `:` | the command line with the agenda kept up, so `:repeat` acts on the card |
| `T` | today's diary entry |
| `q`, `Esc` | close the agenda's tile |

The agenda opens by itself once a day when something is late or due. "Today"
is the day in `WORKSPACE_TIMEZONE`, the same as for the mails, so a browser in
another timezone groups the cards the same way. The bar
shows the `late` and `today` counts whenever either is above zero. With mail
set up, cards due today are mailed in the morning too; see
[Backup and mail](Backup-and-Mail.md#mail).
