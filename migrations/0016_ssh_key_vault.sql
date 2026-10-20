-- Whether a key may read sealed files over the mount while the vault is open.
-- A key is a standing credential kept on some other machine, so it starts
-- without: the owner gives it in the browser, key by key.
ALTER TABLE ssh_keys ADD COLUMN vault INTEGER NOT NULL DEFAULT 0;
