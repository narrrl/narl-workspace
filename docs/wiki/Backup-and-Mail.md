# Backup and mail

## Backing up to Proton Drive

The server can mirror its data directory into Proton Drive, where it appears as
a device of its own, beside the desktop clients. Everything is encrypted before
it leaves the machine, by the same Rust SDK the Linux client uses, so Proton
stores a workspace it cannot read.

It is a third-party client. It names itself to Proton as
`external-drive-narl_workspace@<version>-alpha`, carries no Proton branding,
and is not supported by Proton.

### Logging in

Once, from the workspace:

```text
:backup login <proton-username>
```

It shows the disclosure above and asks before anything else, then asks for the
password and, when the account has one, the two-factor code. If Proton wants a
human check first, its own page opens in a sheet. An account whose only second
factor is a security key cannot log in from here; add an authenticator app to
it. The mirror starts within a minute; no restart is needed.

The same login runs from the server's terminal:

```sh
narl-workspace proton-login
docker compose exec -it workspace narl-workspace proton-login   # in Docker
```

The password itself is never stored, only the session and the key passphrases
derived at login. They are kept as a `0600` file next to the database, or in
the OS keyring with `--features keyring`. In a container,
`WORKSPACE_SESSION_KEY_FILE` seals that file under a key kept off the data
volume, such as a Docker secret.

### What it does

The mirror is one-way and runs after changes: once the workspace has been
quiet for `WORKSPACE_BACKUP_DEBOUNCE_SEC`, with `WORKSPACE_BACKUP_INTERVAL_MIN`
as a backstop. An hour of writing is one backup, not sixty.

```text
narl-workspace/          the device
  data/              the data directory, copied
    RESTORE.txt      what this is, and how to put it back
    diary.db         a consistent snapshot
    uploads/<uuid>   every uploaded file
  files/             every file in the tree again, as <space>/<folder>/<name>
```

- `diary.db` becomes a new revision on Drive each time, so Proton keeps the
  older ones and a mistake that was mirrored can still be undone.
- `files/` is for reading on Drive by name. It follows renames and moves.
- Restoring is copying the contents of `data/` into an empty data directory.
- A delete here does not delete on Drive unless `WORKSPACE_BACKUP_PRUNE` is on.

| command | what it does |
| --- | --- |
| `:backup` | when the last one finished, and whether it failed |
| `:backup!` | run one now |
| `:backup logout` | forget the session; the mirror stays where it is |
| `narl-workspace proton-status` | account, device, schedule, how much is mirrored |
| `narl-workspace backup-now` | mirror once and exit, for cron |

## Mail

Mail is off until `WORKSPACE_SMTP_URL` is set. Then four kinds of plain-text
mail go to `WORKSPACE_MAIL_TO`:

| kind | when |
| --- | --- |
| `reminder` | at `WORKSPACE_REMINDER_AT`, when nothing was written in the diary that day |
| `card_due` | at `WORKSPACE_CARD_DUE_AT`, for every card due that day, and again once it is overdue |
| `digest` | at `WORKSPACE_DIGEST_AT`: what was written in each space, what was finished, what is still open |
| `login` | a browser that has not signed in before signed in, or a run of wrong passwords |

`WORKSPACE_MAIL_KINDS` lists the kinds that may be sent. Every "today" is
counted in `WORKSPACE_TIMEZONE`.

Mail goes through an outbox: a restart cannot send yesterday's reminder twice,
and a relay that refuses cannot lose one. It is retried with a widening gap, up
to an hour apart, and given up after ten refusals. A card mail is checked
against its card before it goes, so a card finished since is not mailed.

```sh
narl-workspace mail-test      # one mail, straight out
narl-workspace mail-status    # what is configured, queued and failed
```

`compose.override.yml.example` has a [mailpit](https://mailpit.axllent.org/)
service for trying the settings out: point `WORKSPACE_SMTP_URL` at
`smtp://mailpit:1025` and read the mail at <http://127.0.0.1:8025>.
