pub mod backup;
pub mod boards;
pub mod export;
pub mod import;
pub mod inspect;
pub mod manifest;
pub mod media;
pub mod nodes;
pub mod seal;
pub mod session;
pub mod share;
pub mod ssh_keys;
pub mod vault;

use axum::{
    extract::DefaultBodyLimit,
    routing::{delete, get, post, put},
    Router,
};

use crate::state::AppState;

/// Anything that is not an upload: a document body, a card, a password.
/// The size of the largest document anyone would write by hand, many times
/// over, and the same ceiling the SFTP mount puts on one.
const JSON_LIMIT: usize = 16 * 1024 * 1024;

/// The only body read before anyone has proved who they are.
const LOGIN_LIMIT: usize = 16 * 1024;

/// `max_upload` applies to the routes that take files and to nothing else, so
/// the upload ceiling is not also how much a stranger may make the server read.
pub fn api_router(max_upload: usize) -> Router<AppState> {
    // Multipart framing on top of the files themselves.
    let uploads = DefaultBodyLimit::max(max_upload.saturating_add(1024 * 1024));
    Router::new()
        .route("/health", get(session::health))
        .route("/me", get(session::me))
        .route(
            "/login",
            post(session::login).layer(DefaultBodyLimit::max(LOGIN_LIMIT)),
        )
        .route("/logout", post(session::logout))
        .route("/export", get(export::export))
        .route("/backup", get(backup::status).post(backup::run))
        .route("/spaces", get(nodes::spaces).post(nodes::create_space))
        .route("/spaces/overview", get(nodes::overview))
        .route("/agenda", get(boards::agenda))
        .route("/spaces/{id}/board", get(boards::board))
        .route("/spaces/{id}/lists", post(boards::create_list))
        .route(
            "/lists/{id}",
            put(boards::rename_list).delete(boards::remove_list),
        )
        .route("/lists/{id}/cards", post(boards::create_card))
        .route(
            "/cards/{id}",
            put(boards::update_card).delete(boards::remove_card),
        )
        .route("/cards/{id}/move", post(boards::move_card))
        .route("/nodes", get(nodes::list).post(nodes::create))
        .route("/resolve", get(nodes::resolve))
        .route("/nodes/{id}", get(nodes::get))
        .route("/nodes/{id}", put(nodes::update))
        .route("/nodes/{id}", delete(nodes::remove))
        .route("/nodes/move", post(nodes::move_many))
        .route("/nodes/{id}/move", post(nodes::move_node))
        .route("/import", post(import::import_root).layer(uploads))
        .route("/nodes/{id}/import", post(import::import).layer(uploads))
        .route("/nodes/{id}/files", post(media::upload_into).layer(uploads))
        .route("/nodes/{id}/encrypt", post(seal::encrypt))
        .route("/nodes/{id}/decrypt", post(seal::decrypt))
        .route("/nodes/{id}/inspect", post(inspect::inspect))
        .route(
            "/vault",
            get(vault::status)
                .post(vault::setup)
                .put(vault::change_password),
        )
        .route("/vault/unlock", post(vault::unlock))
        .route("/vault/lock", post(vault::lock))
        .route("/nodes/{id}/share", post(share::enable))
        .route("/nodes/{id}/share", delete(share::disable))
        .route(
            "/media",
            post(media::upload).layer(uploads).get(media::list),
        )
        .route("/media/{id}", get(media::serve).delete(media::remove))
        .route("/share/{token}/{key}", get(share::read))
        .route("/share/{token}/{key}/media/{id}", get(share::serve_media))
        .route("/ssh-keys", get(ssh_keys::list).post(ssh_keys::add))
        .route("/ssh-keys/{id}", delete(ssh_keys::remove))
        .layer(DefaultBodyLimit::max(JSON_LIMIT))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::{atomic::AtomicI64, Arc};

    use axum::{body::Body, http::Request};
    use regex::Regex;
    use tower::ServiceExt;

    use crate::{backup::Backup, config::Config, notify::Notifier, state::AppState, vault::Vault};

    /// The routes that answer someone who has not logged in: to log in, to
    /// say the server is up, and the share links, which carry their own key.
    const PUBLIC: &[&str] = &[
        "/health",
        "/login",
        "/logout",
        "/share/{token}/{key}",
        "/share/{token}/{key}/media/{id}",
    ];

    async fn state() -> AppState {
        state_in(".".into()).await
    }

    /// A whole `AppState` on an in-memory database, with its uploads under
    /// `data_dir`.
    pub(crate) async fn state_in(data_dir: std::path::PathBuf) -> AppState {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&db).await.unwrap();
        let config = Arc::new(Config {
            data_dir,
            ..Config::for_tests(30)
        });
        AppState {
            vault: Vault::new(db.clone(), config.vault_idle),
            backup: Backup::unavailable(db.clone(), Arc::clone(&config), "tests".into()),
            notify: Notifier::new(db.clone(), Arc::clone(&config)).unwrap(),
            login_throttle: Arc::default(),
            vault_throttle: Arc::default(),
            epoch: Arc::new(AtomicI64::new(0)),
            config,
            db,
        }
    }

    /// Every route this file registers, read from the file itself, so a
    /// route added later is covered without anyone remembering to.
    fn routes() -> Vec<(String, String)> {
        let path = Regex::new(r#"^\s*"([^"]+)""#).unwrap();
        let method = Regex::new(r"\b(get|post|put|delete)\(").unwrap();
        let source = include_str!("mod.rs");
        let source = &source[..source.find("#[cfg(test)]").unwrap()];
        let mut found = Vec::new();
        // `.route(...)` up to its matching bracket, however rustfmt wrapped it.
        for (at, _) in source.match_indices(".route(") {
            let rest = &source[at + ".route(".len()..];
            let mut depth = 1;
            let close = rest
                .char_indices()
                .find(|&(_, c)| {
                    depth += match c {
                        '(' => 1,
                        ')' => -1,
                        _ => 0,
                    };
                    depth == 0
                })
                .map(|(i, _)| i)
                .unwrap();
            let call = &rest[..close];
            let Some(caps) = path.captures(call) else {
                continue;
            };
            for m in method.captures_iter(call) {
                found.push((m[1].to_uppercase(), caps[1].to_string()));
            }
        }
        found
    }

    #[tokio::test]
    async fn every_route_but_the_public_ones_wants_a_session() {
        let routes = routes();
        assert!(
            routes.len() > 40,
            "the route table was not read: {routes:?}"
        );
        let app = super::api_router(1024 * 1024).with_state(state().await);

        for (method, path) in routes {
            let uri = path
                .replace("{id}", "1")
                .replace("{token}", "t")
                .replace("{key}", "k");
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method.as_str())
                        .uri(&uri)
                        .header("content-type", "application/json")
                        .body(Body::from("{}"))
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status().as_u16();
            if PUBLIC.contains(&path.as_str()) {
                assert_ne!(status, 401, "{method} {path} is public and answered 401");
            } else {
                assert_eq!(
                    status, 401,
                    "{method} {path} answered {status} without a session"
                );
            }
        }
    }
}
