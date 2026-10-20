mod auth;
mod backup;
mod cli;
mod config;
mod db;
mod error;
mod mail;
mod notify;
mod routes;
mod sftp;
mod state;
mod static_files;
mod sweep;
mod tasks;
mod throttle;
mod vault;

use std::sync::Arc;

use anyhow::{Context, Result};
use axum::http::{header, HeaderValue};
use axum::Router;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use tower_http::{
    compression::CompressionLayer,
    set_header::SetResponseHeaderLayer,
    trace::{DefaultOnResponse, TraceLayer},
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use crate::{config::Config, state::AppState};

/// Seconds since the Unix epoch. Every timestamp in the database uses this.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is after 1970")
        .as_secs() as i64
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::registry()
        // rPGP warns on every key it re-serialises differently to how Proton
        // wrote it — a cosmetic packet-header difference, once per unlock, that
        // otherwise buries the backup's own log lines. `WORKSPACE_LOG` overrides
        // all of this when something needs looking at.
        .with(
            EnvFilter::try_from_env("WORKSPACE_LOG")
                .or_else(|_| EnvFilter::try_from_env("DIARY_LOG"))
                .unwrap_or_else(|_| EnvFilter::new("info,pgp=error")),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Asked before anything is loaded: `--help` must answer on a host that has
    // not been configured yet, which is exactly where it gets asked.
    if matches!(
        std::env::args().nth(1).as_deref(),
        Some("help" | "--help" | "-h")
    ) {
        print!("{}", cli::USAGE);
        return Ok(());
    }

    let config = Config::from_env()?;
    tokio::fs::create_dir_all(config.uploads_dir())
        .await
        .with_context(|| format!("could not create {}", config.uploads_dir().display()))?;

    // Subcommands share the server's configuration and database, and only the
    // default (no argument) goes on to serve anything.
    match std::env::args().nth(1).as_deref() {
        None => {}
        Some("proton-login") => return cli::proton_login(&config).await,
        Some("proton-logout") => return cli::proton_logout(&config),
        Some("proton-status") => {
            let db = open_db(&config).await?;
            return cli::proton_status(&db, &config).await;
        }
        Some("mail-test") => {
            let db = open_db(&config).await?;
            return cli::mail_test(db, Arc::new(config)).await;
        }
        Some("mail-status") => {
            let db = open_db(&config).await?;
            return cli::mail_status(&db, &config).await;
        }
        Some("backup-now") => {
            let db = open_db(&config).await?;
            let summary = backup::run_once(db, Arc::new(config)).await?;
            println!(
                "Mirrored {} files ({} bytes), skipped {}.",
                summary.uploaded, summary.bytes, summary.skipped
            );
            return Ok(());
        }
        Some(other) => {
            anyhow::bail!("unknown command {other:?}\n\n{}", cli::USAGE);
        }
    }

    let db = open_db(&config).await?;

    let bind = config.bind;
    let max_upload = config.max_upload_bytes;
    let config = Arc::new(config);
    let backup = match backup::Backup::new(db.clone(), Arc::clone(&config)) {
        Ok(backup) => backup,
        Err(e) => {
            let reason = format!("{e:#}");
            tracing::error!("Proton Drive backups are off: {reason}");
            backup::Backup::unavailable(db.clone(), Arc::clone(&config), reason)
        }
    };
    backup::spawn(Arc::clone(&backup));

    let notify = notify::Notifier::new(db.clone(), Arc::clone(&config))?;
    notify::spawn(Arc::clone(&notify));

    sweep::spawn(db.clone(), config.uploads_dir(), config.trash_days);

    let epoch = auth::Epoch::load(&db, &config).await?;
    let sessions = Arc::new(auth::Sessions::load(&db).await?);
    let vault = vault::Vault::new(db.clone(), config.vault_idle);
    vault.spawn_idle_lock();
    let state = AppState {
        vault,
        db,
        config: Arc::clone(&config),
        login_throttle: Arc::default(),
        vault_throttle: Arc::default(),
        proton_throttle: Arc::default(),
        backup,
        notify,
        epoch: Arc::new(std::sync::atomic::AtomicI64::new(epoch)),
        sessions,
    };
    routes::seal::spawn_rekey(state.clone());

    if let Some(sftp_bind) = config.sftp.bind {
        let sftp_state = state.clone();
        tasks::supervise("the sftp server", move || {
            let state = sftp_state.clone();
            async move {
                // A port that is taken now may be free in a minute; the web
                // side carries on either way.
                while let Err(e) = sftp::serve(state.clone(), sftp_bind).await {
                    tracing::error!("sftp server stopped: {e:#}; trying again in a minute");
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                }
            }
        });
    }

    let db = state.db.clone();
    serve(bind, max_upload, state).await?;
    // Checkpoints the write-ahead log, so what was written is in diary.db
    // itself and not only beside it.
    db.close().await;
    Ok(())
}

/// Open the workspace database and bring it up to date. Both the server and the
/// subcommands need it, and both need the same pragmas.
async fn open_db(config: &Config) -> Result<sqlx::SqlitePool> {
    let db = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(
            SqliteConnectOptions::new()
                .filename(config.data_dir.join("diary.db"))
                .create_if_missing(true)
                .journal_mode(SqliteJournalMode::Wal)
                // The standard pairing for WAL: fsync at checkpoints rather
                // than on every commit. A crash can cost the last transaction,
                // never the database.
                .synchronous(SqliteSynchronous::Normal)
                .busy_timeout(std::time::Duration::from_secs(10))
                .foreign_keys(true),
        )
        .await
        .context("could not open the workspace database")?;

    let migrator = sqlx::migrate!("./migrations");
    snapshot_before_migrating(&db, &migrator, config).await?;
    migrator
        .run(&db)
        .await
        .context("could not apply database migrations")?;

    Ok(db)
}

/// A copy of the database as it was, taken before a migration changes it.
/// Migrations only go forward, and some of them reshape or drop what was
/// there; the copy is the way back if one goes wrong on real data. A fresh
/// database, or one already up to date, gets none.
async fn snapshot_before_migrating(
    db: &sqlx::SqlitePool,
    migrator: &sqlx::migrate::Migrator,
    config: &Config,
) -> Result<()> {
    let applied: Option<i64> =
        match sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success")
            .fetch_one(db)
            .await
        {
            Ok(version) => version,
            // No bookkeeping table: nothing has ever been migrated, so there is
            // nothing to keep.
            Err(_) => return Ok(()),
        };
    let Some(applied) = applied else {
        return Ok(());
    };
    let latest = migrator.iter().map(|m| m.version).max().unwrap_or(0);
    if latest <= applied {
        return Ok(());
    }

    let path = config.data_dir.join(format!("diary.db.pre-{latest:04}"));
    if path.exists() {
        return Ok(());
    }
    tracing::info!(
        "database is at migration {applied}, the binary at {latest}: keeping a copy at {}",
        path.display()
    );
    sqlx::query("VACUUM INTO ?1")
        .bind(path.to_string_lossy().into_owned())
        .execute(db)
        .await
        .with_context(|| format!("could not snapshot the database to {}", path.display()))?;
    Ok(())
}

async fn serve(bind: std::net::SocketAddr, max_upload: usize, state: AppState) -> Result<()> {
    let app = Router::new()
        // What the API answers is private and changes; only what says
        // otherwise — the media routes — is kept by a cache.
        .nest(
            "/api",
            routes::api_router(max_upload).layer(SetResponseHeaderLayer::if_not_present(
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-store"),
            )),
        )
        .fallback(static_files::handler)
        .layer(axum::middleware::from_fn(auth::same_origin))
        .layer(axum::middleware::from_fn(deadline))
        .layer(CompressionLayer::new().compress_when(compressible))
        .layer(security_headers())
        .layer(strict_transport(state.config.secure_cookie))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &axum::http::Request<axum::body::Body>| {
                    tracing::info_span!(
                        "request",
                        method = %request.method(),
                        path = %loggable(request.uri().path()),
                    )
                })
                .on_request(())
                .on_response(DefaultOnResponse::new().level(tracing::Level::INFO)),
        )
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("could not bind {bind}"))?;

    tracing::info!("narl-workspace listening on http://{bind}");

    let server = axum::serve(listener, app).with_graceful_shutdown(async {
        shutdown_signal().await;
        tracing::info!("shutting down");
    });
    // Requests in flight get a few seconds to finish; an upload that would
    // take longer is cut off rather than holding the process past the point
    // where a supervisor stops asking and kills it.
    tokio::select! {
        result = server => result?,
        () = async {
            shutdown_signal().await;
            tokio::time::sleep(SHUTDOWN_GRACE).await;
        } => tracing::warn!("requests still open after {SHUTDOWN_GRACE:?}, closing anyway"),
    }
    Ok(())
}

