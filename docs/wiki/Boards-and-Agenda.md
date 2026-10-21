# Boards and the agenda

## The board

Every space except the diary has a board. `b`, the board button on the rail, or
`:board` swaps the tree for it, and `q` swaps back.

![The board of a space](../images/board.png)

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
| `Enter` | the card's notes |
| `D` | due date |
| `R` | rename the card |
| `z` | show done cards older than a week |
| `F` | a letter over every card; type it to jump there |
| `x`, `dd` | delete the card |
| `q`, `Esc` | back to the tree |

### Capturing a card

`c` anywhere outside the board opens `:card ` on the command line. The card goes
into the inbox of the space you are in, or of the first space with a board, and
you stay where you were. From outside the browser, see [Capture](Capture.md).

### Notes

`Enter` on a card opens its notes: a document named after the card, made in the
root of the space the first time. Closing it goes back to the board, on the
same card. `:card!` while reading a document makes a card that points at it.

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

`t` (or `:agenda`) gathers what needs you from every board at once.

![The agenda](../images/agenda.png)

- **late**, **today** and **this week**: unfinished cards by due date;
- **in hand**: cards in a list between the inbox and done;
- whether the diary has an entry for today;
- **on this day**: diary entries from the same day in earlier years. A
  29 February is remembered on the 28th in the years that have none.

| key | what it does |
| --- | --- |
| `j` / `k` | down / up, on into the diary entries |
| `Enter` | to the card on its own board, or open the entry |
| `Space` | finish the card where it stands |
| `D` | due date |
| `H` / `L` | to the list left / right on its board |
| `:` | the command line with the agenda kept up, so `:repeat` acts on the card |
| `T` | today's diary entry |
| `t`, `q`, `Esc` | close |

The agenda opens by itself once a day when something is late or due. "Today"
is the day in `WORKSPACE_TIMEZONE`, the same as for the mails, so a browser in
another timezone groups the cards the same way. The bar
shows the `late` and `today` counts whenever either is above zero. With mail
set up, cards due today are mailed in the morning too; see
[Backup and mail](Backup-and-Mail.md#mail).
