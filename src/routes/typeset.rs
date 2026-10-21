//! `POST /api/typst`: a ```typst block of a document, typeset into SVG.

use std::sync::LazyLock;

use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use crate::{
    auth::Session,
    error::{AppError, AppResult},
    state::AppState,
    typeset,
};

#[derive(Deserialize)]
pub struct TypesetRequest {
    pub source: String,
}

/// The block as SVG, or what typst said about it. A block that does not
/// compile is still an answer: the page shows the error where the block is.
#[derive(Serialize, Debug)]
pub struct Typeset {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub svg: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A page with many blocks asks for all of them at once; two are laid out at a
/// time, so they do not take every core from everyone else.
static AT_ONCE: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(2));

pub async fn render(
    _: Session,
    State(_): State<AppState>,
    Json(body): Json<TypesetRequest>,
) -> AppResult<Json<Typeset>> {
    answer(body.source).await.map(Json)
}

/// Typeset one block, off the async threads.
pub(crate) async fn answer(source: String) -> AppResult<Typeset> {
    if source.len() > typeset::MAX_SOURCE {
        return Err(AppError::BadRequest(format!(
            "a typst block is at most {} KiB",
            typeset::MAX_SOURCE / 1024
        )));
    }
    let _turn = AT_ONCE
        .acquire()
        .await
        .map_err(|e| AppError::Internal(e.into()))?;
    let done = tokio::task::spawn_blocking(move || typeset::render(&source))
        .await
        .map_err(|e| AppError::Internal(e.into()))?;
    Ok(match done {
        Ok(svg) => Typeset {
            svg: Some(svg),
            error: None,
        },
        Err(error) => Typeset {
            svg: None,
            error: Some(error),
        },
    })
}