/// How long an ordinary request may take to be answered. A request that takes
/// this long is stuck — waiting on a lock it will not get — and is better
/// answered with an error than left hanging.
const REQUEST_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);

/// Requests that take as long as the bytes do: uploads, imports, re-sealing,
/// a backup run, the export. Only the time to the first byte of a response is
/// bounded anyway, so a long download of a file is never cut off.
fn is_slow(method: &axum::http::Method, path: &str) -> bool {
    use axum::http::Method;
    match *method {
        Method::POST => {
            path == "/api/media"
                || path == "/api/import"
                || path == "/api/backup"
                // A login waits on Proton's SRP and key unlocking.
                || path.starts_with("/api/backup/login")
                || path.ends_with("/import")
                || path.ends_with("/files")
                || path.ends_with("/encrypt")
                || path.ends_with("/decrypt")
        }
        Method::GET => path == "/api/export",
        _ => false,
    }
}

async fn deadline(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if is_slow(request.method(), request.uri().path()) {
        return next.run(request).await;
    }
    let path = loggable(request.uri().path()).to_string();
    match tokio::time::timeout(REQUEST_DEADLINE, next.run(request)).await {
        Ok(response) => response,
        Err(_) => {
            tracing::error!("{path} took longer than {REQUEST_DEADLINE:?} and was given up on");
            (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                axum::Json(serde_json::json!({ "error": "the server took too long — try again" })),
            )
                .into_response()
        }
    }
}

