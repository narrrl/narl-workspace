-- The text of the files in the tree, so a search finds a sentence in a `.txt`
-- or a `.csv` the way it finds one in a document. One row per file node, read
-- from the bytes `media_id` names; a file that is not text has a row with no
-- text, so it is read once and not again on every pass.
CREATE TABLE file_text (
    node_id  INTEGER PRIMARY KEY REFERENCES nodes (id) ON DELETE CASCADE,
    media_id TEXT    NOT NULL,
    text     TEXT    NOT NULL
);

CREATE VIRTUAL TABLE files_fts USING fts5 (
    text, content = 'file_text', content_rowid = 'node_id', tokenize = 'unicode61'
);

CREATE TRIGGER file_text_ai AFTER INSERT ON file_text BEGIN
    INSERT INTO files_fts (rowid, text) VALUES (new.node_id, new.text);
END;
CREATE TRIGGER file_text_ad AFTER DELETE ON file_text BEGIN
    INSERT INTO files_fts (files_fts, rowid, text) VALUES ('delete', old.node_id, old.text);
END;
CREATE TRIGGER file_text_au AFTER UPDATE ON file_text BEGIN
    INSERT INTO files_fts (files_fts, rowid, text) VALUES ('delete', old.node_id, old.text);
    INSERT INTO files_fts (rowid, text) VALUES (new.node_id, new.text);
END;

-- New bytes make the old text wrong: an SFTP save, and `:encrypt`, which
-- re-writes a file under a new media id. Dropped in the same statement, so
-- the plaintext of a file being sealed is gone the moment it is sealed.
CREATE TRIGGER file_text_stale AFTER UPDATE OF media_id ON nodes
WHEN new.media_id IS NOT old.media_id
BEGIN
    DELETE FROM file_text WHERE node_id = new.id;
END;

CREATE TRIGGER file_text_sealed_media AFTER UPDATE OF encrypted ON media
WHEN new.encrypted <> 0
BEGIN
    DELETE FROM file_text WHERE media_id = new.id;
END;

-- The last word, as in 0015: the text of a sealed file is never stored,
-- whatever path tries. Otherwise it would sit in `diary.db` and its mirror in
-- the clear while the bytes are sealed.
CREATE TRIGGER file_text_never_sealed BEFORE INSERT ON file_text
WHEN (SELECT encrypted FROM media WHERE id = new.media_id) IS NOT 0
BEGIN
    SELECT RAISE(ABORT, 'a sealed file has no text in the index');
END;
