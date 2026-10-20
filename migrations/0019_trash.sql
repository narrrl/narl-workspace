-- A delete puts a node in the trash rather than ending it. The trash is one
-- hidden root: a folder, not a space, so nothing that lists the spaces or
-- walks down from them ever reaches it, and a trashed node — moved under it
-- whole, with everything inside — drops out of the tree, the search, the
-- mount, the export and the readable mirror at once. The slug starts with a
-- dot, which no name ever slugifies to, so no space can take it.
INSERT INTO nodes (parent_id, kind, name, slug, role, created_at, updated_at)
VALUES (NULL, 'folder', 'trash', '.trash', 'trash', strftime('%s', 'now'), strftime('%s', 'now'));

-- One row per thing deleted, not per node under it: a folder comes back with
-- what was in it. Where it came from is kept by id, to go back to, and as the
-- path it had, to show once that folder is gone too. A trashed space has no
-- parent to go back to; it goes back to the root.
CREATE TABLE trash (
    node_id     INTEGER PRIMARY KEY REFERENCES nodes (id) ON DELETE CASCADE,
    from_parent INTEGER,
    from_space  INTEGER,
    from_path   TEXT    NOT NULL,
    deleted_at  INTEGER NOT NULL
);

CREATE INDEX idx_trash_deleted ON trash (deleted_at);
