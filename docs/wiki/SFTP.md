# The SFTP mount

The workspace can be mounted as a filesystem over SFTP, so an editor or a file
manager on another machine works on it directly.

## Turning it on

It is off until `WORKSPACE_SFTP_BIND` is set, for example `127.0.0.1:2222`, or
`0.0.0.0:2222` in a container with the port published. Only public keys get
in, and only the ones registered from inside the workspace:

```text
:sshkey add laptop ssh-ed25519 AAAA… you@laptop
:sshkey                     the settings sheet's ssh keys, with their fingerprints
:sshkey rm laptop           and this one is out, mounted or not
:sshkey vault laptop on     this key may read sealed files while the vault is open
```

Then, on the other machine:

```sh
sshfs -p 2222 me@your-host:/ ~/workspace     # the user name is ignored
```

The server makes its host key on first start, as `sftp_host_key` in the data
directory.

## What it looks like

- The root holds the spaces; `mkdir` there makes a space, anywhere else a
  folder.
- A document is `<slug>.md`, read as `# Title` followed by its body. Writing a
  different first heading back renames it.
- Files sit beside the documents under their own names. A later file with the
  same name is told apart by its id: `slides (42).pdf`.
- A document's own files are in `<slug>/`, beside its `.md`. The directory is
  listed once the document has a file, and can be copied into either way. It
  holds files and nothing else, and moves when the `.md` is renamed.
- Anything copied in that is not a `.md` becomes a file, and its text is
  indexed for search.
- `rm` and `rmdir` put things in the trash, as a delete in the browser does.

## Editors

Editors and `rsync` save by writing a temporary file and renaming it over the
real one. The mount understands that:

- swap files, `~` backups and dotted temporary files stay in memory for the
  session and never appear in the workspace;
- a temporary file renamed onto a real name saves into that document or file;
- a rename onto a name that is taken replaces what was there, as `rename(2)`
  does, without filling the trash.

A save over the mount keeps a version of what it replaces, see
[history](Not-Losing-Things.md#history). If the document was saved in the
browser while the SFTP client had it open, the client's save does not
overwrite it: it lands beside it as `<name>-conflict.md`, and both are kept.

## Sealed files

A file copied into a sealed folder is sealed on the way in, while the vault is
open in the browser. Reading one back needs more: a key is a standing
credential on another machine, so it reads sealed files only after
`:sshkey vault <name> on`, and only while the vault is open. Shutting the vault
or taking the access away cuts off a read already under way. Nothing done over
the mount keeps the vault open; only the browser does.

## Limits

A session holds at most 256 handles and 64 MB in memory, the server at most 32
connections, and a client that stops answering is dropped within a few minutes.
Removing a key in the browser ends what that key has mounted.
