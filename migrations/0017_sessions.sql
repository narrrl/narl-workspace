-- Every signed-in browser gets a row, named by an id its cookie carries, so
-- one can be listed and signed out without signing out the rest. A cookie
-- whose row is gone is refused even though its signature still holds.
-- The epoch in `auth` stays as a second fence: raising it ends every row too.
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    created_at INTEGER NOT NULL,
    last_seen INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    user_agent TEXT NOT NULL DEFAULT ''
);
