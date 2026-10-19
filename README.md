# ~/workspace

A private, terminal-themed workspace that runs in the browser: a diary, the
notes for work and for uni, and a board per space. One user, one SQLite file,
one binary. It was called `narl-diary` until it grew everything but the diary.

- **Rust + axum** backend, **SQLite** storage, **Svelte 5** frontend.
- Spaces (`diary`, `work`, `uni`, …) with folders nested to any depth inside them.
- Markdown documents with full-text search across a whole space (SQLite FTS5).
- Import a folder — or a zip of one — as a tree of documents, links and all.
- A kanban board per space, so what is open at work is not mixed up with uni.
- Drag, drop or paste images, video and audio straight into an entry.
- Files of any kind — PDFs, archives, recordings — live in the tree beside the
  notes about them, streamed to disk however large.
- Share any document behind an unguessable link — revocable at any time.
- Backs itself up to Proton Drive, end-to-end encrypted, as a registered device.
- Mails you — a reminder when the diary is empty, due cards, a weekly digest.
- Real vim keybindings on the desktop (CodeMirror 6 + vim mode), plain taps on
  mobile.
- The whole frontend is embedded in the release binary, so deploying is copying
  one file.

## Quick start

```sh
cp .env.example .env      # set WORKSPACE_USER, WORKSPACE_PASSWORD and WORKSPACE_SECRET
make build                # builds the frontend, then the release binary
./target/release/narl-workspace
```

Then open <http://127.0.0.1:4242>.

Every setting is read as `WORKSPACE_<NAME>`, with the older `DIARY_<NAME>`
spelling still accepted behind it, so a server configured before the rename
keeps running untouched.

`WORKSPACE_SECRET` signs the session cookie; generate one with
`openssl rand -base64 32`. Changing it logs every device out. The database and
uploads live under `WORKSPACE_DATA_DIR` (`./data` by default) — that directory is
the entire backup, and it is exactly what the Proton Drive mirror copies.

### Development

```sh
make dev-api   # cargo run, serving the API on :4242
make dev-web   # vite dev server on :4243, proxying /api to :4242
```

In debug builds the binary reads `web/dist` from disk, so `cargo run` alone also
works once the frontend has been built at least once.

## Keys

The desktop UI is modal, and a key acts on whatever has focus: in the list
that is the row under the cursor, even while another document is open beside
it; in the document pane it is that document. In the browse pane:

| key | action |
| --- | --- |
| `j` / `k` | move down / up |
| `gg` / `G` | first / last entry |
| `Enter`, `l` | open the highlighted entry |
| `o` | a new document here, straight into insert mode |
| `O` | a new folder here |
| `i`, `a` | edit the open entry |
| `R` | rename what is highlighted — document, folder or space |
| `b` | the board of this space |
| `t` | the agenda: what is late, due this week, or in hand, on every board |
| `T` | today's diary entry, started if there is none |
| `c` | capture a card into this space's inbox without leaving where you are |
| `Esc`, `q` | back out to the document |
| `h` | up one level in the tree |
| `/` | full-text search in this space (`n` clears it) |
| `Ctrl-P` | full-text search across every space; `Enter` goes to the match |
| `s` / `y` | toggle sharing / mint a new share link |
| `x`, `dd` | delete the entry under the cursor (asks `y`/`N`) |
| `:` | command line |
| `Ctrl-S` | save from anywhere |
| `?` | help |

