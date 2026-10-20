//! Revisions: what a document said before it was written over, so a `:w!`
//! that should not have been, a save over the mount, or a paragraph deleted
//! last week can be taken back.
//!
//! A revision is kept by the write that replaces the text, in the same
//! transaction, so there is never a save without the version it replaced.
//! The sweep forgets them after `WORKSPACE_REVISION_DAYS`, but always keeps
//! the newest few of every document.

use axum::{
    extract::{Path, State},
    Json,
};
use serde::Serialize;
use sqlx::{Row, SqliteConnection, SqlitePool};

use crate::{
    auth::Session,
    error::{AppError, AppResult},
    routes::trash,
    state::AppState,
};

/// Why a revision was kept.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reason {
    /// An ordinary save, autosave included.
    Edit,
    /// `:w!`: a write that did not check what it wrote over.
    Force,
    /// A save over the SFTP mount.
    Sftp,
    /// A revision brought back, replacing what was there.
    Restore,
}

impl Reason {
    fn as_str(self) -> &'static str {
        match self {
            Reason::Edit => "edit",
            Reason::Force => "force",
            Reason::Sftp => "sftp",
            Reason::Restore => "restore",
        }
    }
}

/// Autosave writes every few seconds; an ordinary save keeps a revision only
/// when the newest one is at least this old.
const EDIT_GAP: i64 = 10 * 60;

/// How many of a document's revisions the sweep never forgets, however old.
pub const ALWAYS_KEPT: i64 = 20;

