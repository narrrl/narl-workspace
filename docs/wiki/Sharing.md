# Sharing

Any document can be shared read-only behind a link that cannot be guessed. The
reader needs no account.

| key or command | what it does |
| --- | --- |
| `:share` | share the document and copy its link |
| `:share new` | make a new link for the same document; the old one stops working (asks first) |
| `:share off` | stop sharing (asks first) |
| `:info share` | the share part of the info sheet: whether the document is public and the shape of its link; `s` there shares or stops, `y` makes a new link |

The link looks like `https://your-host/s/<token>#<key>`. The page renders the
document read-only and serves only the pictures and videos it embeds right now:
take a picture out of the document and it is gone from the link too. Links to
other documents in it are turned into plain text, since the reader could not
follow them. Sealed files are never shown; `:share` says how many the document
holds.

## Why the link has two halves

The token names the document; the key proves the reader was given the whole
link. A browser never sends the part after `#` to the server, so the key stays
out of access logs, referrers and link previews. A crawler or chat client that
kept only the path holds a link that answers `404`.

The server keeps only a SHA-256 of the key and compares it in constant time, and
a wrong key is answered exactly like an unknown token. That also means the full
link exists only once, when `:share` copies it. It cannot be read back from the
document later; `:share new` makes a new one instead, which is also the way to cut a
reader off without unpublishing.

A document in the trash answers like an unknown link, and works again with the
same link when it is restored.