Inside the editor the full vim keymap is live — `w`/`b`, `dd`, `ciw`, visual
mode, macros, `:w` to write. `:help` lists every ex command; the useful ones are
`:w`, `:wq`, `:q!`, `:new [yyyy-mm-dd]`, `:date`, `:name`, `:share`, `:link`,
`:upload`, `:media`, `:search`, `:backup` and
`:set novim`. The tree has its own: `:space [name]` switches (`:space! <name>`
makes one, `:rmspace [name]` deletes one and everything in it), `:mkdir <name>`
makes a folder, `:cd <name|..|/>` walks it and `:mv <folder|..>` moves what is
selected, and `:import` uploads a folder into the one being browsed
(`:import!` takes a `.zip` instead). `:upload` outside the editor puts files
into the folder being browsed, and so does dropping them on the list. `:name <text>` renames whatever is under
the cursor — a document, a folder, or a card on a board — and `:name!` renames
the space itself. The board has
`:board`, `:card <title>` (`:card!` links the open document to it),
`:due <when|->`, `:done` and `:list <name>` (`:list!` deletes one).

On a touch device modal editing is turned off and the same actions are buttons.

## Boards

`b`, the `board` button above the list, or `:board` swaps the tree for the board
of the space being browsed, and swaps it back. Every space has one except the
diary — a journal has no backlog — and `:set board` / `:set noboard` changes
that for the space you are in (the `+board` button above the list does the
same). A new board starts with `backlog`, `doing` and `done`, and the columns mean
something by where they stand rather than by their names: the first is the
inbox new cards land in, the last is done. A card is finished exactly when it
sits in the last column — moving it there stamps it, moving it out reopens it —
so a tick and a column can never disagree. `:list <name>` adds a column in front
of the last one, `:name!` on the board renames the column under the cursor, and
done cards older than a week fold away until `z` shows them.

`c` captures from anywhere: `:card <title>` outside the board puts the card in
this space's inbox (or the first space that has a board) and leaves you where
you were. On the board, `o` or `c` puts a card on the column under the cursor.

| key | action |
| --- | --- |
| `h` / `l` | to the list on the left / right |
| `j` / `k` | down / up the cards of that list |
| `J` / `K` | move the card down / up its list |
| `H` / `L` | move the card to the list on the left / right |
| `Enter` | the card's notes: its document, made on first use; `q` comes back |
| `Space` | into the done column, or back out to the inbox |
| `o`, `c` | a new card on this list |
| `D` | due date — `fri`, `tm`, `+3d`, `+2w`, `10-31`, `2026-10-31` |
| `R` | rename the card, or the column when it has none |
| `z` | show done cards older than a week |
| `x`, `dd` | delete the card |
| `q`, `Esc` | back to the tree |

A card can point at a document: `:card!` while reading one makes a card for it,
and `Enter` on that card opens it again. `Enter` on a card without one makes a
document named after the card in the space's root and opens it to write in, and
closing that document returns to the board on the same card. The link is deliberately loose —
deleting the document leaves the card standing, because the work is still open
once its notes are gone.

## The agenda

`t` (or `:agenda`) gathers every unfinished card from every board that is late,
due within the week, or sitting in a column between the inbox and done — the
work already in hand — and says whether the diary has an entry for today. `j`/`k`
walk it, `Enter` goes to the card on its own board, `Space` finishes it where
it stands, and `T` writes today's entry. The bar shows `late` and `today` counts
whenever either is above zero, and the agenda opens by itself once a day when
something is late or due, so nothing has to be remembered to be looked at.

## Not losing things

Every change is written to `localStorage` as it is typed and to the server a
couple of seconds after typing stops, so `:w` is a habit rather than a
necessity. A document made with `o` and left with neither a name nor a word in
it is removed again on the way out, so abandoned starts do not pile up. If a tab dies mid-document, opening it again restores the draft and
says so — `:e!` throws it away, the way vim handles a swap file.

`:export` downloads the open document as markdown. `:export!` downloads the
whole workspace as a zip: the tree as real directories, one markdown file per
document, every file in the tree where it sits and every other embedded file in
`media/`, under their real names, and relative links between them, so the
archive reads in any markdown viewer without this application.

## Files

