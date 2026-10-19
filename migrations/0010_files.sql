-- Files join the tree. A PDF or a zip used to exist only as media embedded in
-- a document; now it can sit in a folder beside the notes about it, as a node
-- of its own kind that points at one row of `media`.

-- Files only: the bytes. Deleting the media row — `rm` in the media browser —
-- takes the file out of the tree with it.
ALTER TABLE nodes ADD COLUMN media_id TEXT REFERENCES media (id) ON DELETE CASCADE
    CHECK ((kind = 'file') = (media_id IS NOT NULL));

-- `kind` has to admit 'file', and SQLite cannot alter a CHECK constraint. The
-- usual way round is rebuilding the table with foreign keys off, but sqlx runs
-- every migration inside a transaction, where that pragma does nothing — and
-- dropping `nodes` with them on cascades into every card, list and attachment.
--
-- Widening a CHECK changes nothing on disk, though, so SQLite documents editing
-- the stored definition instead. RESET re-reads the schema on this connection;
-- the CREATE INDEX after it bumps the schema cookie, which makes every other
-- connection re-read it too. The INSERT further down is the proof: it fails,
-- and with it the whole migration, unless the new constraint is in force.
PRAGMA writable_schema = ON;
UPDATE sqlite_schema
SET sql = replace(sql, '''folder'', ''document'')', '''folder'', ''document'', ''file'')')
WHERE type = 'table' AND name = 'nodes';
PRAGMA writable_schema = RESET;

CREATE INDEX idx_nodes_media ON nodes (media_id) WHERE media_id IS NOT NULL;

-- An import used to hang a file that no document linked to on the folder it
-- arrived in, where only the media browser could see it. Each of those becomes
-- a file node in that folder. The slug cannot be derived from the name in SQL,
-- so it is the start of the media id: unique, and renaming fixes it for good.
INSERT INTO nodes (parent_id, kind, name, slug, created_at, updated_at, media_id)
SELECT nm.node_id, 'file', m.filename, 'file-' || substr(m.id, 1, 8),
       m.created_at, m.created_at, m.id
FROM node_media nm
JOIN nodes n ON n.id = nm.node_id
JOIN media m ON m.id = nm.media_id
WHERE n.kind IN ('space', 'folder');

-- The file node owns its bytes the same way a document owns what it embeds,
-- so deleting a folder still finds every file that goes with it.
DELETE FROM node_media
WHERE node_id IN (SELECT id FROM nodes WHERE kind IN ('space', 'folder'));

INSERT INTO node_media (node_id, media_id)
SELECT id, media_id FROM nodes WHERE kind = 'file';
