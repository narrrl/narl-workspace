//! Looking at the Proton Drive mirror, asking it to run now, and logging it
//! in to Proton or out again.

use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::{
    auth::Session,
    backup::{proton::DISCLOSURE, LoginStep, Status},
    error::{AppError, AppResult},
    state::AppState,
};

#[derive(Serialize)]
pub struct Shown {
    #[serde(flatten)]
    pub status: Status,
    /// What Proton asks a third-party client to say before it takes a login.
    pub disclosure: &'static str,
    /// Whether a stored session is sealed under a key file off the data volume.
    pub sealed_session: bool,
}

pub async fn status(_: Session, State(state): State<AppState>) -> AppResult<Json<Shown>> {
    Ok(Json(Shown {
        status: state.backup.status(),
        disclosure: DISCLOSURE,
        sealed_session: state.backup.session_sealed(),
    }))
}

/// Mirror now rather than when the workspace next falls quiet. Answers with the
/// status the run produced, so `:backup` can report what it did.
pub async fn run(_: Session, State(state): State<AppState>) -> AppResult<Json<Status>> {
    state.backup.run_now().await?;
    Ok(Json(state.backup.status()))
}

#[derive(Deserialize)]
pub struct LoginInput {
    pub username: String,
    pub password: Zeroizing<String>,
}

#[derive(Deserialize)]
pub struct CodeInput {
    pub code: String,
}

#[derive(Deserialize)]
pub struct TokenInput {
    pub token: String,
}

#[derive(Serialize)]
pub struct Answer {
    #[serde(flatten)]
    pub step: LoginStep,
    pub status: Status,
}

/// Every step of a login goes through its own throttle, so the workspace
/// cannot be made to hammer Proton into locking the account. Proton's reasons
/// for a refusal are passed on as they are; none of them holds a secret.
async fn step(
    state: &AppState,
    run: impl std::future::Future<Output = anyhow::Result<LoginStep>>,
) -> AppResult<Json<Answer>> {
    let attempt = state.proton_throttle.begin().map_err(|wait| {
        tracing::warn!("Proton login refused, {wait}s of backoff remaining");
        AppError::TooManyRequests(wait)
    })?;
    match run.await {
        Ok(step) => {
            attempt.succeeded();
            Ok(Json(Answer {
                step,
                status: state.backup.status(),
            }))
        }
        Err(e) => {
            attempt.failed();
            tracing::warn!("Proton login: {e:#}");
            Err(AppError::BadRequest(format!("{e:#}")))
        }
    }
}

/// `POST /api/backup/login` — start a login with the account's password.
pub async fn login(
    _: Session,
    State(state): State<AppState>,
    Json(input): Json<LoginInput>,
) -> AppResult<Json<Answer>> {
    if input.username.trim().is_empty() {
        return Err(AppError::BadRequest("a Proton username is needed".into()));
    }
    step(&state, state.backup.login(&input.username, input.password)).await
}

/// `POST /api/backup/login/totp` — the code a waiting login asked for.
pub async fn totp(
    _: Session,
    State(state): State<AppState>,
    Json(input): Json<CodeInput>,
) -> AppResult<Json<Answer>> {
    step(&state, state.backup.login_totp(&input.code)).await
}

/// `POST /api/backup/login/captcha` — the token Proton's human check gave.
pub async fn captcha(
    _: Session,
    State(state): State<AppState>,
    Json(input): Json<TokenInput>,
) -> AppResult<Json<Answer>> {
    step(&state, state.backup.login_verified(&input.token)).await
}

/// `DELETE /api/backup/login` — forget the Proton session.
pub async fn logout(_: Session, State(state): State<AppState>) -> AppResult<Json<Status>> {
    state.backup.logout().await?;
    Ok(Json(state.backup.status()))
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{header, Request, StatusCode},
    };
    use tower::ServiceExt;

    #[tokio::test]
    async fn a_code_with_no_login_waiting_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::routes::tests::state_in(dir.path().into()).await;
        crate::auth::Epoch::load(&state.db, &state.config)
            .await
            .unwrap();
        let app = crate::routes::api_router(1024).with_state(state);
        let login = app
            .clone()
            .oneshot(
                Request::post("/login")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"username":"narl","password":"hunter2"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let cookie = login.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        let answer = app
            .oneshot(
                Request::post("/backup/login/totp")
                    .header("content-type", "application/json")
                    .header(header::COOKIE, cookie)
                    .body(Body::from(r#"{"code":"123456"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(answer.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(answer.into_body(), usize::MAX)
            .await
            .unwrap();
        // A server built for tests has backups off; that is said first.
        let text = String::from_utf8_lossy(&body);
        assert!(
            text.contains("backups are off") || text.contains("no Proton login is waiting"),
            "{text}"
        );
    }
}
