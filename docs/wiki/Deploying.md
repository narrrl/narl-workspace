# Deploying

The workspace is one binary with the frontend inside, and one data directory.
It needs no database server and no runtime on the host.

## On a server

1. Build it: `make build` makes `target/release/narl-workspace`.
2. Copy the binary and a `.env` to the server. Set `WORKSPACE_USER`,
   `WORKSPACE_PASSWORD` and `WORKSPACE_SECRET`; the server will not start with
   the examples from `.env.example`.
3. Put a reverse proxy that terminates TLS in front of `WORKSPACE_BIND`, and
   set `WORKSPACE_SECURE_COOKIE=1`.
4. Keep `WORKSPACE_DATA_DIR` safe. It is the whole state: the database and the
   uploads.

The proxy must:

- pass the `Host` header through unchanged, because a write that a browser says
  came from another origin is refused;
- allow request bodies as large as `WORKSPACE_MAX_UPLOAD_MB`;
- not weaken the security headers the app sends (`Content-Security-Policy`,
  `Referrer-Policy: no-referrer` and the rest).

`GET /api/health` answers without a login, for a supervisor. `SIGTERM` gives
requests in flight eight seconds to finish.

## Docker

```sh
cp .env.example .env   # set WORKSPACE_USER, WORKSPACE_PASSWORD, WORKSPACE_SECRET
docker compose up -d --build
```

The image builds the frontend with bun and the binary with cargo, then ships
only the binary on `debian:trixie-slim`, running as an unprivileged user. The
database and uploads live in the `diary-data` volume. Keep
`COMPOSE_PROJECT_NAME=narl-diary` if you rename the directory, or compose looks
for the volume under a new name and finds an empty one.

The port is published on `127.0.0.1:4242` only. To serve it through traefik,
copy `compose.override.yml.example` to `compose.override.yml` and set the host.
It joins the external `narl` network, drops the published port, and sets
`WORKSPACE_SECURE_COOKIE=1`.

## Sessions and security

- Every signed-in browser is a row on the server. `:sessions` lists them,
  `:sessions rm <n>` signs one out and locks the vault, and `:logout!` signs
  out every one.
- Changing `WORKSPACE_SECRET` or `WORKSPACE_PASSWORD` also signs everyone out.
- Wrong passwords are answered more slowly each time, up to 30 seconds. A
  browser that has logged in before is slowed only by its own mistakes.
- With `WORKSPACE_SECURE_COOKIE=1` the cookie is `Secure` and `__Host-`
  prefixed, and every answer carries HSTS for a year.
- Uploads are only served as types that cannot run in the browser.

See [Configuration](Configuration.md) for every setting.
