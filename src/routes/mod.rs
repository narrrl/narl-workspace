pub mod backup;
pub mod boards;
pub mod export;
pub mod import;
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
        .route("/nodes/{id}/move", post(nodes::move_node))
        .route("/import", post(import::import_root).layer(uploads))
        .route("/nodes/{id}/import", post(import::import).layer(uploads))
        .route("/nodes/{id}/files", post(media::upload_into).layer(uploads))
        .route("/nodes/{id}/encrypt", post(seal::encrypt))
        .route("/nodes/{id}/decrypt", post(seal::decrypt))
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
