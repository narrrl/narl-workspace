-- The links between documents, as their text writes them: a `/n/...` path,
-- or a `[[title]]`. Rebuilt from a document's text whenever it is written,
-- and read the other way round: for what links to a document, and for the
-- texts to rewrite when what they link to is renamed or moved.
--
-- Nothing fills it for the documents already written; the server indexes
-- them all when it starts and finds it empty (see `links::backfill`).
CREATE TABLE links (
    from_id INTEGER NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    kind    TEXT    NOT NULL CHECK (kind IN ('path', 'wiki')),
    -- What the link says: the path after `/n/`, or the title in the brackets.
    target  TEXT    NOT NULL,
    to_id   INTEGER NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    PRIMARY KEY (from_id, kind, target)
) WITHOUT ROWID;

CREATE INDEX idx_links_to ON links (to_id);
