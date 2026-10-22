-- A file belongs to a node. A picture pasted into a document used to be a row
-- of `media` and nothing more: the document embedded it, but only the media
-- browser could list it, and it did not move, seal or go to the trash with
-- the document. Now an upload while writing is a file node of the document,
-- and each of the old ones becomes one too, under the document that embeds
-- it. The media id stays, so no `/api/media/<id>` in any body changes.
--
-- A file embedded in several documents goes to one of them, a document that
-- is not in the trash before one that is, and the oldest of those. The slug
-- cannot be derived from the name in SQL, so, as in 0010, it is the start of
-- the media id: unique, and renaming the file fixes it for good.
WITH RECURSIVE trashed (id) AS (
    SELECT id FROM nodes WHERE role = 'trash'
    UNION ALL
    SELECT n.id FROM nodes n JOIN trashed t ON n.parent_id = t.id
),
owners (media_id, node_id, rank) AS (
    SELECT nm.media_id, nm.node_id,
           ROW_NUMBER() OVER (
               PARTITION BY nm.media_id
               ORDER BY nm.node_id IN (SELECT id FROM trashed), nm.node_id
           )
    FROM node_media nm
    JOIN nodes d ON d.id = nm.node_id
    WHERE d.kind = 'document'
)
INSERT INTO nodes (parent_id, kind, name, slug, created_at, updated_at, media_id)
SELECT o.node_id, 'file', m.filename, 'file-' || substr(m.id, 1, 8),
       m.created_at, m.created_at, m.id
FROM media m
JOIN owners o ON o.media_id = m.id AND o.rank = 1
WHERE NOT EXISTS (SELECT 1 FROM nodes f WHERE f.media_id = m.id);

-- A file node owns its bytes the way a document owns what it embeds, so
-- purging the document still finds every file that goes with it.
INSERT OR IGNORE INTO node_media (node_id, media_id)
SELECT id, media_id FROM nodes WHERE kind = 'file';

-- A document is not sealed itself, but the folder it is in decides for its
-- files, the same as for a file beside it. The triggers of 0015 asked only
-- the parent, which for a document's file is the document.
DROP TRIGGER nodes_sealed_insert;
DROP TRIGGER nodes_sealed_update;

CREATE TRIGGER nodes_sealed_insert BEFORE INSERT ON nodes
WHEN new.kind = 'file'
    AND (SELECT CASE WHEN p.kind = 'document'
                     THEN (SELECT g.sealed FROM nodes g WHERE g.id = p.parent_id)
                     ELSE p.sealed END
         FROM nodes p WHERE p.id = new.parent_id) = 1
    AND (SELECT encrypted FROM media WHERE id = new.media_id) = 0
BEGIN
    SELECT RAISE(ABORT, 'sealed folder: a file here must be encrypted');
END;

CREATE TRIGGER nodes_sealed_update BEFORE UPDATE OF parent_id, media_id ON nodes
WHEN new.kind = 'file'
    AND (SELECT CASE WHEN p.kind = 'document'
                     THEN (SELECT g.sealed FROM nodes g WHERE g.id = p.parent_id)
                     ELSE p.sealed END
         FROM nodes p WHERE p.id = new.parent_id) = 1
    AND (SELECT encrypted FROM media WHERE id = new.media_id) = 0
BEGIN
    SELECT RAISE(ABORT, 'sealed folder: a file here must be encrypted');
END;