A file is a node like a document, only with bytes instead of text: it sits in
a folder, shows its size in the list, and `Enter` opens it — images, video,
audio and plain text are shown in place, anything else is a download. `:name`
renames it, and the download is called whatever it is called now; `:mv`, `m`
and `p` move it; `x` deletes it. `:upload` picks files for the folder being
browsed, or drop them onto the list. Each upload is written to disk as it
arrives, never held in memory, so the only ceiling is `WORKSPACE_MAX_UPLOAD_MB`
(1024 by default) — which is per request, so a proxy in front needs a body
limit at least that large too.

The SFTP mount does not show files yet; a folder that holds only files lists
as empty there.

## Importing a folder

`:import` picks a folder and uploads it whole — every file carries the path it
had inside, and the tree is rebuilt on the other side: directories become
folders, `.md` and `.markdown` files become documents (titled by their first
heading), anything else becomes a file in the tree. `:import!` does the same with
a zip, which is the shape `:export!` writes, so a workspace can be poured back
into another one.

Links between the imported files are rewritten as they land: a link to a
markdown file becomes the document's address, a link to an image becomes its
media URL, and anything pointing outside the upload — a URL, a file that was not
uploaded — is left exactly as it was written. Absolute paths, `..` segments and
anything a zip smuggles in are refused, and both the upload and what a zip unpacks to are bounded by
`WORKSPACE_MAX_UPLOAD_MB`.

A document has a real address, `/n/<space>/<folder>/<slug>`, which is what those
rewritten links point at and what the address bar shows, so any document can be
linked to from outside — behind the login, unlike a share link.

## Backing up to Proton Drive

The server can mirror itself into Proton Drive, where it appears as a device —
its own sync root, alongside the desktop clients — rather than as a folder
dropped in *My Files*. Everything is encrypted client-side before it leaves the
machine, by the same Rust SDK the Linux client uses, so Proton stores a
workspace it cannot read.

It is a third-party client: it identifies itself to Proton as
`external-drive-narl_workspace@<version>-alpha` and says so before it asks for
account details. It carries no Proton branding and is not supported by Proton.

Set it up once, interactively, because SRP and 2FA need a human:

```sh
narl-workspace proton-login                       # or, in Docker:
docker compose exec -it workspace narl-workspace proton-login
```

That stores a session — tokens, the mailbox password needed to rebuild the key
chain, and the account key salts — as a `0600` file next to the database. It
sits on the same volume as the workspace it protects, and it is enough to read the
account, so the volume is the thing to keep private. Built with
`--features keyring` on a host that has a Secret Service, the session goes to
the OS keyring instead and the file is only a fallback; the container has no
session bus, so there it is always the file.

Afterwards the server resumes on its own and nothing prompts again. Refresh
tokens are single-use, so every rotation is written back immediately — which is
also why two servers must not share one session file.

The mirror is one-way and change-driven: a write marks the workspace dirty, and once
it has been quiet for `WORKSPACE_BACKUP_DEBOUNCE_SEC` the mirror runs, with
`WORKSPACE_BACKUP_INTERVAL_MIN` as a backstop. An hour of writing is one backup, not
sixty. The device folder ends up as a copy of the data directory:

```text
narl-workspace/          the device — Proton allows only folders in a device root
  data/              the data directory, copied
    RESTORE.txt      what this is, and how to put it back
    diary.db         a VACUUM INTO snapshot — consistent, no write-ahead log
                     (the file keeps the name it was created with)
    uploads/<uuid>   every uploaded file, under the name the database knows
  files/             every file node again, as <space>/<folder>/<name>
```

`diary.db` becomes a new revision each time, so Proton Drive keeps the older
ones and a mistake that was mirrored can still be undone. Uploads are written
once and never rewritten. Restoring is copying the contents of `data/`
back into an empty data directory — no tool in between, which is the point.

| command | what it does |
| --- | --- |
| `narl-workspace proton-login` | log in and enable backups |
| `narl-workspace proton-status` | account, device, schedule, how much is mirrored |
| `narl-workspace backup-now` | mirror once and exit — for cron, or for nerves |
| `narl-workspace proton-logout` | forget the session; the mirror stays where it is |

