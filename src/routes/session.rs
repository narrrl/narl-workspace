use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;

use crate::{
    auth::{self, Session},
    throttle::FREE_ATTEMPTS,
    error::{AppError, AppResult},
    state::AppState,
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

    let token = auth::issue_token(&state.config);
    Ok((
        StatusCode::OK,
        [(
            header::SET_COOKIE,
            auth::session_cookie(&state.config, &token),
        )],
        Json(json!({ "user": state.config.username })),
    ))
}

pub async fn logout(session: Option<Session>, State(state): State<AppState>) -> impl IntoResponse {
    // Leaving is the moment the vault should not outlive — but only the owner
    // leaving: anyone may clear a cookie they do not have, and that must not
    // be a way to shut the vault on the one who does.
    if session.is_some() {
        state.vault.lock();
    }
    (
        StatusCode::OK,
        [(header::SET_COOKIE, auth::clear_cookie(&state.config))],
        Json(json!({ "ok": true })),
    )
}

pub async fn me(_: Session, State(state): State<AppState>) -> impl IntoResponse {
    Json(json!({ "user": state.config.username }))
}
