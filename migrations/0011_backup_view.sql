-- Bookkeeping for `files/` on the device: the file nodes laid out as the tree
-- shows them, so Proton Drive's own apps can open them by name.
--
-- One row per file node mirrored there. No foreign key to `nodes`: the row has
-- to outlive the node, because it is how the mirror knows what to trash once
-- the node is gone. `media_id` is what was uploaded; a node whose bytes change
-- is uploaded again.
CREATE TABLE backup_view (
    node_id  INTEGER PRIMARY KEY,
    path     TEXT NOT NULL,
    media_id TEXT NOT NULL,
    node_uid TEXT NOT NULL
);

-- Every folder made under `files/`, by its path there, so one that ends up
-- empty can be trashed and the rest are not looked up again on every run.
CREATE TABLE backup_view_dirs (
    path     TEXT PRIMARY KEY,
    node_uid TEXT NOT NULL
) WITHOUT ROWID;
