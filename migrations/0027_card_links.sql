-- A document and its cards, seen from both ends. A card has always pointed at
-- one document, its notes, and the document could not see it back; a card's
-- summary could link to documents, and nothing kept those links.
--
-- `card_links` is `links` for a card's summary: what its `/n/` paths and
-- `[[title]]`s lead to, rebuilt whenever the summary is written. Read the
-- other way round, it is the cards that mention a document, and the summaries
-- a rename or a move rewrites along with the documents.
--
-- Nothing fills it for the cards already written; the server indexes them when
-- it starts and finds it empty (see `links::backfill`).
CREATE TABLE card_links (
    card_id INTEGER NOT NULL REFERENCES cards (id) ON DELETE CASCADE,
    kind    TEXT    NOT NULL CHECK (kind IN ('path', 'wiki')),
    target  TEXT    NOT NULL,
    to_id   INTEGER NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    PRIMARY KEY (card_id, kind, target)
) WITHOUT ROWID;

CREATE INDEX idx_card_links_to ON card_links (to_id);

-- Search finds a card the way it finds a document: by its title, its summary,
-- or a step on its checklist. The steps are JSON on the card, so the index
-- keeps its own copy of their text rather than reading the card's columns.
CREATE VIRTUAL TABLE cards_fts USING fts5 (title, body, steps, tokenize = 'unicode61');

CREATE TRIGGER cards_ai AFTER INSERT ON cards BEGIN
    INSERT INTO cards_fts (rowid, title, body, steps)
    VALUES (new.id, new.title, new.body,
            (SELECT COALESCE(group_concat(json_extract(value, '$.text'), ' '), '')
             FROM json_each(new.checklist)));
END;
CREATE TRIGGER cards_ad AFTER DELETE ON cards BEGIN
    DELETE FROM cards_fts WHERE rowid = old.id;
END;
CREATE TRIGGER cards_au AFTER UPDATE OF title, body, checklist ON cards BEGIN
    DELETE FROM cards_fts WHERE rowid = old.id;
    INSERT INTO cards_fts (rowid, title, body, steps)
    VALUES (new.id, new.title, new.body,
            (SELECT COALESCE(group_concat(json_extract(value, '$.text'), ' '), '')
             FROM json_each(new.checklist)));
END;

-- The cards written before the triggers existed.
INSERT INTO cards_fts (rowid, title, body, steps)
SELECT id, title, body,
       (SELECT COALESCE(group_concat(json_extract(value, '$.text'), ' '), '')
        FROM json_each(cards.checklist))
FROM cards;
