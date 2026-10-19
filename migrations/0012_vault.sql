-- The vault: a password that is never stored, unlocking a master key that
-- seals files at rest. One row at most. The master key is kept here only
-- sealed under a key Argon2id derives from the password, with the costs it was
-- derived at, so raising them later does not lock an existing vault out.
CREATE TABLE vault (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    salt       BLOB    NOT NULL,
    m_cost     INTEGER NOT NULL,
    t_cost     INTEGER NOT NULL,
    p_cost     INTEGER NOT NULL,
    nonce      BLOB    NOT NULL,
    sealed     BLOB    NOT NULL,
    created_at INTEGER NOT NULL
);

-- A sealed file's bytes on disk are ciphertext; `size` stays the size of what
-- was uploaded, which is what a download of it is.
ALTER TABLE media ADD COLUMN encrypted INTEGER NOT NULL DEFAULT 0;

-- Remote copies the mirror has to take off Drive whatever
-- WORKSPACE_BACKUP_PRUNE says: the plaintext of a file that has since been
-- sealed. Keeping it would undo the sealing for anyone who reads the mirror.
CREATE TABLE backup_retired (
    path TEXT PRIMARY KEY
) WITHOUT ROWID;
