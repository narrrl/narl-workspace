-- The scratchpad is where things are written down before it is clear where
-- they belong, which makes it the first place to look for something half
-- remembered. Search finds a note the way it finds a document or a card.
CREATE VIRTUAL TABLE scratch_fts USING fts5 (body, tokenize = 'unicode61');

CREATE TRIGGER scratch_ai AFTER INSERT ON scratch_notes BEGIN
    INSERT INTO scratch_fts (rowid, body) VALUES (new.id, new.body);
END;
CREATE TRIGGER scratch_ad AFTER DELETE ON scratch_notes BEGIN
    DELETE FROM scratch_fts WHERE rowid = old.id;
END;
CREATE TRIGGER scratch_au AFTER UPDATE OF body ON scratch_notes BEGIN
    DELETE FROM scratch_fts WHERE rowid = old.id;
    INSERT INTO scratch_fts (rowid, body) VALUES (new.id, new.body);
END;

-- The notes written before the triggers existed.
INSERT INTO scratch_fts (rowid, body) SELECT id, body FROM scratch_notes;
