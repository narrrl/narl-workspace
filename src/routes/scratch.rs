//! The scratchpad: notes taken in passing, outside the tree.
//!
//! A note here has a body and nothing else — no space, no folder, no name —
//! because deciding those is the cost a quick note should not pay. The drawer
//! lists them newest first; one worth keeping is made into a document by the
//! client, which then deletes it here, so a note lives in one place at a time.

use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqliteConnection};

use crate::{
    auth::Session,
    error::{AppError, AppResult},
    routes::manifest::ScratchMeta,
    state::AppState,
};

/// A note is a few lines; this is far past that, and short of what would make
/// the drawer slow to load.
const MAX_BODY: usize = 256 * 1024;

#[derive(Serialize)]
pub struct Note {
    pub id: i64,
    pub body: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Deserialize)]
pub struct NoteInput {
    #[serde(default)]
    pub body: String,
}

fn checked(body: &str) -> AppResult<()> {
    if body.len() > MAX_BODY {
        return Err(AppError::BadRequest(
            "a scratch note is at most 256 KiB — make it a document".into(),
        ));
    }
    Ok(())
}

fn row_to_note(row: &sqlx::sqlite::SqliteRow) -> Note {
    Note {
        id: row.get("id"),
        body: row.get("body"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

/// Every note, the one touched last first.
pub async fn list(_: Session, State(state): State<AppState>) -> AppResult<Json<Vec<Note>>> {
    let rows = sqlx::query(
        "SELECT id, body, created_at, updated_at FROM scratch_notes
         ORDER BY updated_at DESC, id DESC",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.iter().map(row_to_note).collect()))
}

pub async fn create(
    _: Session,
    State(state): State<AppState>,
    Json(input): Json<NoteInput>,
) -> AppResult<Json<Note>> {
    checked(&input.body)?;
    let now = crate::now();
    let row = sqlx::query(
        "INSERT INTO scratch_notes (body, created_at, updated_at) VALUES (?1, ?2, ?2)
         RETURNING id, body, created_at, updated_at",
    )
    .bind(&input.body)
    .bind(now)
    .fetch_one(&state.db)
    .await?;
    state.backup.signal();
    Ok(Json(row_to_note(&row)))
}

pub async fn update(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<NoteInput>,
) -> AppResult<Json<Note>> {
    checked(&input.body)?;
    let row = sqlx::query(
        "UPDATE scratch_notes SET body = ?1, updated_at = ?2 WHERE id = ?3
         RETURNING id, body, created_at, updated_at",
    )
    .bind(&input.body)
    .bind(crate::now())
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    state.backup.signal();
    Ok(Json(row_to_note(&row)))
}

pub async fn remove(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<serde_json::Value>> {
    let affected = sqlx::query("DELETE FROM scratch_notes WHERE id = ?1")
        .bind(id)
        .execute(&state.db)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound);
    }
    state.backup.signal();
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// The notes as an export carries them.
pub(crate) async fn export(db: &sqlx::SqlitePool) -> AppResult<Vec<ScratchMeta>> {
    let rows = sqlx::query(
        "SELECT body, created_at, updated_at FROM scratch_notes ORDER BY created_at, id",
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .iter()
        .map(|row| ScratchMeta {
            body: row.get("body"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        })
        .collect())
}

/// An export's notes put back beside whatever notes there are already.
pub(crate) async fn restore(db: &mut SqliteConnection, notes: &[ScratchMeta]) -> AppResult<()> {
    for note in notes {
        sqlx::query("INSERT INTO scratch_notes (body, created_at, updated_at) VALUES (?1, ?2, ?3)")
            .bind(&note.body)
            .bind(note.created_at)
            .bind(note.updated_at)
            .execute(&mut *db)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{export, restore};
    use crate::routes::manifest::ScratchMeta;

    #[tokio::test]
    async fn the_notes_go_out_with_an_export_and_come_back() {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&db).await.unwrap();
        let note = |body: &str, at| ScratchMeta {
            body: body.into(),
            created_at: at,
            updated_at: at + 1,
        };
        let mut conn = db.acquire().await.unwrap();
        restore(
            &mut conn,
            &[note("call the landlord", 10), note("idea: a drawer", 20)],
        )
        .await
        .unwrap();
        drop(conn);
        let out = export(&db).await.unwrap();
        let bodies: Vec<&str> = out.iter().map(|n| n.body.as_str()).collect();
        assert_eq!(bodies, ["call the landlord", "idea: a drawer"]);
        assert_eq!(out[1].updated_at, 21);
    }
}