/// Keep what a document says now, before `new_body` replaces it. Called by
/// every write of a document's text, in its transaction. Returns whether a
/// revision was kept.
pub async fn keep(
    db: &mut SqliteConnection,
    id: i64,
    new_body: &str,
    reason: Reason,
) -> AppResult<bool> {
    let Some(row) = sqlx::query("SELECT kind, name, body FROM nodes WHERE id = ?1")
        .bind(id)
        .fetch_optional(&mut *db)
        .await?
    else {
        return Ok(false);
    };
    let body: String = row.get("body");
    // Only text is worth keeping: a rename is not a loss, and an empty
    // document has nothing to lose.
    if row.get::<String, _>("kind") != crate::routes::nodes::DOCUMENT
        || body == new_body
        || body.trim().is_empty()
    {
        return Ok(false);
    }

    let now = crate::now();
    if reason == Reason::Edit {
        let newest: Option<i64> =
            sqlx::query_scalar("SELECT MAX(saved_at) FROM revisions WHERE node_id = ?1")
                .bind(id)
                .fetch_one(&mut *db)
                .await?;
        if newest.is_some_and(|at| now - at < EDIT_GAP) {
            return Ok(false);
        }
    }

    sqlx::query(
        "INSERT INTO revisions (node_id, name, body, saved_at, reason)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(id)
    .bind(row.get::<String, _>("name"))
    .bind(&body)
    .bind(now)
    .bind(reason.as_str())
    .execute(&mut *db)
    .await?;
    Ok(true)
}

/// Forget the revisions saved before `cutoff`, except the newest
/// `ALWAYS_KEPT` of each document. Returns how many went.
pub async fn forget_older(db: &SqlitePool, cutoff: i64) -> sqlx::Result<u64> {
    let done = sqlx::query(
        "DELETE FROM revisions
         WHERE saved_at < ?1
           AND id NOT IN (
               SELECT r.id FROM revisions r
               WHERE r.node_id = revisions.node_id
               ORDER BY r.saved_at DESC, r.id DESC
               LIMIT ?2
           )",
    )
    .bind(cutoff)
    .bind(ALWAYS_KEPT)
    .execute(db)
    .await?;
    Ok(done.rows_affected())
}

#[derive(Serialize)]
pub struct RevisionSummary {
    pub id: i64,
    pub name: String,
    pub saved_at: i64,
    pub reason: String,
    /// Characters of text.
    pub size: i64,
}

#[derive(Serialize)]
pub struct Revision {
    pub id: i64,
    pub name: String,
    pub body: String,
    pub saved_at: i64,
    pub reason: String,
}

/// A document in the trash has no history to read, as it has no text: both
/// come back with it.
async fn readable(state: &AppState, id: i64) -> AppResult<()> {
    if trash::in_trash(&mut *state.db.acquire().await?, id).await? {
        return Err(AppError::NotFound);
    }
    Ok(())
}

/// `GET /api/nodes/{id}/revisions` — the newest first.
pub async fn list(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<Vec<RevisionSummary>>> {
    readable(&state, id).await?;
    let rows = sqlx::query(
        "SELECT id, name, saved_at, reason, length(body) AS size FROM revisions
         WHERE node_id = ?1 ORDER BY saved_at DESC, id DESC",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.iter()
            .map(|row| RevisionSummary {
                id: row.get("id"),
                name: row.get("name"),
                saved_at: row.get("saved_at"),
                reason: row.get("reason"),
                size: row.get("size"),
            })
            .collect(),
    ))
}

/// `GET /api/nodes/{id}/revisions/{rev}` — one, with its text.
pub async fn get(
    _: Session,
    State(state): State<AppState>,
    Path((id, rev)): Path<(i64, i64)>,
) -> AppResult<Json<Revision>> {
    readable(&state, id).await?;
    let row = sqlx::query(
        "SELECT id, name, body, saved_at, reason FROM revisions WHERE id = ?1 AND node_id = ?2",
    )
    .bind(rev)
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;
    Ok(Json(Revision {
        id: row.get("id"),
        name: row.get("name"),
        body: row.get("body"),
        saved_at: row.get("saved_at"),
        reason: row.get("reason"),
    }))
}

#[cfg(test)]
mod tests {
    use super::{forget_older, keep, Reason, ALWAYS_KEPT};
    use sqlx::SqlitePool;

    async fn db() -> SqlitePool {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&db).await.unwrap();
        db
    }

    async fn document(db: &SqlitePool, slug: &str, body: &str) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO nodes (parent_id, kind, name, slug, body, created_at, updated_at)
             VALUES ((SELECT id FROM nodes WHERE slug = 'work'), 'document', ?1, ?1, ?2, 0, 0)
             RETURNING id",
        )
        .bind(slug)
        .bind(body)
        .fetch_one(db)
        .await
        .unwrap()
    }

    async fn write(db: &SqlitePool, id: i64, body: &str, reason: Reason) -> bool {
        let mut conn = db.acquire().await.unwrap();
        let kept = keep(&mut conn, id, body, reason).await.unwrap();
        sqlx::query("UPDATE nodes SET body = ?1 WHERE id = ?2")
            .bind(body)
            .bind(id)
            .execute(&mut *conn)
            .await
            .unwrap();
        kept
    }

    async fn bodies(db: &SqlitePool, id: i64) -> Vec<String> {
        sqlx::query_scalar("SELECT body FROM revisions WHERE node_id = ?1 ORDER BY id")
            .bind(id)
            .fetch_all(db)
            .await
            .unwrap()
    }

    /// Autosave keeps one revision per ten minutes; `:w!` and a save over
    /// the mount always keep one; nothing is kept of an unchanged text or
    /// of an empty one.
    #[tokio::test]
    async fn what_a_write_keeps() {
        let db = db().await;
        let id = document(&db, "plan", "").await;
        assert!(!write(&db, id, "one", Reason::Edit).await);
        assert!(write(&db, id, "two", Reason::Edit).await);
        assert!(!write(&db, id, "three", Reason::Edit).await);
        assert!(!write(&db, id, "three", Reason::Force).await);
        assert!(write(&db, id, "four", Reason::Force).await);
        assert!(write(&db, id, "five", Reason::Sftp).await);
        assert_eq!(bodies(&db, id).await, ["one", "three", "four"]);

        sqlx::query("UPDATE revisions SET saved_at = saved_at - 3600")
            .execute(&db)
            .await
            .unwrap();
        assert!(write(&db, id, "six", Reason::Edit).await);
    }

    /// Through the API: a save that checked what it wrote over keeps one
    /// revision per ten minutes, `:w!` and a restore always keep one.
    #[tokio::test]
    async fn saves_keep_what_they_replace() {
        use axum::extract::{Path, State};

        let state = crate::routes::tests::state_in(".".into()).await;
        let id = document(&state.db, "plan", "first").await;
        let save = |body: &'static str, base: bool, restore: bool| {
            let state = state.clone();
            async move {
                let input = serde_json::from_value(serde_json::json!({
                    "name": "plan",
                    "body": body,
                    "base_updated_at": base.then_some(i64::MAX),
                    "restore": restore,
                }))
                .unwrap();
                let _saved = crate::routes::nodes::update(
                    crate::auth::Session { id: "test".into() },
                    State(state),
                    Path(id),
                    axum::Json(input),
                )
                .await
                .unwrap();
            }
        };
        save("second", true, false).await;
        save("third", true, false).await;
        save("fourth", false, false).await;
        save("first", true, true).await;

        let kept: Vec<(String, String)> =
            sqlx::query_as("SELECT body, reason FROM revisions WHERE node_id = ?1 ORDER BY id")
                .bind(id)
                .fetch_all(&state.db)
                .await
                .unwrap();
        let kept: Vec<(&str, &str)> = kept.iter().map(|(b, r)| (b.as_str(), r.as_str())).collect();
        assert_eq!(
            kept,
            [("first", "edit"), ("third", "force"), ("fourth", "restore")]
        );
    }

    /// Old revisions go, except the newest few of every document.
    #[tokio::test]
    async fn the_newest_stay_however_old() {
        let db = db().await;
        let busy = document(&db, "busy", "start").await;
        let quiet = document(&db, "quiet", "start").await;
        for n in 0..ALWAYS_KEPT + 5 {
            write(&db, busy, &format!("busy {n}"), Reason::Force).await;
        }
        write(&db, quiet, "only", Reason::Force).await;
        sqlx::query("UPDATE revisions SET saved_at = id")
            .execute(&db)
            .await
            .unwrap();

        assert_eq!(forget_older(&db, i64::MAX).await.unwrap(), 5);
        assert_eq!(bodies(&db, busy).await.len() as i64, ALWAYS_KEPT);
        assert_eq!(bodies(&db, busy).await[0], "busy 4");
        assert_eq!(bodies(&db, quiet).await, ["start"]);
    }
}
