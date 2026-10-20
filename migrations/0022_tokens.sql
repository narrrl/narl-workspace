-- Tokens that may write into the workspace from outside a logged-in browser:
-- a phone shortcut, a script, `curl`. One per thing that captures, so losing
-- a phone means removing one row. A token opens `POST /api/capture` and
-- nothing else; the session cookie remains the only way to read anything.
CREATE TABLE api_tokens (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    name         TEXT    NOT NULL UNIQUE,
    -- SHA-256 of the token, base64url. The token itself is shown once, when
    -- it is made, and kept nowhere.
    hash         TEXT    NOT NULL UNIQUE,
    created_at   INTEGER NOT NULL,
    last_used_at INTEGER
);
