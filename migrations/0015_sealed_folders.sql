-- A sealed folder or space keeps what lands in it sealed: an upload, an
-- import, an SFTP write, a file moved in. The flag is on every folder of a
-- sealed subtree, not only on the one `:encrypt` was given, so "is this
-- place sealed?" is one column of the parent rather than a walk up the tree.
ALTER TABLE nodes ADD COLUMN sealed INTEGER NOT NULL DEFAULT 0;

-- A folder made inside a sealed one is sealed from the start, whatever made it.
CREATE TRIGGER nodes_sealed_inherit AFTER INSERT ON nodes
WHEN new.kind = 'folder' AND (SELECT sealed FROM nodes WHERE id = new.parent_id) = 1
BEGIN
    UPDATE nodes SET sealed = 1 WHERE id = new.id;
END;

-- The last word on it: whichever path a file takes into a sealed place, it
-- does not get there as plaintext. The code seals first and so never meets
-- these; a path that forgot to fails here instead of leaving the bytes open.
CREATE TRIGGER nodes_sealed_insert BEFORE INSERT ON nodes
WHEN new.kind = 'file'
    AND (SELECT sealed FROM nodes WHERE id = new.parent_id) = 1
    AND (SELECT encrypted FROM media WHERE id = new.media_id) = 0
BEGIN
    SELECT RAISE(ABORT, 'sealed folder: a file here must be encrypted');
END;

CREATE TRIGGER nodes_sealed_update BEFORE UPDATE OF parent_id, media_id ON nodes
WHEN new.kind = 'file'
    AND (SELECT sealed FROM nodes WHERE id = new.parent_id) = 1
    AND (SELECT encrypted FROM media WHERE id = new.media_id) = 0
BEGIN
    SELECT RAISE(ABORT, 'sealed folder: a file here must be encrypted');
END;

-- A folder moves into a sealed place only once it is sealed itself.
CREATE TRIGGER nodes_sealed_move BEFORE UPDATE OF parent_id ON nodes
WHEN new.kind = 'folder' AND new.sealed = 0
    AND (SELECT sealed FROM nodes WHERE id = new.parent_id) = 1
BEGIN
    SELECT RAISE(ABORT, 'sealed folder: a folder moved here must be sealed first');
END;