From inside the workspace, `:backup` says when the last one finished and `:backup!`
runs one now. A failed backup is loud: it is reported by `:backup`, and the next
tick retries it.

`files/` is for reading, not restoring: the same bytes as `data/uploads/`, laid
out the way the tree shows them, so a PDF can be opened from Proton Drive's own
apps by its name. It follows the workspace — a rename or a move there is a
rename or a move on Drive, not a second upload, and a deleted file goes to
Drive's trash whatever `WORKSPACE_BACKUP_PRUNE` says, since `data/` still has
it. Two files that share a name in a folder are told apart by the later one's
id, `slides (42).pdf`. It costs the storage of every file twice; anything put
into `files/` by hand may be trashed, because the workspace owns that folder.

Deleting an entry does not delete it from the mirror unless `WORKSPACE_BACKUP_PRUNE`
is on. A backup that forgets on command is one accident away from being no
backup at all.


## Mail

Off until `WORKSPACE_SMTP_URL` is set, and then four kinds of mail, all plain
text and all addressed to `WORKSPACE_MAIL_TO`:

| kind | when |
| --- | --- |
| `reminder` | at `WORKSPACE_REMINDER_AT`, if nothing was written in the diary space that day |
| `card_due` | at `WORKSPACE_CARD_DUE_AT`, for every card due that day, and again once it is overdue |
| `digest` | at `WORKSPACE_DIGEST_AT`: what was written per space, what was finished, what is still open |
| `login` | a browser signed in that has not signed in before, or a run of wrong passwords |

`WORKSPACE_MAIL_KINDS` is the list of kinds that may be sent; drop one to switch
it off. Every "today" is asked in `WORKSPACE_TIMEZONE`, because whether an entry
was written today depends on where you are, and a day is not always 24 hours
long where the clocks change.

Nothing is sent straight from the code that decides a mail is due. It goes into
an outbox table under a key that names the occurrence — `reminder:2026-09-18`,
`card_due:41:2026-09-20:overdue` — and a tick a minute drains it. A restart
cannot resend yesterday's reminder, because the row is already there; a relay
that refuses cannot lose one, because the row is still there and is retried with
a widening gap, up to an hour apart.

The login mail says *a new browser*, not *a new person*: the device is
remembered as a SHA-256 of its `User-Agent`, which is honest about what it can
tell and needs no proxy header to be trusted for it.

```sh
narl-workspace mail-test      # one mail, straight out, bypassing the outbox
narl-workspace mail-status    # what is configured, what is queued, what failed
```

