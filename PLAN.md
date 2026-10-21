# Findings, possible bugs, and what comes next

Written on 2026-10-21, after the seven slices of the feature plan had landed
(trash `8ffad63`, revisions `fc2be66`, links `a42f907`, capture tokens
`f629f5a`, diary `824f2bb`, recurring cards `6673298`, search in files
`909abf5`). That plan has fully landed, and git keeps it.

These came out of writing the wiki, taking the screenshots against a fresh
workspace, and probing the API on a throwaway server. All of them are fixed
now, except two limits of search noted at the end of the list.

## Fixed

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

## Docs and tooling, done in this pass

- An icon, `~/` and a block cursor, as the favicon, the touch icon, and the
  mark in the bar, the login and the share page.
- A README that opens with the banner, the badges, screenshots and the
  features grouped by what they are for.
- A wiki in `docs/wiki`: a getting-started page, a guide that walks through a
  day, a page per part of the app, and three reference pages generated from the
  code (`scripts/wiki.ts`).
- Screenshots in `docs/images`, taken by `scripts/screenshots.ts` against a
  throwaway workspace, so they can be taken again when the UI changes.
- CI: `.github/workflows/ci.yml` runs the Rust checks, svelte-check and the
  wiki check on every push. `.github/workflows/wiki.yml` publishes the wiki on
  every push to `main`.

**Before the first wiki publish:** GitHub creates a wiki's repository only
when its first page is saved, so save any page once in the repository's Wiki
tab. The workflow replaces it on its first run.

## Next, unplanned

Carried over from the last plan, still not started:

- Install the app on a phone (a PWA: manifest plus a service worker for the
  app shell, CSP `worker-src 'self'`), with a share target that posts into
  capture. The new icon is ready for the manifest.
- Task lists: toggle `- [ ]` in the reader, and `:tasks` gathers the open ones
  in a space.
- A daily review in the agenda: yesterday's entry, what was finished this week,
  and cards in `doing` for more than 14 days.
- Marks: `ma` … `mz` per document, `'a` to jump. This clashes with `m` for
  picking nodes up, so choose the keys first.
- Mail-in capture: IMAP polling of a plus-address into the inbox.

Suggested next: the PWA, which the new icon makes cheap.
