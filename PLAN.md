# Findings, possible bugs, and what comes next

Written on 2026-10-21, after the seven slices of the feature plan had landed
(trash `8ffad63`, revisions `fc2be66`, links `a42f907`, capture tokens
`f629f5a`, diary `824f2bb`, recurring cards `6673298`, search in files
`909abf5`). That plan has fully landed, and git keeps it.

These came out of writing the wiki, taking the screenshots against a fresh
workspace, and probing the API on a throwaway server. Each entry says what
happens, where, and a way to fix it. None of them loses data outright. Several
can overwrite text, but a revision of it is kept.

## Bugs

### 1. A rename can overwrite a newer save without asking

`renameNode` (`web/src/lib/store.svelte.ts:1171`) renames by sending the whole
node: name, body and date, and no `base_updated_at`. That has two effects:

- For the document open in a tile, the body sent is the local draft. If the
  document was saved somewhere else since it was opened (another tab, the
  phone, the SFTP mount), the rename writes the draft over that save. `:w`
  would have refused and offered `:e!`; `:name` does not.
- For any other node, the body is fetched with `api.getNode` and sent back. A
  save that lands between the two requests is overwritten.

The server keeps a `force` revision in both cases (`nodes.rs:757`), so the
other text can be brought back with `gh`. Nothing says that it is needed.

**Fix:** send `base_updated_at` from the tile, or from the node just fetched,
and handle the 409 the way `save` does. Better still, see 2.

### 2. `PUT /api/nodes/{id}` without `body` empties the document

`UpdateInput.body` defaults to `""` (`src/routes/nodes.rs:161`). A script, or a
future client, that renames a document with `{"name": "…"}` alone erases its
text. A `force` revision is kept, so it can be restored, but the API invites
the mistake.

**Fix:** make `body` an `Option<String>` and leave the text alone when it is
absent. The same for `created_at`, which already is optional. Then
`renameNode` can send only the name, and 1 goes away for every node that is not
being edited.

### 3. A search made only of punctuation lists the spaces

`fts_query` (`nodes.rs:251`) turns `!!!` or `--` into no terms. `list` then
falls through to browsing with `parent = None` (`nodes.rs:514`), and answers
with the list of spaces, as if they were search results: on a throwaway
server, `GET /api/nodes?q=%21%21%21` answered with every space.

**Fix:** when `q` is present but yields no terms, answer an empty list.

### 4. The agenda opens once per browser day, not per workspace day

`openAgendaIfDue` (`store.svelte.ts:1311`) keys "seen today" on
`new Date().toDateString()`, the browser's local day. The server counts days in
`WORKSPACE_TIMEZONE`. A browser in another timezone than the setting (a laptop
on a trip, a browser set to UTC) opens the agenda at the wrong moment, or twice
on one workspace day.

**Fix:** key it on the server's local day, which the agenda answer could carry
next to `diary_today`.

## Rough edges

### 5. Two spaces can have the same name

`create_space` (`nodes.rs:458`) makes the slug unique (`work-2`) but keeps the
name, so the bar shows two pills that read alike, and `:space work` picks the
first. Folders and documents may share names on purpose; spaces are a short,
numbered list where that is only confusing.

**Fix:** refuse a space whose name is taken, case-insensitively, with the
reason.

### 6. Names have no length limit

A 5000-character name is accepted, and so is its slug, which ends up in every
`/n/…` address and in the SFTP mount, where most filesystems refuse names over
255 bytes.

**Fix:** cap names (255 characters, say) on the server, and cut slugs shorter
than that.

### 7. A hard-wrapped paragraph renders with its line breaks

`marked` runs with `breaks: true` (`web/src/lib/markdown.ts:6`), so a single
newline is a `<br>`. That suits text typed in the browser, but markdown that
comes from elsewhere is often wrapped at 80 columns: an import, a file saved
over the SFTP mount from vim with `textwidth`, a pasted README. Those read with
a break in the middle of every sentence, more visibly on a phone.

**Fix:** a `:set nobreaks` per document or per space, or treating a line break
as a space when the paragraph's lines are all close to the same width. At the
least, say so in the docs; the wiki's Writing page now does.

### 8. A fresh clone does not compile until the frontend is built

`rust-embed` needs `web/dist` to exist (`src/static_files.rs:10`), so
`cargo check`, `cargo test` and rust-analyzer fail on a fresh clone with a
derive error that does not say what to do. CI builds the frontend first.

**Fix:** `#[allow_missing = true]` on the embed, so a missing frontend is the
404 the handler already answers with ("frontend not built"), not a compile
error.

### 9. Search inside files: known limits

- `file_text::catch_up` scans every file node without a row on each upload,
  import and SFTP write. That is fast at a few thousand files and will not
  stay so.
- Results from documents and from files are ranked by two separate BM25
  indexes, so their order relative to each other is approximate.
- The excerpt does not mark the words that matched.
- PDFs and office documents are not read.

**Fix:** refresh only the node that changed, and keep the full scan for start
up. Mark the match in the snippet with a character the client draws as
highlight.

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

Suggested order: 2 and 1 together (one change, and the only findings that can
overwrite text), then 3, 8 and 5, which are small. Then the PWA, which the new
icon makes cheap.
