# ~/workspace

A private, terminal-themed workspace that runs in the browser: a diary, the
notes for work and for uni, and a board per space. One user, one SQLite file,
one binary. It was called `narl-diary` until it grew everything but the diary.

- **Rust + axum** backend, **SQLite** storage, **Svelte 5** frontend.
- Spaces (`diary`, `work`, `uni`, …) with folders nested to any depth inside them.
- Markdown documents with full-text search across a whole space (SQLite FTS5),
  and math in them set by KaTeX: `$x^2$` inline, `$$…$$` or a ```` ```math ````
  block on its own.
- Import a folder — or a zip of one — as a tree of documents, links and all.
- A kanban board per space, so what is open at work is not mixed up with uni.
- Drag, drop or paste images, video and audio straight into an entry.
- Files of any kind — PDFs, archives, recordings — live in the tree beside the
  notes about them, streamed to disk however large — and PDFs, text, code,
  tables and certificates open in place.
- Share any document behind an unguessable link — revocable at any time.
- A vault: files sealed with AES-256-GCM under a password that is never stored,
  readable only while it is unlocked.
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
`openssl rand -base64 32`. Changing it logs every device out, and so does
changing `WORKSPACE_PASSWORD` or `:logout!`, which ends every session but
needs no restart. Every signed-in browser is a row on the server: `:sessions`
lists them and `:sessions rm <n>` signs one out — a lost phone, say — without
touching the rest, and locks the vault as well. A setting that is set but cannot be read — a number out of
range, a switch that is neither on nor off — stops the start with the reason,
rather than being taken for its default. The database and
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
| `1` … `9`, `0` | switch to that space |
| `Shift-1` … `9` | send the marked nodes, or the selected one, to that space |
| `Tab`, `Shift-Tab` | next / previous space |
| `w` | overview of every space; `hjkl` then `Enter` |
| `f` | fullscreen the focused pane, or the focused tile |
| `Alt-h/j/k/l` | focus the tile in that direction, from the editor too; `Alt-h` past the leftmost tile focuses the list |
| `Alt-Shift-h/j/k/l` | move the focused tile that way |
| `L`, `Shift-Enter` | open the highlighted row in a new tile instead of the focused one |
| `gt` / `gT` | next / previous tile, in the editor too |
| `Alt-q` | close the focused tile (`:close`); asks first if its draft could not be written |
| `j` / `k` | move down / up, or scroll the document when it has focus |
| `gg` / `G` | first / last entry, or top / bottom of the document; a first `g`, `d`, `]` or `[` shows in the status line for a second while it waits for the second key |
| `Ctrl-D` / `Ctrl-U` | half a page down / up the open document |
| `zl` / `zh`, `zL` / `zH` | scroll the table, code block or formula at the top of the view sideways, a little or by half its width |
| `]]` / `[[` | the next / previous heading of the open document, in the editor too |
| `gO` | the document's headings as a list — `j`/`k`, `Enter` jumps there (`:outline`) |
| `gx` | open the file under the cursor in a tab of its own |
| `F` | a letter over every link and button in view; typing it follows it — links out of the workspace open in a tab of their own |
| `Enter`, `l` | open the highlighted entry; on an open keystore or sealed file, `Enter` asks for its password |
| `o` | a new document here, straight into insert mode |
| `O` | a new folder here |
| `i`, `a` | edit the open entry |
| `R` | rename what is highlighted — a document, a folder or a file — starting from its name (`:name!` renames the space) |
| `m` / `p` | pick nodes up — `m` again puts one back, `M` takes the whole folder — then drop them all into the folder you walk to |
| `Ctrl`/`Shift`-click | pick a row up / every row from the cursor to it; drag picked-up rows onto a folder, a crumb or a space |
| `b` | the board of this space |
| `t` | the agenda: what is late, due this week, or in hand, on every board |
| `T` | today's diary entry, started if there is none |
| `c` | capture a card into this space's inbox without leaving where you are |
| `Esc`, `q` | close the document, and its tile when there are several |
| `h` | up one level in the tree |
| `/` | full-text search in this space (`:noh` clears it); `Tab` or `Ctrl-N`/`Ctrl-P` walk the results without leaving the line, `Enter` opens the one lit |
| `Ctrl-P` | full-text search across every space; `Enter` goes to the match |
| `n` / `N` | the next / previous search result, opened beside the results; past the last it starts again at the first |
| `s` / `y` | toggle sharing / mint a new share link — either asks first when it would cut readers off |
| `x`, `dd` | delete the entry under the cursor (asks `y`/`N`) |
| `:` | command line — `Tab` / `Shift-Tab` cycle the matching commands, then the argument (folders for `:cd` and `:mv`, spaces for `:space`, options for `:set` …); `↑` / `↓` recall the lines run before that start as this one does |
| `Ctrl-S` | save from anywhere |
| `?` | help |

Inside the editor the full vim keymap is live — `w`/`b`, `dd`, `ciw`, visual
mode, macros, `:w` to write. `:help` lists every ex command; the useful ones are
`:w`, `:wq`, `:q!`, `:new [yyyy-mm-dd]`, `:date`, `:name`, `:share`, `:link`,
`:upload`, `:media`, `:search`, `:backup` and
`:set novim`. `:name` on its own moves up into the title; `Enter`, `Esc` or `↓`
there goes back down into the text. In the text, `]]`, `[[` and `gO` walk the
headings as they do in the reader. `gf` follows the link under the cursor, saving
first, and `gx` opens it in a new tab. The rest: `:today` opens today's entry, `:edit [id]` (or `:12`)
opens a node by id, `:next`, `:prev`, `:first`, `:last` and `:random` walk the
list (`:bn` and `:bp` walk the tiles), `:delete [id]` deletes, `:copy` puts the whole document on the clipboard,
`:clear` drops the search, `:reload` re-reads the folder, `:stats` counts words
and lines, `:search!` searches every space, `:set clipboard` / `:set
noclipboard` mirrors vim yanks to the system clipboard, `:logout` ends the
session, `:logout!` every session and `:sessions rm <n>` one other. The tree has its own: `:space [name]` switches (`:space! <name>`
makes one, `:rmspace [name]` deletes one and everything in it), `:mkdir <name>`
makes a folder, `:cd <name|..|/>` walks it and `:mv <folder|..|/>` moves what is
selected, and `:import` uploads a folder into the one being browsed
(`:import!` takes a `.zip` instead). `:upload` outside the editor puts files
into the folder being browsed, and so does dropping them on the list. `:name <text>` renames whatever is under
the cursor — a document, a folder, or a card on a board — and `:name!` renames
the space itself. The board has
`:board`, `:card <title>` (`:card!` links the open document to it),
`:due <when|->`, `:done` and `:list <name>` (`:list!` deletes one).

On a touch device modal editing is off by default (`:set vim` turns it on) and
the same actions are buttons: the `⋯` on a row — or a long press — opens what
the keys do to it: rename, move, share, seal, delete. A move is picked up
there and dropped with the button the dock then shows, in whichever folder
is open. Inside a folder the list starts with `..`, the way up. The dock also
has today's entry, an upload into the folder, a search across every space,
and on the board the card moves, renames and deletes; the agenda has a done
button per card.

## Tiles

Several documents can be open at once. `L` (or `Shift-Enter`) on a row opens
it in a tile of its own; `Enter` and `l` still open it in the focused tile.
Right of the list the tiles share the space the way hyprland's dwindle layout
does: two side by side, a third splits the newer one, and so on. Each keeps
its own scroll position and its own editor, and the keys always act on the
focused one. Opening a document that is already in a tile focuses that tile
rather than opening it twice.

`Alt-h/j/k/l` moves the focus between tiles and `Alt-Shift-h/j/k/l` moves the
tile itself; `gt` / `gT` and `:bn` / `:bp` cycle through them, `:ls` lists
them, `:b <n or name>` focuses one, `:close` (`Alt-q`) closes one and `:only`
every other. `:next` and `:prev` still walk the list. Each space remembers its
tiles, so switching away and back finds them as they were. A card's notes
open in a tile beside what was being read, and `q` there goes back to the
board. Under 768px wide only the focused tile is shown.

## The SFTP mount

The workspace can be mounted as a filesystem over SFTP, so an editor or a file
manager on another machine works on it directly. It is off until
`WORKSPACE_SFTP_BIND` is set (`127.0.0.1:2222`, or `0.0.0.0:2222` inside a
container, with the port published in `compose.override.yml`). Only public
keys get in, and only the ones registered from inside the workspace:

```text
:sshkey add laptop ssh-ed25519 AAAA… you@laptop
:sshkey              lists them, with their fingerprints
:sshkey rm laptop    and this one is out, mounted or not
:sshkey vault laptop on    this key may read sealed files while the vault is open
```

```sh
sshfs -p 2222 me@your-host:/ ~/workspace     # the user name is ignored
```

The root holds the spaces, `mkdir` there makes one, and anywhere else a
folder. A document is `<slug>.md`, read as `# Title` followed by its body, and a
leading heading written back renames it. The server's host key is made on first
start as `sftp_host_key` in the data directory.

## Boards

`b`, the board door on the rail, or `:board` swaps the tree for the board
of the space being browsed, and swaps it back. Every space has one except the
diary — a journal has no backlog — and `:set board` / `:set noboard` changes
that for the space you are in (the dimmed board door on the rail does the
same). A new board starts with `backlog`, `doing` and `done`, and the columns mean
something by where they stand rather than by their names: the first is the
inbox new cards land in, the last is done. A card is finished exactly when it
sits in the last column — moving it there stamps it, moving it out reopens it —
so a tick and a column can never disagree; marking a card done through the API
moves it there too, and deleting the last column makes the one before it done.
`:list <name>` adds a column in front of the last one, `:name!` on the board renames the column under the cursor, and
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
once its notes are gone. Only a document can be a card's notes; a folder or a
file is refused.

## The agenda

`t` (or `:agenda`) gathers every unfinished card from every board that is late,
due within the week, or sitting in a column between the inbox and done — the
work already in hand — and says whether the diary has an entry for today. `j`/`k`
walk it, `Enter` goes to the card on its own board, `Space` finishes it where
it stands, `D` sets its due date (`:due` on the line), `H` / `L` send it to
the list left / right on its board, and `T` writes today's entry. The bar shows `late` and `today` counts
whenever either is above zero, and the agenda opens by itself once a day when
something is late or due, so nothing has to be remembered to be looked at.

## Not losing things

Every change is written to `localStorage` as it is typed and to the server a
couple of seconds after typing stops, so `:w` is a habit rather than a
necessity. When the server cannot be reached the draft stays in the browser
and the save is tried again, less often the longer it fails, and at once when
the browser says it is back online. A session that runs out mid-sentence
shows the login, and logging in again carries on where it stopped, draft and
all.

A document saved somewhere else since it was opened here — another tab, the
phone, the SFTP mount — is not written over: `:w` says so, `:e!` takes the
other version, and `:w!` writes this one anyway. Search results say which
space and folder each match is in. A document made with `o` and left with neither a name nor a word in
it is removed again on the way out, so abandoned starts do not pile up. If a tab dies mid-document, opening it again restores the draft and
says so — `:e!` throws it away, the way vim handles a swap file.

`:export` downloads the open document as markdown. `:export!` downloads the
whole workspace as a zip: the tree as real directories, empty folders too, one
markdown file per document, every file in the tree where it sits and every
other embedded file in `media/`, under their real names, and relative links
between documents, folders and files, so the archive reads in any markdown
viewer without this application. What plain files have no place for — when
each entry was written, names as they were typed, which space is the diary,
every board with its cards — goes into one `.workspace.json` beside them.

The diary is the diary by role, not by name: rename it and the reminder and
the agenda still know which space it is, and it cannot be deleted — empty it
instead. Before a new build migrates the database it keeps a copy of it as
it was, `diary.db.pre-<version>`, next to it in the data directory.

An upload that nothing embeds any more — pasted, then deleted from the
document — is kept a week and then removed, along with what a killed upload
or a crash leaves in `uploads/`. That runs at start and once a day. A file in
the tree is never one of them: it is deleted from its folder, not from
`:media`.

`:media` lists every upload: `j`/`k` walk it, `i` or `Enter` embeds the
highlighted one at the cursor while writing, `y` copies its address and `x`
deletes it, asking first.

## Files

A file is a node like a document, only with bytes instead of text: it sits in
a folder, shows its size in the list, and `Enter` opens it. What is shown in
place:

- images, video and audio;
- PDFs, in the browser's own viewer;
- text and source code of any kind, recognised by its name or, failing that, by
  its first few kilobytes, with line numbers and the first 256 KiB of it;
- markdown, rendered, with `source` (or `:source`) to see it as written;
- CSV and TSV, as a table of the first 500 rows;
- certificates, keys and keystores — `.pem`, `.crt`, `.cer`, `.der`, `.csr`,
  `.key`, `.p8`, `.p12`, `.pfx` — read on the server: subject, issuer,
  validity, key, names, usage and fingerprints, and which key belongs to which
  certificate. A `.p12` or an encrypted key asks for its password, which opens
  it for that one look and is never stored. Nothing of a private key is ever
  sent back but its algorithm and size.

Anything else is a download: `:download` (`:dl`) fetches the file under the
cursor, and `gx` opens one in a tab of its own as the browser shows it. `:name`
renames it, and the download is called whatever it is called now; `:mv`, `m`
and `p` move it; `x` deletes it. `:upload` picks files for the folder being
browsed, or drop them onto the list. Each upload is written to disk as it
arrives, never held in memory, so the only ceiling is `WORKSPACE_MAX_UPLOAD_MB`
(1024 by default) — which is per request, so a proxy in front needs a body
limit at least that large too.

The SFTP mount shows files beside the documents, under their own names — a
later file that shares one is told apart by its id, `slides (42).pdf` — and
anything copied in that is not a `.md` becomes a file in the tree, streamed to
disk under the same size limit. A file copied into a sealed folder is sealed
on the way in while the vault is open in the browser. Reading one back takes
more: a key is a standing credential kept on another machine, so it reads
sealed files only once given vault access with `:sshkey vault <name> on`, and
none has it to start with. Such a key, while the vault is open, opens a sealed
file decrypted as it is read, and it is sealed again when it is saved,
wherever it sits. While the vault is shut, or once the access is taken away,
both are refused, and either cuts off a read already under way — though a
client that cached what it read keeps that copy until it lets go of it.
Nothing done over the mount holds the vault open; only the browser does. A
session holds at most 256 handles and 64 MB in memory, the server at most 32
connections, and one whose client has stopped answering is dropped within a
few minutes.
Editor droppings — swap files, `~` backups, dotted temp files — stay in
memory for the session, and a dotted temp file renamed onto a real name is kept
as that file, which is how `rsync` and most editors save.

If a document is saved in the browser while an SFTP client holds it open, the
client's save does not overwrite it: it lands beside it as `<name>-conflict.md`,
and both are kept. Deleting a key in the browser ends what that key mounted,
not just its next login.

## The vault

Some files deserve more than the login: scans of documents, contracts, health
records. The vault encrypts them at rest under a password of their own, so
they stay unreadable to someone who has the data directory or the Proton
account the mirror writes to, and to a logged-in browser while the vault is
shut — to anyone, in short, who does not know the vault password.

`:vault` sets the password the first time (ten characters at least) and says
whether the vault is open afterwards. `:upload!` puts files into the folder
being browsed sealed — encrypted as they stream in, so the plaintext never
touches the disk. `:encrypt` seals the file under the cursor; `:decrypt` takes
it back out. `:unlock` and `:lock` open and close it, `:passwd` changes the
password. A sealed file carries a lock in the list, and the bar says `vault
open` while it is.

`:passwd` makes a new master key as well as a new password. The old password
stops opening anything at once; every sealed file is then re-sealed under the
new key in the background, under a new name, while the vault is open — `:vault`
shows how far it has come, and locking halfway only pauses it until the next
`:unlock`. The old copies are deleted from `uploads/` and, on the next mirror
run, from Proton Drive for good. After that, a copy of `diary.db` from before
the change (a `diary.db.pre-*` snapshot, an old revision in Proton Drive) and
the old password open nothing that is still kept anywhere. What they cannot
take back is a copy someone already made of the files themselves. Re-sealing
rewrites only each file's header, but every sealed file is uploaded to Proton
Drive once more. A second `:passwd` waits until the first has finished.

`:encrypt` on a folder or space seals it for good, not just once: every file
in it now, and every file that lands in it later. An upload, an import, an SFTP
write and a file or folder moved in are all sealed on the way in. While the
vault is shut they are refused rather than stored readable, and the browser
asks for the password instead. Folders made inside are sealed too, and a
sealed folder carries a lock like a sealed file. `:decrypt` on that folder
unseals it with everything in it; a file or folder inside it cannot be
decrypted on its own. Documents are not files and stay readable, as below.
The database holds the line as well: a file node whose bytes are not sealed
cannot be put in a sealed folder by any path.

The vault closes itself after `WORKSPACE_VAULT_IDLE_MIN` minutes of nobody
using it (15 by default) — the key is dropped from memory then, not merely
refused on the next look — on logout, and whenever the server restarts. While
it is closed a sealed file shows as sealed and nothing more; a share link never
serves one, open or not; and `:export!` refuses until it is open, because an
archive meant to be read without this application has to hold the files
opened.

How it works: Argon2id turns the password into a key that seals a random
master key, and only that sealed copy is stored. Each file gets a random key of
its own, sealed under the master key in the file's header together with its
id, and its bytes are AES-256-GCM in 64 KiB records, the last one marked as
last — so a file that is swapped, cut short or tampered with refuses to open.
Changing the password makes a new master key. Until every file is re-sealed,
the old one is kept sealed under the new one, so either kind of file opens and
an interrupted re-seal carries on where it stopped; re-sealing a file wraps its
own key under the new master key and copies its records as they are.

A sealed file is served with `Cache-Control: no-store`, so the browser keeps
no copy of what it showed once the vault shuts. A sealed image in a document
shows as a button while the vault is shut, which asks for the password and
puts the image back. `:share` says how many sealed files a document holds,
since its readers will not see them, and `:encrypt` says how many shared
documents just lost a picture to the vault. `:encrypt` on a folder seals
what it can and counts what it could not, rather than stopping at the first.

Only the bytes are sealed. A sealed file's name, and the names and text of
the documents around it, stay readable in `diary.db` — and in the mirror of
it — like everything else outside the vault.

**There is no recovery.** Forget the vault password and every sealed file is
gone — from the server and from every backup — which is exactly what makes it
worth having. The password is not in `.env`, not in `diary.db`, and not in the
mirror.

Sealing a file that already exists writes it anew under a new id, rewrites the
links of documents that embed it, and deletes the plaintext from the disk. The
mirror then deletes the old copy from Proton Drive outright — trash and all —
whatever `WORKSPACE_BACKUP_PRUNE` says, and takes the file out of the readable
`files/` view. The sealed copy goes up to `data/uploads/` like any other.
Proton cannot read either; only the vault password can read the sealed one.

## Importing a folder

`:import` picks a folder and uploads it whole — every file carries the path it
had inside, and the tree is rebuilt on the other side: directories become
folders, `.md` and `.markdown` files become documents (titled by their first
heading), anything else becomes a file in the tree. `:import!` does the same with
a zip, which is the shape `:export!` writes. `:import! /` pours an `:export!`
back in at the root: each directory becomes a space again — or goes into the
space already there by that name — and with `.workspace.json` beside it the
dates, names, boards and cards come back as they were, embedded files go back
to being embedded rather than a `media` folder, and a file that happened to end
in `.md` stays a file.

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

Set it up once, interactively, because SRP and 2FA need a human. From the
workspace:

```text
:backup login <proton-username>
```

It shows the disclosure above and asks before anything else, then asks for the
password and, when the account has one, the two-factor code. If Proton wants a
human check first, its own page opens in a sheet; solve it and the login goes
on. An account whose only second factor is a security key cannot log in from
here — add an authenticator app to it. `:backup logout` forgets the session
again. The mirror starts within a minute of the login; no restart is needed.

The same login runs from the server's terminal, for a check that a terminal
can answer:

```sh
narl-workspace proton-login                       # or, in Docker:
docker compose exec -it workspace narl-workspace proton-login
```

That checks the password against the account's keys and stores a session —
the tokens, and the key passphrases derived from the password at login — as a
`0600` file next to the database. The password itself is never stored: the
passphrases unlock the keys but cannot log in, change the password or turn 2FA
off. They are still enough to read the account, and the file sits on the same
volume as the workspace it protects, so the volume is the thing to keep private.
A session stored by an older build, which kept the password, is rewritten
without it on the next start; no new login is needed. Built with
`--features keyring` on a host that has a Secret Service, the session goes to
the OS keyring instead and the file is only a fallback; the container has no
session bus, so there it is always the file.

In a container, `WORKSPACE_SESSION_KEY_FILE` does what the keyring would: the
file is sealed with AES-256-GCM under a key read from somewhere other than the
volume, so a copy of the volume alone is no longer a session. A Docker secret is
the place for it:

```sh
mkdir -p secrets
openssl rand -base64 32 > secrets/session.key
sudo chown 10001 secrets/session.key && sudo chmod 400 secrets/session.key
```

then the `secrets:` lines from `compose.override.yml.example`, and
`WORKSPACE_SESSION_KEY_FILE=/run/secrets/session_key` in `.env`. A plain session
already on the volume is sealed the next time it is read. Lose the key and the
session goes with it; `proton-login` makes a new one.

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
a widening gap, up to an hour apart. After ten refusals a mail is left for
`mail-status` to show and no longer stands in front of the ones behind it.

A card mail is checked against its card before it goes: one finished, deleted
or re-dated since it was queued, or on a board that has been switched off, is
dropped unsent. Mails that are done with are forgotten after 90 days. The
reminder needs a diary to be about, and says nothing when there is none.

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

The page sends the key on to the server once, in the body of the request that
reads the entry, never in a URL. That answer gives the browser a cookie scoped
to this one link's path, good for 12 hours, and the entry's pictures and
videos are fetched with it — so not even this server's reverse proxy logs the
key. A fresh key from `:link`, or `:unshare`, voids the cookies handed out
under the old one.

Because only the hash is kept, a link exists in full exactly once: in the moment
`:share` copies it. It cannot be re-read from the entry afterwards. `:link`
mints a fresh key for the same token — which retires the previous link, making
it the way to cut off a reader without unpublishing — and `:unshare` destroys
both halves.

## API

Everything except the login, the health check and the two share routes
requires the session cookie. Media answers `Range` requests, so a video seeks,
sealed or not.

| method | path | purpose |
| --- | --- | --- |
| `GET` | `/api/health` | public: up, and the database answers — for a supervisor |
| `POST` | `/api/login`, `/api/logout` | session; `/api/logout?everywhere=true` ends every one |
| `GET` | `/api/me` | current user |
| `GET` | `/api/sessions` | every signed-in browser, the one seen last first |
| `DELETE` | `/api/sessions/{id}` | sign another browser out, and lock the vault |
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
| `POST` | `/api/nodes/move` | `{ ids, parent_id }`: several into one folder, all or none |
| `POST` | `/api/nodes/{id}/import` | import a folder or zip (multipart) under this node |
| `POST` | `/api/import` | import an export at the root: directories become spaces |
| `GET` | `/api/resolve?path=` | the node behind a slug path, `work/notes/monday` |
| `POST`/`DELETE` | `/api/nodes/{id}/share` | mint or rotate / revoke a share link |
| `GET` | `/api/export` | the whole workspace, boards and dates included, as a zip |
| `GET`/`POST` | `/api/backup` | Proton Drive mirror: status / run now |
| `POST`/`DELETE` | `/api/backup/login` | log the mirror in to Proton, body `{"username","password"}` / forget its session |
| `POST` | `/api/backup/login/totp`, `/api/backup/login/captcha` | the two-factor code `{"code"}`, or the solved human check `{"token"}`, a waiting login asked for |
| `GET`/`POST` | `/api/media` | list / upload (multipart) |
| `GET`/`DELETE` | `/api/media/{id}` | serve (with `Range`) / delete a file nothing in the tree owns |
| `GET` | `/api/spaces/overview` | every space with what was last written in it |
| `POST` | `/api/nodes/{id}/files` | upload files into this folder as file nodes (`?encrypt=true` seals them) |
| `POST` | `/api/nodes/{id}/inspect` | what a certificate, key or keystore file holds (`{password}` opens a protected one) |
| `POST` | `/api/nodes/{id}/encrypt`, `/decrypt` | seal a file, or a folder for good — what is in it and what lands later — or take it out |
| `GET`/`POST`/`PUT` | `/api/vault` | status / set the password once / change it |
| `POST` | `/api/vault/unlock`, `/api/vault/lock` | open / close the vault |
| `GET`/`POST` | `/api/ssh-keys` | the keys that may mount over SFTP / add one |
| `DELETE` | `/api/ssh-keys/{id}` | remove a key, which ends its mounts |
| `POST` | `/api/share/{token}` | public: read a shared document, body `{"key"}`; sets the share's media cookie |
| `GET` | `/api/share/{token}/media/{id}` | public: media inside a shared document, with that cookie |

## Deploying

Put a TLS-terminating reverse proxy in front of it, point it at `WORKSPACE_BIND`,
and set `WORKSPACE_SECURE_COOKIE=1`. Nothing else is needed: no database server, no
runtime dependencies, no build tools on the host. The server will not start
while `WORKSPACE_SECRET` or `WORKSPACE_PASSWORD` is still the example from
`.env.example`: with the example secret anyone could sign themselves a session.

Wrong passwords are answered more and more slowly — free for three, then 1 s,
2 s, 4 s… up to 30 s. A browser that has logged in before keeps a signed
`diary_device` cookie (`__Host-diary_device` over HTTPS) and is slowed only by
its own mistakes, so someone guessing at the login from elsewhere cannot keep
you out of a browser you already use. A browser that has never logged in waits
in the same line as the guesses. The cookie opens nothing and survives
logging out.

With `WORKSPACE_SECURE_COOKIE=1` the session cookie is `Secure` and named
`__Host-diary_session`, so the browser keeps it to this host and HTTPS, and every
response carries `Strict-Transport-Security` for a year. Turning it on logs
everyone out once.

The app sends its own `Content-Security-Policy`, `Referrer-Policy: no-referrer`,
`X-Content-Type-Options`, `X-Frame-Options`, `Cross-Origin-Opener-Policy`,
`Cross-Origin-Resource-Policy` and a `Permissions-Policy` that turns off camera,
microphone and location; API answers are `Cache-Control: no-store` unless they
say otherwise. A proxy that adds its own should not weaken them. `no-referrer` matters in particular, because a share token
lives in the URL and would otherwise leak to any site a shared entry links to.
Share links minted before the key existed are revoked by migration `0005`;
re-run `:share` on those entries to publish them again.
Uploaded files are only ever served as types that cannot execute — anything else
is handed back as an opaque download.

A write that a browser says came from another origin — `Sec-Fetch-Site`, or
`Origin` against `Host` where a browser is too old to send that — is refused,
so a proxy must pass the `Host` header through unchanged (traefik and nginx's
`proxy_set_header Host $host` both do). Only the upload routes accept a body as
large as `WORKSPACE_MAX_UPLOAD_MB`; everything else stops at 16 MiB, and the
login at 16 KiB, before anyone has proved who they are. `SIGTERM` gets in-flight
requests eight seconds to finish, inside the ten `docker stop` waits. Any
other request that has not been answered after 30 seconds is given up on with
a `503`, except the ones that move files around: uploads, imports, the export,
re-sealing and a backup run.

Every request is logged at `info`, path and status and time, never a query
string, and with a share link's token and key left out. A background loop that
panics — the mirror, the mail, the sweep, the SFTP server — is logged and
started again rather than lost until the next restart. If
`WORKSPACE_SESSION_KEY_FILE` cannot be read the server starts anyway with
backups off, and `:backup` says why.

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
port, routes to container port 4242, and sets `WORKSPACE_SECURE_COOKIE=1`, since
traefik serves it over HTTPS.
