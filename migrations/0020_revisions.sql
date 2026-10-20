-- What a document said before it was written over. A row holds the text that
-- was replaced, not the text that replaced it: the newest version is the
-- document itself. Autosave writes every few seconds, so an ordinary save
-- keeps one at most every ten minutes; `:w!`, a save over the mount and a
-- restore always keep one, because each of those may undo something.
CREATE TABLE revisions (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    node_id  INTEGER NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
    name     TEXT    NOT NULL,
    body     TEXT    NOT NULL,
    saved_at INTEGER NOT NULL,
    reason   TEXT    NOT NULL CHECK (reason IN ('edit', 'force', 'sftp', 'restore'))
);

CREATE INDEX idx_revisions_node ON revisions (node_id, saved_at DESC);
