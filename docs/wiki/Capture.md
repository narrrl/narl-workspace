# Capture

A thought arrives when no tab is open. `POST /api/capture` writes it down from
anything that can send a request: a phone shortcut, a script, `curl`. It takes a
token in place of a login.

## Tokens

```text
:token add phone     makes one, shows it once and copies it
:token               the settings sheet's tokens, each with when it was last used — x there retires one
:token rm phone      and that one captures nothing from the next request on
```

A token opens this one route and nothing else. It cannot read anything, not even
the list of spaces. Only its SHA-256 is kept on the server, so it is shown once,
when it is made. Wrong tokens slow down the way wrong passwords do, on a line of
their own, so guessing at them does not hold up the login.

## Sending

```sh
curl -H "Authorization: Bearer nwc_…" -H 'content-type: application/json' \
  -d '{"text":"call the bank"}' https://your-host/api/capture
```

| field | meaning |
| --- | --- |
| `text` | what to write down; for a card, the first line is the title and the rest its notes |
| `to` | `"card"` (the default), `"diary"`, `"note"` or `"document"` |
| `space` | for a card, the slug of the space whose inbox it goes in; for a document, the space its title is looked for in first |
| `document` | for `"document"`, which one, as a link names it: `Plan`, `[[Plan]]` or `/n/work/plan` |
| `note` | the id of the scratch note the text came from; it is dropped once the text has landed |

- `"to": "card"` puts a card in the inbox of the space named by `space`, or else
  of the first space that has a board.
- `"to": "diary"` adds `- 14:05 call the bank` to the end of today's entry, in
  `WORKSPACE_TIMEZONE`, and starts the entry from `templates/day` when there is
  none yet.
- `"to": "note"` puts it on the scratchpad, for when it is not clear yet where
  it belongs.
- `"to": "document"` adds it as a paragraph at the end of a document. The text
  it replaces is kept in the document's history, as a save keeps it.

The answer says where it went: `{"to":"card","id":7,"where":"work/backlog"}`.

## From a phone

**iPhone**: in Shortcuts, make a shortcut with *Ask for Input*, then *Get
Contents of URL*:

- URL: `https://your-host/api/capture`
- Method: `POST`
- Headers: `Authorization` = `Bearer nwc_…`
- Request body: JSON, with `text` set to the *Provided Input*, and `to` set to
  `diary` if that is where it should go.

**Android**: in [HTTP Shortcuts](https://http-shortcuts.rmy.ch/), make a shortcut
with a text-input variable, method `POST`, the same header, and a JSON body
`{"text": "{{input}}"}`.

Either one can sit on the home screen, or behind the share sheet.
