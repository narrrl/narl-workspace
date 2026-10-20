//! Capture tokens: what a phone shortcut or a script sends to
//! `POST /api/capture` instead of a session. See [`crate::auth::Capturer`].

use axum::{
    extract::{Path, State},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    auth::{token_hash, Session},
    error::{AppError, AppResult},
    state::AppState,
};

/// Put in front of every token, so one pasted somewhere it should not be is
/// recognisable for what it is.
const PREFIX: &str = "nwc_";

#[derive(Serialize, sqlx::FromRow)]
pub struct TokenSummary {
    pub id: i64,
    pub name: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

#[derive(Serialize)]
pub struct NewToken {
    #[serde(flatten)]
    pub summary: TokenSummary,
    /// The token itself, here and nowhere else: only its hash is kept.
    pub token: String,
}

#[derive(Deserialize)]
pub struct AddInput {
    pub name: String,
}

fn new_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("{PREFIX}{}", B64.encode(bytes))
}

pub async fn list(_: Session, State(state): State<AppState>) -> AppResult<Json<Vec<TokenSummary>>> {
    let tokens = sqlx::query_as(
        "SELECT id, name, created_at, last_used_at FROM api_tokens ORDER BY created_at, id",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(tokens))
}

pub async fn add(
    _: Session,
    State(state): State<AppState>,
    Json(input): Json<AddInput>,
) -> AppResult<Json<NewToken>> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("a token needs a name".into()));
    }
    let token = new_token();
    let now = crate::now();
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO api_tokens (name, hash, created_at) VALUES (?1, ?2, ?3) RETURNING id",
    )
    .bind(name)
    .bind(token_hash(&token))
    .bind(now)
    .fetch_one(&state.db)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            AppError::BadRequest(format!("there is a token called {name} already"))
        }
        e => e.into(),
    })?;
    tracing::info!(token = id, "capture token added");
    state.backup.signal();
    Ok(Json(NewToken {
        summary: TokenSummary {
            id,
            name: name.to_string(),
            created_at: now,
            last_used_at: None,
        },
        token,
    }))
}

/// Refused from the next request on: every capture looks the hash up again.
pub async fn remove(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<serde_json::Value>> {
    let affected = sqlx::query("DELETE FROM api_tokens WHERE id = ?1")
        .bind(id)
        .execute(&state.db)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound);
    }
    tracing::info!(token = id, "capture token removed");
    state.backup.signal();
    Ok(Json(json!({ "ok": true })))
}
