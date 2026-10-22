//! `/api/vault`: set the vault up, open it, close it, change its password.

use axum::{extract::State, Json};
use serde::Deserialize;

use crate::{
    auth::Session,
    error::{AppError, AppResult},
    state::AppState,
    throttle::Attempt,
    vault::{Password, Status, Unlock, MIN_PASSWORD},
};

#[derive(Deserialize)]
pub struct PasswordInput {
    pub password: Password,
}

#[derive(Deserialize)]
pub struct ChangeInput {
    pub current: Password,
    pub password: Password,
}

fn strong_enough(password: &str) -> AppResult<()> {
    if password.chars().count() < MIN_PASSWORD {
        return Err(AppError::BadRequest(format!(
            "a vault password needs at least {MIN_PASSWORD} characters"
        )));
    }
    Ok(())
}

pub async fn status(_: Session, State(state): State<AppState>) -> AppResult<Json<Status>> {
    Ok(Json(state.vault.status().await?))
}

/// `POST /api/vault` — make the vault. Only once.
pub async fn setup(
    _: Session,
    State(state): State<AppState>,
    Json(input): Json<PasswordInput>,
) -> AppResult<Json<Status>> {
    strong_enough(&input.password)?;
    if !state.vault.setup(input.password).await? {
        return Err(AppError::BadRequest(
            "there is a vault already — :vault passwd changes its password".into(),
        ));
    }
    tracing::info!("vault set up");
    state.backup.signal();
    Ok(Json(state.vault.status().await?))
}

/// Wrong guesses are slowed down exactly like logins. Argon2 already makes
/// each one cost a fraction of a second; this makes a run of them cost minutes.
fn throttle(state: &AppState) -> AppResult<Attempt<'_>> {
    state.vault_throttle.begin().map_err(|wait| {
        tracing::warn!("vault unlock refused, {wait}s of backoff remaining");
        AppError::TooManyRequests(wait)
    })
}

fn answer(attempt: Attempt<'_>, outcome: Unlock) -> AppResult<()> {
    match outcome {
        Unlock::Opened => {
            attempt.succeeded();
            Ok(())
        }
        Unlock::WrongPassword => {
            attempt.failed();
            Err(AppError::BadRequest("wrong vault password".into()))
        }
        Unlock::NotConfigured => Err(AppError::BadRequest(
            "there is no vault yet — :vault sets one up".into(),
        )),
        // The password was right; the vault is open, re-sealing goes on.
        Unlock::Rekeying => {
            attempt.succeeded();
            Err(AppError::Conflict(
                "files are still being re-sealed after the last password change — \
                 leave the vault open until :vault says it is done"
                    .into(),
            ))
        }
    }
}

pub async fn unlock(
    _: Session,
    State(state): State<AppState>,
    Json(input): Json<PasswordInput>,
) -> AppResult<Json<Status>> {
    let attempt = throttle(&state)?;
    answer(attempt, state.vault.unlock(input.password).await?)?;
    Ok(Json(state.vault.status().await?))
}

pub async fn lock(_: Session, State(state): State<AppState>) -> AppResult<Json<Status>> {
    state.vault.lock();
    Ok(Json(state.vault.status().await?))
}

pub async fn change_password(
    _: Session,
    State(state): State<AppState>,
    Json(input): Json<ChangeInput>,
) -> AppResult<Json<Status>> {
    strong_enough(&input.password)?;
    let attempt = throttle(&state)?;
    answer(
        attempt,
        state
            .vault
            .change_password(input.current, input.password)
            .await?,
    )?;
    let status = state.vault.status().await?;
    match status.rekey {
        Some(_) => {
            tracing::info!("vault password changed; re-sealing every file under the new key")
        }
        None => tracing::info!("vault password changed"),
    }
    state.backup.signal();
    Ok(Json(status))
}
