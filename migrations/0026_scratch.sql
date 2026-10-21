-- The scratchpad: notes written down in passing, before it is clear where they
-- belong. They live outside the tree on purpose — no space, no folder, no
-- name — so taking one costs nothing. Keeping one is making it a document,
-- which takes it out of here.
CREATE TABLE scratch_notes (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    body       TEXT    NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX idx_scratch_notes_updated ON scratch_notes (updated_at);
