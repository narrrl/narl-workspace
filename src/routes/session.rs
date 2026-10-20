use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{AppendHeaders, IntoResponse},
    Json,
};
use serde::{Deserialize, Serialize};
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
    let device = auth::device_id(&state.config, &headers);
    let throttle = state.login_throttle.for_device(device.as_deref());
    let attempt = throttle.begin().map_err(|wait| {
        tracing::warn!(
            known_device = device.is_some(),
            "login refused, {wait}s of backoff remaining"
        );
        AppError::TooManyRequests(wait)
    })?;

    if !auth::credentials_match(&state.config, &body.username, &body.password) {
        let failures = attempt.failed();
        // Worth a mail once someone is past guessing and into trying.
        if failures > FREE_ATTEMPTS {
            state
                .notify
                .note_failed_logins(failures, device.is_some())
                .await;
        }
        return Err(AppError::Unauthorized);
    }

    attempt.succeeded();
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");
    state.notify.note_login(user_agent).await;

    let expires = auth::session_expiry(&state.config);
    let id = state.sessions.open(&state.db, user_agent, expires).await?;
    let token = auth::issue_token(
        expires,
        state.epoch.load(std::sync::atomic::Ordering::SeqCst),
        &id,
        &state.config,
    );
    // Signed again on every login, so a browser in use never runs out of it.
    let device = device.unwrap_or_else(auth::new_device_id);
    Ok((
        StatusCode::OK,
        AppendHeaders([
            (
                header::SET_COOKIE,
                auth::session_cookie(&state.config, &token),
            ),
            (
                header::SET_COOKIE,
                auth::device_cookie(&state.config, &device),
            ),
        ]),
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
    if let Some(session) = session {
        state.vault.lock();
        state.sessions.end(&state.db, &session.id).await?;
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

#[derive(Serialize, sqlx::FromRow)]
pub struct SessionSummary {
    pub id: String,
    pub created_at: i64,
    pub last_seen: i64,
    pub user_agent: String,
    /// The session asking.
    #[sqlx(skip)]
    pub current: bool,
}

/// `GET /api/sessions` — every browser signed in, the one seen last first.
pub async fn list(
    session: Session,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<SessionSummary>>> {
    let mut sessions: Vec<SessionSummary> = sqlx::query_as(
        "SELECT id, created_at, last_seen, user_agent FROM sessions WHERE expires_at > ?1",
    )
    .bind(crate::now())
    .fetch_all(&state.db)
    .await?;
    for summary in &mut sessions {
        summary.current = summary.id == session.id;
        if let Some(seen) = state.sessions.last_seen(&summary.id) {
            summary.last_seen = summary.last_seen.max(seen);
        }
    }
    sessions.sort_by_key(|summary| std::cmp::Reverse(summary.last_seen));
    Ok(Json(sessions))
}

/// `DELETE /api/sessions/{id}` — sign one other browser out. The vault is
/// locked as well: it is open for the whole workspace, and the browser just
/// signed out may be the one that opened it.
pub async fn revoke(
    session: Session,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    if id == session.id {
        return Err(AppError::BadRequest(
            "that is the session you are using — log out to end it".into(),
        ));
    }
    if !state.sessions.end(&state.db, &id).await? {
        return Err(AppError::NotFound);
    }
    state.vault.lock();
    tracing::info!("a session was signed out; the vault is locked");
    Ok(Json(json!({ "ok": true })))
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

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{header, Request, StatusCode},
        Router,
    };
    use tower::ServiceExt;

    async fn app() -> Router {
        let dir = std::env::temp_dir().join(format!("sessions-{}", uuid::Uuid::new_v4()));
        let state = crate::routes::tests::state_in(dir).await;
        crate::auth::Epoch::load(&state.db, &state.config)
            .await
            .unwrap();
        crate::routes::api_router(1024).with_state(state)
    }

    async fn call(
        app: &Router,
        method: &str,
        uri: &str,
        cookie: Option<&str>,
    ) -> (StatusCode, String) {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(cookie) = cookie {
            request = request.header(header::COOKIE, cookie);
        }
        let body = if uri == "/login" {
            r#"{"username":"narl","password":"hunter2"}"#
        } else {
            "{}"
        };
        let response = app
            .clone()
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
            .unwrap_or_default()
            .to_string();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        if uri == "/login" {
            (status, cookie)
        } else {
            (status, String::from_utf8_lossy(&body).into_owned())
        }
    }

    #[tokio::test]
    async fn a_signed_out_session_is_refused_while_the_others_carry_on() {
        let app = app().await;
        let (_, phone) = call(&app, "POST", "/login", None).await;
        let (_, laptop) = call(&app, "POST", "/login", None).await;
        assert_ne!(phone, laptop);

        let (status, listed) = call(&app, "GET", "/sessions", Some(&laptop)).await;
        assert_eq!(status, StatusCode::OK);
        let listed: Vec<serde_json::Value> = serde_json::from_str(&listed).unwrap();
        assert_eq!(listed.len(), 2);
        let mine = listed.iter().find(|s| s["current"] == true).unwrap();
        let other = listed.iter().find(|s| s["current"] == false).unwrap();

        // Not this way for the session asking.
        let own = format!("/sessions/{}", mine["id"].as_str().unwrap());
        assert_eq!(
            call(&app, "DELETE", &own, Some(&laptop)).await.0,
            StatusCode::BAD_REQUEST
        );

        let other = format!("/sessions/{}", other["id"].as_str().unwrap());
        assert_eq!(
            call(&app, "DELETE", &other, Some(&laptop)).await.0,
            StatusCode::OK
        );
        assert_eq!(
            call(&app, "GET", "/me", Some(&phone)).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call(&app, "GET", "/me", Some(&laptop)).await.0,
            StatusCode::OK
        );
        assert_eq!(
            call(&app, "DELETE", &other, Some(&laptop)).await.0,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn logging_out_everywhere_ends_every_session() {
        let app = app().await;
        let (_, phone) = call(&app, "POST", "/login", None).await;
        let (_, laptop) = call(&app, "POST", "/login", None).await;

        assert_eq!(
            call(&app, "POST", "/logout", Some(&phone)).await.0,
            StatusCode::OK
        );
        assert_eq!(
            call(&app, "GET", "/me", Some(&phone)).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            call(&app, "GET", "/me", Some(&laptop)).await.0,
            StatusCode::OK
        );

        let (_, tablet) = call(&app, "POST", "/login", None).await;
        let everywhere = "/logout?everywhere=true";
        assert_eq!(
            call(&app, "POST", everywhere, Some(&tablet)).await.0,
            StatusCode::OK
        );
        for cookie in [&laptop, &tablet] {
            assert_eq!(
                call(&app, "GET", "/me", Some(cookie)).await.0,
                StatusCode::UNAUTHORIZED
            );
        }
    }
}
