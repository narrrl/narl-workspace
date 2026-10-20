-- Sessions are signed cookies, not rows: nothing on the server lists them, so
-- nothing can revoke one. Signing a number into every cookie that has to match
-- this one gives a single lever instead — raise it, and every cookie issued
-- before is refused. It is raised by `:logout!` and whenever the login
-- password changes, which is recognised by a keyed hash of it kept here.
CREATE TABLE auth (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    epoch         INTEGER NOT NULL,
    password_hash TEXT    NOT NULL
);