/// Below the ten seconds `docker stop` waits before it sends SIGKILL.
const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(8);

/// A request path as it may go into the log: a share link's token is half of
/// its secret. The other half, the key, never travels in a path — it rides in
/// the URL fragment, which no server sees, and then in the body of the read
/// that trades it for a cookie. Query strings, which carry search terms, are
/// never logged at all.
fn loggable(path: &str) -> &str {
    if path.starts_with("/api/share/") {
        "/api/share/…"
    } else if path.starts_with("/s/") {
        "/s/…"
    } else {
        path
    }
}

/// Text is worth compressing; uploads are not. Media is already compressed,
/// and a compressed body no longer matches the byte ranges a video player
/// asks for, so nothing that answers a range — or could — is touched.
fn compressible(
    status: axum::http::StatusCode,
    _: axum::http::Version,
    headers: &axum::http::HeaderMap,
    _: &axum::http::Extensions,
) -> bool {
    if status == axum::http::StatusCode::PARTIAL_CONTENT
        || headers.contains_key(header::ACCEPT_RANGES)
    {
        return false;
    }
    let kind = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    kind.starts_with("text/")
        || kind.starts_with("application/json")
        || kind.starts_with("application/javascript")
        || kind.starts_with("image/svg+xml")
}

/// Content-Security-Policy for the app shell. `unsafe-inline` is granted to
/// styles only, because CodeMirror injects its theme as a <style> element at
/// runtime; scripts stay confined to the bundled files.
const CSP: &str = "default-src 'self'; img-src 'self' data:; media-src 'self'; \
    style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self'; \
    frame-src 'self' https://verify.proton.me; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";

type HeaderLayer = SetResponseHeaderLayer<HeaderValue>;

/// Headers every response carries. `if_not_present` throughout, so a handler
/// that has already said something stricter about itself — the media routes and
/// their sandbox CSP — keeps its own answer.
fn security_headers() -> (
    HeaderLayer,
    HeaderLayer,
    HeaderLayer,
    HeaderLayer,
    HeaderLayer,
    HeaderLayer,
    HeaderLayer,
) {
    let header = |name: header::HeaderName, value: &'static str| {
        SetResponseHeaderLayer::if_not_present(name, HeaderValue::from_static(value))
    };

    (
        // A share token lives in the URL path, so an outbound link from a shared
        // entry would otherwise hand the whole private link to a third party.
        header(header::REFERRER_POLICY, "no-referrer"),
        header(header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        header(header::X_FRAME_OPTIONS, "DENY"),
        header(header::CONTENT_SECURITY_POLICY, CSP),
        // No other site's window keeps a handle on this one, nor embeds what
        // it serves.
        header(
            header::HeaderName::from_static("cross-origin-opener-policy"),
            "same-origin",
        ),
        header(
            header::HeaderName::from_static("cross-origin-resource-policy"),
            "same-origin",
        ),
        header(
            header::HeaderName::from_static("permissions-policy"),
            "camera=(), microphone=(), geolocation=(), payment=(), usb=()",
        ),
    )
}

/// Over HTTPS, the browser is told to come back over nothing else for a year,
/// so a mistyped `http://` never carries the cookie in the clear.
fn strict_transport(
    secure: bool,
) -> SetResponseHeaderLayer<impl Fn(&axum::response::Response) -> Option<HeaderValue> + Clone> {
    SetResponseHeaderLayer::if_not_present(
        header::STRICT_TRANSPORT_SECURITY,
        move |_: &axum::response::Response| {
            secure.then(|| HeaderValue::from_static("max-age=31536000"))
        },
    )
}

/// Ctrl-C at a terminal, SIGTERM from anything that supervises the process —
/// docker, systemd. In a container this is PID 1, and the kernel gives PID 1 no
/// default action for SIGTERM: unless it is handled here, it is ignored.
async fn shutdown_signal() {
    let interrupt = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(e) => {
                tracing::warn!("cannot listen for SIGTERM: {e}");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = interrupt => {}
        () = terminate => {}
    }
}
