# The diary

The space with the diary role is a journal. Each document in it is an entry,
filed under the day it is about.

- `T` (or `:today`) opens today's entry, and starts it when there is none.
- `[d` and `]d` step to the entry before or after the one open, by date.
- `:date 2026-10-01` files the open entry under another day.
- `:new 2026-10-01` starts an entry for a day that has passed.

Days are counted in `WORKSPACE_TIMEZONE`, the same as the evening reminder, so
an entry written after midnight on a trip still lands on the right day.

## The calendar

`C` (or `:cal`) shows the diary a month at a time, in the
[special workspace](Boards-and-Agenda.md#the-special-workspace) under the
agenda.

![The diary by month](../images/calendar.png)

| key | what it does |
| --- | --- |
| `h` / `l` | a day back / forward |
| `j` / `k` | a week forward / back |
| `H` / `L` | a month back / forward |
| `.` | today |
| `Enter` | open the day's entry, or start one dated that day |
| `C`, `Esc` | put the special workspace away |

A dot marks a day with an entry. A number says how many when there are more
than one.

## The day template

When the diary has a `templates` folder with a document called `day` in it,
every new entry starts from it: from `T`, `o`, the calendar, or a
[capture](Capture.md) to the diary. For example:

```markdown
## {{weekday}}

### What happened

### What I am thankful for
```

See [Writing](Writing.md#templates) for the placeholders. The templates are not
entries: they are not on the calendar and do not count as having written today.

## On this day

The [agenda](Boards-and-Agenda.md#the-agenda) ends with what you wrote on the
same day in earlier years. `j` walks into them and `Enter` opens one.

## The reminder

With mail set up, an evening reminder arrives at `WORKSPACE_REMINDER_AT`
(20:00 by default) when nothing was written in the diary that day. See
[Backup and mail](Backup-and-Mail.md#mail).
