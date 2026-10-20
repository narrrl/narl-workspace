//! The trash: where a delete puts things, so a slip of `dd` is a `u` away
//! from being undone rather than an old revision of the whole database away.
//!
//! A trashed node is moved, whole, under one hidden root (see migration
//! `0019`). Nothing that walks down from the spaces reaches it there, so it
//! leaves the tree, the search, the mount, the export and the readable mirror
//! without any of them having to know about the trash. What is still in it
//! after `WORKSPACE_TRASH_DAYS` is purged by the sweep, and only then are its
//! files deleted.

use axum::{
    extract::{Path, State},
    Json,
};
use serde::Serialize;
use sqlx::{Row, SqliteConnection, SqlitePool};

use crate::{
    auth::Session,
    error::{AppError, AppResult},
    routes::nodes::{self, Node},
    state::AppState,
};

/// The `role` of the hidden root everything trashed hangs under.
pub const TRASH: &str = "trash";

/// The trash root's id. Made by a migration, so it is always there.
pub async fn root(db: &mut SqliteConnection) -> AppResult<i64> {
    Ok(sqlx::query_scalar("SELECT id FROM nodes WHERE role = ?1")
        .bind(TRASH)
        .fetch_one(&mut *db)
        .await?)
}

/// Whether a node is in the trash, at any depth — or is the trash itself.
pub async fn in_trash(db: &mut SqliteConnection, id: i64) -> AppResult<bool> {
    let found: i64 = sqlx::query_scalar(
        "WITH RECURSIVE up (id, parent_id) AS (
             SELECT id, parent_id FROM nodes WHERE id = ?1
             UNION ALL
             SELECT n.id, n.parent_id FROM nodes n JOIN up ON n.id = up.parent_id
         )
         SELECT count(*) FROM up JOIN nodes n ON n.id = up.id WHERE n.role = ?2",
    )
    .bind(id)
    .bind(TRASH)
    .fetch_one(&mut *db)
    .await?;
    Ok(found > 0)
}

