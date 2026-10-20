use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;

use crate::{
    auth::{self, Session},
    error::{AppError, AppResult},
    state::AppState,
    throttle::FREE_ATTEMPTS,
};

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<LoginRequest>,
) -> AppResult<impl IntoResponse> {
    let attempt = state.login_throttle.begin().map_err(|wait| {
        tracing::warn!("login refused, {wait}s of backoff remaining");
        AppError::TooManyRequests(wait)
    })?;

    if !auth::credentials_match(&state.config, &body.username, &body.password) {
        let failures = attempt.failed();
        // Worth a mail once someone is past guessing and into trying.
        if failures > FREE_ATTEMPTS {
            state.notify.note_failed_logins(failures).await;
        }
        return Err(AppError::Unauthorized);
    }

    attempt.succeeded();
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");
    state.notify.note_login(user_agent).await;

    let token = auth::issue_token(
        &state.config,
        state.epoch.load(std::sync::atomic::Ordering::SeqCst),
    );
    Ok((
        StatusCode::OK,
        [(
            header::SET_COOKIE,
            auth::session_cookie(&state.config, &token),
        )],
        Json(json!({ "user": state.config.username })),
    ))
}

#[derive(Deserialize)]
pub struct LogoutQuery {
    /// Every device, not just this one: the session epoch moves on.
    #[serde(default)]
    pub everywhere: bool,
}

pub async fn logout(
    session: Option<Session>,
    State(state): State<AppState>,
    Query(query): Query<LogoutQuery>,
) -> AppResult<impl IntoResponse> {
    // Leaving is the moment the vault should not outlive — but only the owner
    // leaving: anyone may clear a cookie they do not have, and that must not
    // be a way to shut the vault on the one who does.
    if session.is_some() {
        state.vault.lock();
        if query.everywhere {
            auth::Epoch::advance(&state).await?;
            tracing::info!("signed out everywhere");
        }
    }
    Ok((
        StatusCode::OK,
        [(header::SET_COOKIE, auth::clear_cookie(&state.config))],
        Json(json!({ "ok": true })),
    ))
}

pub async fn me(_: Session, State(state): State<AppState>) -> impl IntoResponse {
    Json(json!({ "user": state.config.username }))
}

/// `GET /api/health` — for whatever supervises the process. Answers without a
/// session and says nothing about the workspace, only that the server is up
/// and its database answers.
pub async fn health(State(state): State<AppState>) -> impl IntoResponse {
    match sqlx::query("SELECT 1").execute(&state.db).await {
        Ok(_) => (StatusCode::OK, Json(json!({ "ok": true }))),
        Err(e) => {
            tracing::error!("health check: the database does not answer: {e}");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "ok": false })),
            )
        }
    }
}