SMTP settings are wrong the first time, always. `compose.override.yml.example`
carries a [mailpit](https://mailpit.axllent.org/) service for exactly that:
point `WORKSPACE_SMTP_URL` at `smtp://mailpit:1025` and read the
reminders at <http://127.0.0.1:8025> before involving a real relay.

## Sharing

`:share` mints two 192-bit random secrets and copies
`https://your-host/s/<token>#<key>`. That page needs no session, renders the
entry read-only, and serves only the media files that entry currently embeds —
dropping a picture out of a shared entry immediately makes it unreachable
through the link.

The two halves do different jobs. The token names the entry; the key proves the
reader was given the whole link. A browser never sends a fragment to the server,
so the key stays on the reader's machine and out of every access log, referrer
and link preview along the way — a crawler or a chat client that harvested only
the path holds a URL that answers `404`. The server stores nothing but a
SHA-256 of the key and compares in constant time, and a wrong key is answered
exactly like an unknown token, so guessing never confirms that a token is live.

Because only the hash is kept, a link exists in full exactly once: in the moment
`:share` copies it. It cannot be re-read from the entry afterwards. `:link`
mints a fresh key for the same token — which retires the previous link, making
it the way to cut off a reader without unpublishing — and `:unshare` destroys
both halves.

## API

Everything except the two share routes requires the session cookie.

| method | path | purpose |
| --- | --- | --- |
| `POST` | `/api/login`, `/api/logout` | session |
| `GET` | `/api/me` | current user |
| `GET`/`POST` | `/api/spaces` | the spaces / create one |
| `GET`/`POST` | `/api/nodes` | children (`?parent=`) or search (`?q=`, `?space=`) / create |
| `GET`/`PUT`/`DELETE` | `/api/nodes/{id}` | read / update / delete (a delete takes the subtree) |
| `GET` | `/api/spaces/{id}/board` | the lists and cards of one space |
| `GET` | `/api/agenda` | open cards due within a week or in hand, on every board, and whether the diary has today |
| `POST` | `/api/spaces/{id}/lists` | add a list |
| `PUT`/`DELETE` | `/api/lists/{id}` | rename / delete a list and its cards |
| `POST` | `/api/lists/{id}/cards` | add a card |
| `PUT`/`DELETE` | `/api/cards/{id}` | update (title, body, due, done, link) / delete |
| `POST` | `/api/cards/{id}/move` | to another list and place in it |
| `POST` | `/api/nodes/{id}/move` | reparent, and place among the new siblings |
| `POST` | `/api/nodes/{id}/import` | import a folder or zip (multipart) under this node |
| `GET` | `/api/resolve?path=` | the node behind a slug path, `work/notes/monday` |
| `POST`/`DELETE` | `/api/nodes/{id}/share` | mint or rotate / revoke a share link |
| `GET` | `/api/export` | every document and file, as a zip |
| `GET`/`POST` | `/api/backup` | Proton Drive mirror: status / run now |
| `GET`/`POST` | `/api/media` | list / upload (multipart) |
| `GET`/`DELETE` | `/api/media/{id}` | serve / delete a file |
| `GET` | `/api/share/{token}/{key}` | public: read a shared document |
| `GET` | `/api/share/{token}/{key}/media/{id}` | public: media inside a shared document |

## Deploying

Put a TLS-terminating reverse proxy in front of it, point it at `WORKSPACE_BIND`,
and set `WORKSPACE_SECURE_COOKIE=1`. Nothing else is needed: no database server, no
runtime dependencies, no build tools on the host.

The app sends its own `Content-Security-Policy`, `Referrer-Policy: no-referrer`,
`X-Content-Type-Options` and `X-Frame-Options`; a proxy that adds its own should
not weaken them. `no-referrer` matters in particular, because a share token
lives in the URL and would otherwise leak to any site a shared entry links to.
Share links minted before the key existed are revoked by migration `0005`;
re-run `:share` on those entries to publish them again.
Uploaded files are only ever served as types that cannot execute — anything else
is handed back as an opaque download.

### Docker

```sh
cp .env.example .env   # set WORKSPACE_USER, WORKSPACE_PASSWORD, WORKSPACE_SECRET
docker compose up -d --build
```

The image builds the frontend with bun and the binary with cargo, then ships
only the binary on `debian:trixie-slim` (~86 MB), running as an unprivileged
user. `WORKSPACE_BIND` and `WORKSPACE_DATA_DIR` are forced in `compose.yml`, so the
values in `.env` do not have to change; everything else is read from `.env`.
Database and uploads live in the `diary-data` volume — named for what it held
when it was made — and survive recreation. Keep the project name if you rename
the directory (`COMPOSE_PROJECT_NAME=narl-diary`), or compose will look for the
volume under a new name and find an empty one.

By default the port is published on `127.0.0.1:4242` only. To serve it through
traefik instead, copy `compose.override.yml.example` to `compose.override.yml`
and adjust the host — it joins the external `narl` network, drops the published
port, and routes to container port 4242. Set `WORKSPACE_SECURE_COOKIE=1` in `.env`
once it is served over HTTPS.