/// Move a node and everything under it into the trash. Returns how many
/// nodes went. The one way a node is deleted, from the browser and from the
/// mount alike; its files stay on the disk until it is purged.
pub async fn trash_subtree(db: &SqlitePool, id: i64) -> AppResult<usize> {
    let mut tx = crate::db::begin_write(db).await?;

    let row = sqlx::query("SELECT parent_id, slug, role FROM nodes WHERE id = ?1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AppError::NotFound)?;
    match row.get::<Option<String>, _>("role").as_deref() {
        Some(nodes::DIARY) => {
            return Err(AppError::BadRequest(
                "the diary space stays — empty it instead, or rename it".into(),
            ))
        }
        Some(TRASH) => {
            return Err(AppError::BadRequest(
                "the trash stays — :trash purge empties it".into(),
            ))
        }
        _ => {}
    }
    if in_trash(&mut tx, id).await? {
        return Err(AppError::BadRequest("that is in the trash already".into()));
    }

    let ancestry = ancestry(&mut tx, id).await?;
    let from_path = ancestry
        .iter()
        .take(ancestry.len().saturating_sub(1))
        .map(|(_, slug)| slug.as_str())
        .collect::<Vec<_>>()
        .join("/");
    let from_space = ancestry
        .first()
        .map(|(id, _)| *id)
        .filter(|&space| space != id);
    let count = nodes::descendants(&mut tx, id).await?.len();

    let trash = root(&mut tx).await?;
    let slug = nodes::unique_slug(
        &mut tx,
        Some(trash),
        &row.get::<String, _>("slug"),
        Some(id),
    )
    .await?;
    // `updated_at` stays as it was: it says when the thing was last written,
    // and a restore should bring it back as it was, not as new.
    sqlx::query("UPDATE nodes SET parent_id = ?1, slug = ?2 WHERE id = ?3")
        .bind(trash)
        .bind(&slug)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT OR REPLACE INTO trash (node_id, from_parent, from_space, from_path, deleted_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(id)
    .bind(row.get::<Option<i64>, _>("parent_id"))
    .bind(from_space)
    .bind(&from_path)
    .bind(crate::now())
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(count)
}

/// The ids and slugs from the root down to the node itself.
async fn ancestry(db: &mut SqliteConnection, id: i64) -> AppResult<Vec<(i64, String)>> {
    let rows = sqlx::query(
        "WITH RECURSIVE up (id, parent_id, slug, depth) AS (
             SELECT id, parent_id, slug, 0 FROM nodes WHERE id = ?1
             UNION ALL
             SELECT n.id, n.parent_id, n.slug, up.depth + 1
             FROM nodes n JOIN up ON n.id = up.parent_id
         )
         SELECT id, slug FROM up ORDER BY depth DESC",
    )
    .bind(id)
    .fetch_all(&mut *db)
    .await?;
    Ok(rows
        .iter()
        .map(|row| (row.get("id"), row.get("slug")))
        .collect())
}

#[derive(Serialize)]
pub struct TrashItem {
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub slug: String,
    /// The path of the folder it was deleted from, `work/notes`; empty for a
    /// space.
    pub from: String,
    pub deleted_at: i64,
    /// When the sweep purges it.
    pub purge_at: i64,
    /// How many nodes are inside it, for a folder or a space.
    pub inside: usize,
}

/// The rows that stand for something still in the trash. A row whose node
/// has left it some other way — moved out by hand — stands for nothing.
const LIVE_ROWS: &str = "FROM trash t JOIN nodes n ON n.id = t.node_id
     WHERE n.parent_id = (SELECT id FROM nodes WHERE role = 'trash')";

/// `GET /api/trash` — what is in it, the last thing deleted first.
pub async fn list(_: Session, State(state): State<AppState>) -> AppResult<Json<Vec<TrashItem>>> {
    let mut conn = state.db.acquire().await?;
    let rows = sqlx::query(&format!(
        "SELECT t.node_id, t.from_path, t.deleted_at, n.kind, n.name, n.slug {LIVE_ROWS}
         ORDER BY t.deleted_at DESC, t.node_id DESC"
    ))
    .fetch_all(&mut *conn)
    .await?;

    let keep = state.config.trash_days * 86_400;
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let id: i64 = row.get("node_id");
        let deleted_at: i64 = row.get("deleted_at");
        items.push(TrashItem {
            id,
            kind: row.get("kind"),
            name: row.get("name"),
            slug: row.get("slug"),
            from: row.get("from_path"),
            deleted_at,
            purge_at: deleted_at + keep,
            inside: nodes::descendants(&mut conn, id).await?.len() - 1,
        });
    }
    Ok(Json(items))
}

#[derive(Serialize)]
pub struct Restored {
    pub node: Node,
    /// Where it came from was gone or in the trash itself, so it went to the
    /// root of its space — or of the first space, when that is gone too.
    pub elsewhere: bool,
}

