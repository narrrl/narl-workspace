use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

#[derive(Debug)]
pub enum AppError {
    Unauthorized,
    NotFound,
    BadRequest(String),
    /// The caller must wait this many seconds before trying again.
    TooManyRequests(i64),
    /// The file is sealed and the vault is closed.
    Locked,
    /// Written over something newer than the writer had seen.
    Conflict(String),
    Internal(anyhow::Error),
}

pub type AppResult<T> = Result<T, AppError>;

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized".to_string()),
            AppError::NotFound => (StatusCode::NOT_FOUND, "not found".to_string()),
            AppError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            AppError::Conflict(m) => (StatusCode::CONFLICT, m),
            AppError::Locked => (
                StatusCode::LOCKED,
                "the vault is locked — :unlock opens it".to_string(),
            ),
            AppError::TooManyRequests(wait) => (
                StatusCode::TOO_MANY_REQUESTS,
                format!("too many attempts — wait {wait}s"),
            ),
            AppError::Internal(e) => {
                tracing::error!("internal error: {e:#}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal error".to_string(),
                )
            }
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}

impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        let err: anyhow::Error = err.into();
        // The triggers of 0015 refuse plaintext in a sealed folder; that is
        // an answer for the caller, not a fault of the server.
        if let Some(sqlx::Error::Database(db)) = err.downcast_ref::<sqlx::Error>() {
            if let Some(reason) = db.message().strip_prefix("sealed folder: ") {
                return AppError::BadRequest(format!("{reason} — the folder is sealed"));
            }
        }
        AppError::Internal(err)
    }
}
