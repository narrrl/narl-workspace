# The vault

Some files deserve more than the login: scans of documents, contracts, health
records. The vault encrypts them at rest under a password of their own. They
stay unreadable to anyone without it: someone with a copy of the data directory
or the Proton account the mirror writes to, and even a logged-in browser while
the vault is shut.

> **There is no recovery.** Forget the vault password and every sealed file is
> gone, from the server and from every backup. The password is not in `.env`,
> not in `diary.db` and not in the mirror.

## Using it

| command | what it does |
| --- | --- |
| `:vault` | set the password the first time (ten characters at least); afterwards, say whether the vault is open |
| `:unlock` / `:lock` | open / close the vault |
| `:upload!` | upload files sealed, encrypted as they stream in, so the plaintext never touches the disk |
| `:encrypt` | seal the file under the cursor; on a folder or space, seal it for good |
| `:decrypt` | take the file, or the sealed folder, back out |
| `:passwd` | change the password |

A sealed file carries a lock in the list, and the bar says `vault open` while it
is. While the vault is shut, `Enter` on a sealed file asks for the password.

## Sealed folders

`:encrypt` on a folder or a space seals it for good: every file in it now, and
every file that lands in it later. An upload, an import, an SFTP write and a
file moved in are all sealed on the way in. While the vault is shut they are
refused rather than stored readable. Folders made inside are sealed too.
`:decrypt` on the folder unseals it with everything in it.

Documents are not files and stay readable. Only the bytes of files are sealed;
the names of sealed files, and the documents around them, are as readable in
`diary.db` as everything else.

## When it closes

The vault closes itself after `WORKSPACE_VAULT_IDLE_MIN` minutes with nobody
using it (15 by default), on logout, and whenever the server restarts. The key
is dropped from memory then. While it is shut:

- a sealed file shows as sealed and nothing more;
- a share link never serves one, open or not;
- `:export!` refuses.

A sealed file is served with `Cache-Control: no-store`, so the browser keeps no
copy once the vault shuts. A sealed file is never indexed for search.

## Changing the password

`:passwd` makes a new master key as well as a new password. The old password
stops opening anything at once. Every sealed file is then sealed again under the
new key in the background while the vault is open; `:vault` shows how far it
has come. After that, an old copy of `diary.db` together with the old password
opens nothing still kept anywhere.

## How it works

Argon2id turns the password into a key that seals a random master key; only
that sealed copy is stored. Each file has a random key of its own, sealed under
the master key in the file's header together with the file's id. Its bytes are
AES-256-GCM in 64 KiB records, the last one marked as last, so a file that is
swapped, cut short or changed refuses to open.

Sealing an existing file writes it again under a new id, rewrites the documents
that embed it, and deletes the plaintext from the disk and from Proton Drive.

Over the [SFTP mount](SFTP.md), a key reads sealed files only once it has been
given vault access with `:sshkey vault <name> on`, and only while the vault is
open in a browser.