/// `POST /api/trash/{id}/restore` — back where it was deleted from.
pub async fn restore(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<Restored>> {
    let elsewhere = restore_node(&state, id).await?;
    state.backup.signal();
    Ok(Json(Restored {
        node: nodes::load(&state, id).await?,
        elsewhere,
    }))
}

/// Put one trashed node back. A space goes back to the root, last in the
/// switcher; anything else is moved like any other move, so a sealed folder
/// it goes back into seals it on the way in, or refuses while the vault is
/// shut. Returns whether it had to go somewhere other than where it was.
pub async fn restore_node(state: &AppState, id: i64) -> AppResult<bool> {
    let mut conn = state.db.acquire().await?;
    let row = sqlx::query(&format!(
        "SELECT n.kind, n.name, t.from_parent, t.from_space {LIVE_ROWS} AND t.node_id = ?1"
    ))
    .bind(id)
    .fetch_optional(&mut *conn)
    .await?
    .ok_or(AppError::NotFound)?;
    let kind: String = row.get("kind");
    let name: String = row.get("name");

    if kind == nodes::SPACE {
        drop(conn);
        let mut tx = crate::db::begin_write(&state.db).await?;
        let slug = nodes::unique_slug(&mut tx, None, &nodes::slugify(&name), Some(id)).await?;
        sqlx::query(
            "UPDATE nodes SET parent_id = NULL, slug = ?1,
                 position = (SELECT COALESCE(MAX(position), -1) + 1 FROM nodes WHERE parent_id IS NULL)
             WHERE id = ?2",
        )
        .bind(&slug)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM trash WHERE node_id = ?1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        // Under another slug when its own was taken meanwhile.
        crate::routes::links::follow(&mut tx, id).await?;
        tx.commit().await?;
        return Ok(false);
    }

    let from_parent: Option<i64> = row.get("from_parent");
    let from_space: Option<i64> = row.get("from_space");
    let (target, elsewhere) = match from_parent {
        Some(parent) if is_live_container(&mut conn, parent).await? => (parent, false),
        _ => (fallback(&mut conn, from_space).await?, true),
    };
    drop(conn);

    nodes::move_nodes(state, &[id], target, None).await?;
    // After the move, not with it: the move seals what it has to, which takes
    // transactions of its own. Should this delete not happen, the row stands
    // for nothing — it no longer matches `LIVE_ROWS` — and the sweep drops it.
    sqlx::query("DELETE FROM trash WHERE node_id = ?1")
        .bind(id)
        .execute(&state.db)
        .await?;
    Ok(elsewhere)
}

async fn is_live_container(db: &mut SqliteConnection, id: i64) -> AppResult<bool> {
    let kind: Option<String> = sqlx::query_scalar("SELECT kind FROM nodes WHERE id = ?1")
        .bind(id)
        .fetch_optional(&mut *db)
        .await?;
    Ok(kind.is_some_and(|kind| nodes::is_container(&kind)) && !in_trash(db, id).await?)
}

/// Where a node goes back to when its folder is gone: the root of its space,
/// or, when the space was purged, of the first space there is. A space that
/// is in the trash itself has to come back first, or the node would go
/// straight back into the trash with it.
async fn fallback(db: &mut SqliteConnection, space: Option<i64>) -> AppResult<i64> {
    if let Some(space) = space {
        let row = sqlx::query("SELECT name, parent_id FROM nodes WHERE id = ?1")
            .bind(space)
            .fetch_optional(&mut *db)
            .await?;
        if let Some(row) = row {
            if row.get::<Option<i64>, _>("parent_id").is_none() {
                return Ok(space);
            }
            return Err(AppError::BadRequest(format!(
                "its space, {}, is in the trash too — restore that first",
                row.get::<String, _>("name")
            )));
        }
    }
    sqlx::query_scalar(
        "SELECT id FROM nodes WHERE parent_id IS NULL AND kind = 'space'
         ORDER BY position, name LIMIT 1",
    )
    .fetch_optional(&mut *db)
    .await?
    .ok_or_else(|| {
        AppError::BadRequest("there is no space to restore it into — :space! makes one".into())
    })
}

/// `DELETE /api/trash/{id}` — purge one thing now, files and all.
pub async fn purge(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<serde_json::Value>> {
    let listed: Option<i64> =
        sqlx::query_scalar(&format!("SELECT t.node_id {LIVE_ROWS} AND t.node_id = ?1"))
            .bind(id)
            .fetch_optional(&state.db)
            .await?;
    if listed.is_none() {
        return Err(AppError::NotFound);
    }
    let removed = nodes::purge_subtree(&state.db, &state.config.uploads_dir(), id).await?;
    state.backup.signal();
    Ok(Json(serde_json::json!({ "ok": true, "removed": removed })))
}

/// `DELETE /api/trash` — empty it.
pub async fn empty(
    _: Session,
    State(state): State<AppState>,
) -> AppResult<Json<serde_json::Value>> {
    let removed = purge_older(&state.db, &state.config.uploads_dir(), i64::MAX).await?;
    state.backup.signal();
    Ok(Json(serde_json::json!({ "ok": true, "removed": removed })))
}

/// Purge everything deleted before `cutoff`, and forget the rows that no
/// longer stand for anything. Returns how many things were purged.
pub async fn purge_older(
    db: &SqlitePool,
    uploads: &std::path::Path,
    cutoff: i64,
) -> AppResult<usize> {
    let due: Vec<i64> = sqlx::query_scalar(&format!(
        "SELECT t.node_id {LIVE_ROWS} AND t.deleted_at < ?1"
    ))
    .bind(cutoff)
    .fetch_all(db)
    .await?;
    for &id in &due {
        nodes::purge_subtree(db, uploads, id).await?;
    }
    sqlx::query(
        "DELETE FROM trash WHERE node_id NOT IN
             (SELECT id FROM nodes WHERE parent_id = (SELECT id FROM nodes WHERE role = 'trash'))",
    )
    .execute(db)
    .await?;
    Ok(due.len())
}

#[cfg(test)]
mod tests {
    use super::{in_trash, purge_older, restore_node, trash_subtree};
    use crate::{error::AppError, state::AppState};

    async fn node(state: &AppState, parent: i64, kind: &str, slug: &str) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO nodes (parent_id, kind, name, slug, body, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3, 'text', 0, 0) RETURNING id",
        )
        .bind(parent)
        .bind(kind)
        .bind(slug)
        .fetch_one(&state.db)
        .await
        .unwrap()
    }

    async fn space(state: &AppState, slug: &str) -> i64 {
        sqlx::query_scalar("SELECT id FROM nodes WHERE slug = ?1 AND parent_id IS NULL")
            .bind(slug)
            .fetch_one(&state.db)
            .await
            .unwrap()
    }

    async fn parent_and_slug(state: &AppState, id: i64) -> (Option<i64>, String) {
        sqlx::query_as("SELECT parent_id, slug FROM nodes WHERE id = ?1")
            .bind(id)
            .fetch_one(&state.db)
            .await
            .unwrap()
    }

    async fn trashed(state: &AppState, id: i64) -> bool {
        in_trash(&mut state.db.acquire().await.unwrap(), id)
            .await
            .unwrap()
    }

    /// A folder goes with what is in it and comes back to where it was —
    /// under its old slug, unless a new sibling took it meanwhile.
    #[tokio::test]
    async fn a_delete_comes_back_where_it_was() {
        let state = crate::routes::tests::state_in(".".into()).await;
        let work = space(&state, "work").await;
        let folder = node(&state, work, "folder", "notes").await;
        let inside = node(&state, folder, "document", "monday").await;

        assert_eq!(trash_subtree(&state.db, folder).await.unwrap(), 2);
        assert!(trashed(&state, folder).await);
        assert!(trashed(&state, inside).await);

        let taken = node(&state, work, "folder", "notes").await;
        assert!(!restore_node(&state, folder).await.unwrap());
        assert_eq!(
            parent_and_slug(&state, folder).await,
            (Some(work), "notes-2".into())
        );
        assert_eq!(parent_and_slug(&state, inside).await.0, Some(folder));
        assert!(!trashed(&state, inside).await);
        assert!(!trashed(&state, taken).await);

        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM trash")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(rows, 0);
    }

    /// Out of a folder that has since gone, a node lands at the root of its
    /// space; out of a space that is in the trash too, it waits for that.
    #[tokio::test]
    async fn without_its_folder_it_lands_in_its_space() {
        let state = crate::routes::tests::state_in(".".into()).await;
        let uni = space(&state, "uni").await;
        let folder = node(&state, uni, "folder", "algo").await;
        let doc = node(&state, folder, "document", "sheet-1").await;

        trash_subtree(&state.db, doc).await.unwrap();
        trash_subtree(&state.db, folder).await.unwrap();
        assert!(restore_node(&state, doc).await.unwrap());
        assert_eq!(parent_and_slug(&state, doc).await.0, Some(uni));

        trash_subtree(&state.db, doc).await.unwrap();
        trash_subtree(&state.db, uni).await.unwrap();
        assert!(matches!(
            restore_node(&state, doc).await,
            Err(AppError::BadRequest(_))
        ));
        assert!(!restore_node(&state, uni).await.unwrap());
        assert_eq!(parent_and_slug(&state, uni).await, (None, "uni".into()));
        assert!(!restore_node(&state, doc).await.unwrap());
        assert_eq!(parent_and_slug(&state, doc).await.0, Some(uni));
    }

    /// The diary and the trash itself cannot be thrown away, and nothing is
    /// thrown away twice.
    #[tokio::test]
    async fn some_things_stay() {
        let state = crate::routes::tests::state_in(".".into()).await;
        let diary: i64 = sqlx::query_scalar("SELECT id FROM nodes WHERE role = 'diary'")
            .fetch_one(&state.db)
            .await
            .unwrap();
        let trash = super::root(&mut state.db.acquire().await.unwrap())
            .await
            .unwrap();
        for id in [diary, trash] {
            assert!(matches!(
                trash_subtree(&state.db, id).await,
                Err(AppError::BadRequest(_))
            ));
        }
        let doc = node(&state, diary, "document", "today").await;
        trash_subtree(&state.db, doc).await.unwrap();
        assert!(matches!(
            trash_subtree(&state.db, doc).await,
            Err(AppError::BadRequest(_))
        ));
    }

    /// What is in the trash is not found by a search across every space, and
    /// the cards of a space in the trash leave the agenda.
    #[tokio::test]
    async fn the_trash_is_out_of_sight() {
        use axum::extract::{Query, State};

        let state = crate::routes::tests::state_in(".".into()).await;
        let session = || crate::auth::Session { id: "test".into() };
        let work = space(&state, "work").await;
        let doc = node(&state, work, "document", "quarterly").await;
        sqlx::query("UPDATE nodes SET body = 'zebra crossing' WHERE id = ?1")
            .bind(doc)
            .execute(&state.db)
            .await
            .unwrap();
        for (name, position) in [("inbox", 0), ("done", 1)] {
            sqlx::query(
                "INSERT INTO board_lists (space_id, name, position, created_at)
                 VALUES (?1, ?2, ?3, 0)",
            )
            .bind(work)
            .bind(name)
            .bind(position)
            .execute(&state.db)
            .await
            .unwrap();
        }
        let list: i64 = sqlx::query_scalar(
            "SELECT id FROM board_lists WHERE space_id = ?1 ORDER BY position LIMIT 1",
        )
        .bind(work)
        .fetch_one(&state.db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO cards (list_id, title, due_at, position, created_at, updated_at)
             VALUES (?1, 'late', 1, 0, 0, 0)",
        )
        .bind(list)
        .execute(&state.db)
        .await
        .unwrap();

        let search = || async {
            let query = serde_json::from_value(serde_json::json!({ "q": "zebra" })).unwrap();
            crate::routes::nodes::list(session(), State(state.clone()), Query(query))
                .await
                .unwrap()
                .0
                .len()
        };
        let agenda = || async {
            let agenda = crate::routes::boards::agenda(session(), State(state.clone()))
                .await
                .unwrap()
                .0;
            serde_json::to_value(agenda).unwrap()["cards"]
                .as_array()
                .unwrap()
                .len()
        };
        assert_eq!(search().await, 1);
        assert_eq!(agenda().await, 1);

        trash_subtree(&state.db, doc).await.unwrap();
        assert_eq!(search().await, 0);
        trash_subtree(&state.db, work).await.unwrap();
        assert_eq!(agenda().await, 0);
    }

    /// The sweep purges what is past the cutoff and leaves the rest, and a
    /// row left behind by a node moved out by hand is forgotten, not purged.
    #[tokio::test]
    async fn only_what_is_old_enough_is_purged() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::routes::tests::state_in(dir.path().into()).await;
        let work = space(&state, "work").await;
        let old = node(&state, work, "document", "old").await;
        let new = node(&state, work, "document", "new").await;
        let moved = node(&state, work, "document", "moved").await;
        for id in [old, new, moved] {
            trash_subtree(&state.db, id).await.unwrap();
        }
        sqlx::query("UPDATE trash SET deleted_at = 100 WHERE node_id IN (?1, ?2)")
            .bind(old)
            .bind(moved)
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("UPDATE nodes SET parent_id = ?1 WHERE id = ?2")
            .bind(work)
            .bind(moved)
            .execute(&state.db)
            .await
            .unwrap();

        assert_eq!(purge_older(&state.db, dir.path(), 1000).await.unwrap(), 1);
        let left: Vec<i64> =
            sqlx::query_scalar("SELECT id FROM nodes WHERE id IN (?1, ?2, ?3) ORDER BY id")
                .bind(old)
                .bind(new)
                .bind(moved)
                .fetch_all(&state.db)
                .await
                .unwrap();
        assert_eq!(left, [new, moved]);
        let rows: Vec<i64> = sqlx::query_scalar("SELECT node_id FROM trash")
            .fetch_all(&state.db)
            .await
            .unwrap();
        assert_eq!(rows, [new]);
    }
}
