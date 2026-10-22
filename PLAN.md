# Findings, possible bugs, and what comes next

## The simplification, done

Written on 2026-10-22. The app had grown slice by slice to about 85 commands,
60 browse keys and 11 overlays, with the same idea modelled two or three times:
text in three editors, files in two places, cards that pointed at a document
the document could not see, four inboxes, two layout systems, and keys
documented by hand in seven places. A plan in eleven slices made the same
features out of fewer things, one place for each, and one key grammar. Every
one has landed:

| | slice | commit |
| --- | --- | --- |
| 0 | the audit's fixes: `Ctrl-P` keeps its query, `:today` by the server's day, card descriptions resolve links | `7f8a381` |
| 1 | one keymap table drives the dispatch, help, the footers, the rail and the wiki; a mode stack replaces the flags | `0eabcb2` |
| 2 | one `Sheet` component; the info sheet holds headings, versions, backlinks and sharing | `26b3c68` |
| 3 | the board, agenda, calendar and a card's back are tiles; the special workspace is gone | `5684614` |
| 4 | documents and cards linked both ways: `gc`, card verbs on the card in focus, cards in search | `b8b3641` |
| 5 | one editor for documents, descriptions and notes, all saving themselves | `ccc9102` |
| 6 | the scratchpad is the inbox, with three ways out and a search | `2233fa4` |
| 7 | a file belongs to a node; the media browser is gone | `d8e72c6` |
| 8 | `!` means force or everything only; merged commands; a settings sheet; two-level help | `7f8d625` |
| 9 | the store split into parts under one `Workspace` | `b110701` |
| 9b | fixes from daily use: `Ctrl-h/j/k/l` between tiles, `Ctrl-Q` from anywhere, the plan screen | `650fc40` |
| 10 | the docs: the guide around the four things, the README, new screenshots | this commit |

The model it leaves: a **document** is the thorough record, a **card** the
short actionable summary of work, a **note** a thought not sorted yet, and a
**diary entry** a document about a day. A file sits in a folder or belongs to
its document. The documents and the plan each have a screen, and the
scratchpad comes down over either.

### The navigation fixes

The plan's navigation fixes sat outside the numbered slices, and landed after
them, in one commit:

- The list is by name, folders first, everywhere but the diary, which stays a
  journal by day; a document's row there shows when it was last edited.
- `Ctrl-P` goes to anything by its name, in every space (`GET /api/jump`): the
  last word is looked for in the name, the ones before along the path. `/` is
  the one full-text search, and `Tab` on its line takes it to every space.
- A card's notes are made in the folder being browsed when that is in the
  card's space, and a `[[missing]]` link's document beside the one linking to
  it.
- `o` in a space with templates offers them on the line, `:new `; `Enter` on
  nothing is still a blank document.

## Fixed before, on 2026-10-21

After the seven slices of the feature plan (trash `8ffad63`, revisions
`fc2be66`, links `a42f907`, capture tokens `f629f5a`, diary `824f2bb`,
recurring cards `6673298`, search in files `909abf5`), writing the wiki, taking
the screenshots and probing the API turned these up:

| | finding | fix |
| --- | --- | --- |
| 1 | A rename sent the whole node without the version it had seen, so it could write a stale draft over a save made elsewhere. | `3a6ad94`: a rename sends the name alone, with the tile's `updated_at` when the document is open, and is refused like `:w` when it was saved since. |
| 2 | `PUT /api/nodes/{id}` without `body` emptied the document. | `3a6ad94`: every field changes only when it is sent. |
| 3 | A search of only punctuation answered with the list of spaces. | `a1f6f60`: it finds nothing. |
| 4 | The agenda grouped cards and opened itself by the browser's day, not `WORKSPACE_TIMEZONE`, and took a day to be 24 hours. | `0e0d215`: the agenda answer carries `today` with its bounds, from the same `day_bounds` the mails use. |
| 5 | Two spaces could share a name. | `a1f6f60`: refused, in any case or by slug, for a new space, a rename, and `mkdir` at the root of the mount. |
| 6 | Names had no length limit. | `a1f6f60`: at most 200 characters. |
| 7 | A hard-wrapped paragraph rendered with a break in every line. | `767dd07`: a paragraph that looks wrapped at a width has its soft breaks read as spaces; hand-broken lines and explicit breaks stay. |
| 8 | A fresh clone did not compile before the frontend was built. | `a59ba4a`: `allow_missing` on the embed. |
| 9 | Indexing a file walked every file node on each write, and the excerpt did not mark the match. | `c72b65f`: a write reads only the folder it wrote into, and the matched words are lit. |

### Still open, on purpose

- Results from documents and from files are ranked by two separate BM25
  indexes, so their order relative to each other is approximate. A shared
  rank would mean one index for both, which costs more than it gives for a
  list this short.
- PDFs and office documents are not read for search. That needs a parser
  crate, and waits until it is missed.

**Before the first wiki publish:** GitHub creates a wiki's repository only
when its first page is saved, so save any page once in the repository's Wiki
tab. `.github/workflows/wiki.yml` replaces it on its first run.

## Next, unplanned

The backlog, fitted to the model rather than given new surfaces:

- Install the app on a phone (a PWA: manifest plus a service worker for the
  app shell, CSP `worker-src 'self'`), with a share target that posts into
  capture as a note. The touch and desktop menus are already the same.
- Task lists: toggle `- [ ]` in the reader, and the open ones of a space as a
  section of the agenda rather than a `:tasks` command.
- A daily review, also as a section of the agenda: yesterday's entry, what was
  finished this week, and cards in `doing` for more than 14 days.
- Mail-in capture: IMAP polling of a plus-address onto the scratchpad.

Dropped: marks (`ma` … `mz`). The info sheet's headings and the tiles cover
what they were for, and `m` stays picking nodes up.

Suggested next: the PWA.
