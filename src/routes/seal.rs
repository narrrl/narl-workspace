//! Moving files that already exist into the vault, and back out of it.
//!
//! `POST /api/nodes/{id}/encrypt` seals a file node, or every file node under
//! a folder or space; `/decrypt` does the opposite. Either needs the vault open.
//!
//! A folder or space that is encrypted stays sealed: it and every folder under
//! it carry `sealed`, and whatever lands there later — an upload, an import, an
//! SFTP write, a file moved in — is sealed on the way in, or refused while the
//! vault is shut. `enter` is that step for a move. Decrypting the folder lifts
//! it; a file or folder inside a sealed one cannot be decrypted on its own.
//! Documents are not files and stay as they are.
//!
//! A file is re-written under a **new** media id rather than in place. The
//! mirror treats `uploads/<uuid>` as immutable, so rewriting one would leave
//! the old bytes on Drive as the previous revision of the same file — the
//! plaintext, in the case that matters. Under a new id the sealed copy is a new
//! file, and the old id is put on `backup_retired`, which the mirror takes off
//! Drive for good whatever `WORKSPACE_BACKUP_PRUNE` says. Documents that embed
//! the file have their links moved to the new id in the same transaction.

use axum::{
    extract::{Path, State},
    Json,
};
use serde::Serialize;
use sqlx::Row;
use uuid::Uuid;

use crate::{
    auth::Session,
    error::{AppError, AppResult},
    routes::{media::MEDIA_REF, nodes},
    state::AppState,
    vault,
};

#[derive(Serialize, Default)]
pub struct Resealed {
    /// How many files changed.
    pub files: usize,
    /// Of what was asked about, how many were already that way.
    pub unchanged: usize,
    /// How many could not be re-written; the rest still were.
    pub failed: usize,
    /// Shared documents that embed a file just sealed: their readers now see
    /// a gap where it was, because a share link never serves a sealed file.
    pub shared: usize,
    /// Whether what was asked about is now a sealed folder or space.
    pub sealed: bool,
}

pub async fn encrypt(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<Resealed>> {
    seal_node(&state, id).await.map(Json)
}

pub async fn decrypt(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<Resealed>> {
    open_node(&state, id).await.map(Json)
}

async fn seal_node(state: &AppState, id: i64) -> AppResult<Resealed> {
    let (kind, _) = place(state, id).await?;
    if !nodes::is_container(&kind) {
        return reseal(state, id, true).await;
    }
    state.vault.key().ok_or(AppError::Locked)?;
    // Flagged first, so nothing that lands while the files are re-written
    // slips in as plaintext; and an empty folder is worth sealing too.
    mark(state, id, true).await?;
    let mut done = reseal(state, id, true).await?;
    done.sealed = true;
    Ok(done)
}

async fn open_node(state: &AppState, id: i64) -> AppResult<Resealed> {
    let (kind, parent) = place(state, id).await?;
    if let Some(parent) = parent {
        if sealed(state, parent).await? {
            let what = if kind == nodes::FILE {
                "file"
            } else {
                "folder"
            };
            return Err(AppError::BadRequest(format!(
                "this {what} is inside a sealed folder — decrypt that folder instead"
            )));
        }
    }
    if !nodes::is_container(&kind) {
        return reseal(state, id, false).await;
    }
    state.vault.key().ok_or(AppError::Locked)?;
    mark(state, id, false).await?;
    reseal(state, id, false).await
}

/// Put `id` under `parent`'s rules before it moves there: when `parent` is
/// sealed, flag the folders of `id` and seal its files. The caller moves it
/// only if this succeeds — the triggers of 0015 refuse it otherwise.
pub async fn enter(state: &AppState, id: i64, parent: i64) -> AppResult<()> {
    if !sealed(state, parent).await? {
        return Ok(());
    }
    // Already sealed — a rename inside the folder, or a move between two
    // sealed ones — needs nothing, not even the vault open.
    let ready: Option<bool> = sqlx::query_scalar(
        "SELECT CASE WHEN n.kind = 'file' THEN m.encrypted
                     WHEN n.kind = 'document' THEN 1
                     ELSE n.sealed END
         FROM nodes n LEFT JOIN media m ON m.id = n.media_id WHERE n.id = ?1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?;
    match ready {
        None => return Err(AppError::NotFound),
        // A document is not sealed; it moves in as it is.
        Some(true) => return Ok(()),
        Some(false) => {}
    }
    state.vault.key().ok_or(AppError::Locked)?;
    let (kind, _) = place(state, id).await?;
    if nodes::is_container(&kind) {
        mark(state, id, true).await?;
    }
    let done = reseal(state, id, true).await?;
    if done.failed > 0 {
        return Err(AppError::BadRequest(format!(
            "{} file(s) would not seal, so nothing moved into the sealed folder",
            done.failed
        )));
    }
    Ok(())
}

/// Whether files that land in this folder or space are sealed.
pub async fn sealed(state: &AppState, id: i64) -> AppResult<bool> {
    Ok(sqlx::query_scalar("SELECT sealed FROM nodes WHERE id = ?1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .unwrap_or(false))
}

async fn place(state: &AppState, id: i64) -> AppResult<(String, Option<i64>)> {
    let row = sqlx::query("SELECT kind, parent_id FROM nodes WHERE id = ?1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok((row.get("kind"), row.get("parent_id")))
}

/// Set or lift `sealed` on a folder or space and every folder under it.
async fn mark(state: &AppState, id: i64, on: bool) -> AppResult<()> {
    let mut tx = crate::db::begin_write(&state.db).await?;
    let list = nodes::descendants(&mut tx, id)
        .await?
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    sqlx::query(&format!(
        "UPDATE nodes SET sealed = ?1 WHERE kind IN ('space', 'folder') AND id IN ({list})"
    ))
    .bind(on)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

struct Target {
    node_id: i64,
    media_id: String,
    encrypted: bool,
}

async fn reseal(state: &AppState, id: i64, seal: bool) -> AppResult<Resealed> {
    let key = state.vault.key().ok_or(AppError::Locked)?;

    let subtree = nodes::descendants(&mut *state.db.acquire().await?, id).await?;
    if subtree.is_empty() {
        return Err(AppError::NotFound);
    }
    let list = subtree
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let targets: Vec<Target> = sqlx::query(&format!(
        "SELECT n.id, n.media_id, m.encrypted
         FROM nodes n JOIN media m ON m.id = n.media_id
         WHERE n.kind = 'file' AND n.id IN ({list})"
    ))
    .fetch_all(&state.db)
    .await?
    .iter()
    .map(|row| Target {
        node_id: row.get("id"),
        media_id: row.get("media_id"),
        encrypted: row.get("encrypted"),
    })
    .collect();

    if targets.is_empty() {
        // An empty folder is nothing to re-write, but it is no mistake either.
        let (kind, _) = place(state, id).await?;
        if nodes::is_container(&kind) {
            return Ok(Resealed::default());
        }
        return Err(AppError::BadRequest("there are no files there".into()));
    }

    // A folder is many files, and one that will not re-write — gone from the
    // disk, cut short — is no reason to leave the rest as they were.
    let mut files = 0;
    let mut unchanged = 0;
    let mut failed = Vec::new();
    for target in &targets {
        if target.encrypted == seal {
            unchanged += 1;
            continue;
        }
        match rewrite(state, &key, target, seal).await {
            Ok(()) => files += 1,
            Err(e) => {
                tracing::warn!(node = target.node_id, "could not re-write a file: {e:?}");
                failed.push(e);
            }
        }
    }

    if files > 0 {
        state.backup.signal();
    }
    // Asked about one file, the reason it failed is the answer.
    if files == 0 && unchanged == 0 && failed.len() == 1 {
        return Err(failed.remove(0));
    }
    let shared: i64 = if seal && files > 0 {
        sqlx::query_scalar(&format!(
            "SELECT COUNT(DISTINCT d.id) FROM nodes d
             JOIN node_media nm ON nm.node_id = d.id
             JOIN nodes f ON f.media_id = nm.media_id
             WHERE d.kind = 'document' AND d.share_token IS NOT NULL AND f.id IN ({list})"
        ))
        .fetch_one(&state.db)
        .await?
    } else {
        0
    };
    Ok(Resealed {
        files,
        unchanged,
        failed: failed.len(),
        shared: shared as usize,
        sealed: false,
    })
}

/// Re-write one file sealed or opened under a new id, then point everything
/// at the new id and retire the old one.
async fn rewrite(state: &AppState, key: &vault::Key, target: &Target, seal: bool) -> AppResult<()> {
    let uploads = state.config.uploads_dir();
    let old = target.media_id.clone();
    let new = Uuid::new_v4().to_string();
    let source = uploads.join(&old);
    let partial = uploads.join(format!("{new}.part"));

    let written = {
        let (key, old, new, source, partial) = (
            key.clone(),
            old.clone(),
            new.clone(),
            source.clone(),
            partial.clone(),
        );
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            if seal {
                let mut plain = std::fs::File::open(&source)?;
                vault::seal_file(&key, &new, &mut plain, &partial)?;
            } else {
                let mut opened = vault::Reader::open(&key, &old, &source)?;
                let mut out = std::fs::File::create(&partial)?;
                std::io::copy(&mut opened, &mut out)?;
                out.sync_all()?;
            }
            Ok(())
        })
        .await
        .map_err(anyhow::Error::from)
        .and_then(|done| done)
    };
    if let Err(e) = written {
        let _ = tokio::fs::remove_file(&partial).await;
        return Err(e.into());
    }
    tokio::fs::rename(&partial, uploads.join(&new)).await?;

    let committed = async {
        let mut tx = crate::db::begin_write(&state.db).await?;
        let copied = sqlx::query(
            "INSERT INTO media (id, filename, mime, size, created_at, encrypted)
             SELECT ?1, filename, mime, size, created_at, ?2 FROM media WHERE id = ?3",
        )
        .bind(&new)
        .bind(seal)
        .bind(&old)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        // Another re-write of the same file got there first.
        if copied == 0 {
            return Err(AppError::NotFound);
        }
        sqlx::query("UPDATE nodes SET media_id = ?1 WHERE media_id = ?2")
            .bind(&new)
            .bind(&old)
            .execute(&mut *tx)
            .await?;
        // Every document and card that embeds the file keeps showing it —
        // however the link was written: an id typed in capitals is the
        // same file.
        let bodies = sqlx::query(
            "SELECT 'nodes' AS tbl, id, body FROM nodes WHERE instr(lower(body), ?1) > 0
             UNION ALL
             SELECT 'cards', id, body FROM cards WHERE instr(lower(body), ?1) > 0",
        )
        .bind(&old)
        .fetch_all(&mut *tx)
        .await?;
        for row in bodies {
            let body: String = row.get("body");
            let moved = MEDIA_REF.replace_all(&body, |caps: &regex::Captures| {
                if caps[1].eq_ignore_ascii_case(&old) {
                    format!("/api/media/{new}")
                } else {
                    caps[0].to_string()
                }
            });
            if moved == body {
                continue;
            }
            let table: String = row.get("tbl");
            let update = if table == "cards" {
                "UPDATE cards SET body = ?1 WHERE id = ?2"
            } else {
                "UPDATE nodes SET body = ?1 WHERE id = ?2"
            };
            sqlx::query(update)
                .bind(moved.as_ref())
                .bind(row.get::<i64, _>("id"))
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("UPDATE node_media SET media_id = ?1 WHERE media_id = ?2")
            .bind(&new)
            .bind(&old)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT OR IGNORE INTO backup_retired (path) VALUES ('uploads/' || ?1)")
            .bind(&old)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM media WHERE id = ?1")
            .bind(&old)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        AppResult::Ok(())
    }
    .await;

    if let Err(e) = committed {
        let _ = tokio::fs::remove_file(uploads.join(&new)).await;
        return Err(e);
    }

    // The database no longer knows the old bytes; they go from the disk too.
    let _ = tokio::fs::remove_file(&source).await;
    tracing::info!(node = target.node_id, from = %old, to = %new, sealed = seal, "file re-written");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeroize::Zeroizing;

    use crate::{error::AppError, routes::media};

    struct World {
        state: AppState,
        _dir: tempfile::TempDir,
        space: i64,
        folder: i64,
    }

    /// A space with a folder in it, and a vault that is open.
    async fn world() -> World {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("uploads")).unwrap();
        let state = crate::routes::tests::state_in(dir.path().into()).await;
        state.vault.open_with(Zeroizing::new([7u8; 32]));
        let space = node(&state, None, "space", "vaulted").await;
        let folder = node(&state, Some(space), "folder", "keys").await;
        World {
            state,
            _dir: dir,
            space,
            folder,
        }
    }

    async fn node(state: &AppState, parent: Option<i64>, kind: &str, name: &str) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO nodes (parent_id, kind, name, slug, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3, 1, 1) RETURNING id",
        )
        .bind(parent)
        .bind(kind)
        .bind(name)
        .fetch_one(&state.db)
        .await
        .unwrap()
    }

    /// A plaintext file, written the way an upload writes one.
    async fn plain_file(state: &AppState, parent: i64, name: &str) -> AppResult<i64> {
        let id = Uuid::new_v4().to_string();
        std::fs::write(state.config.uploads_dir().join(&id), b"secret bytes").unwrap();
        media::insert_file(&state.db, parent, &id, name, "text/plain", 12, false).await
    }

    async fn encrypted(state: &AppState, node: i64) -> bool {
        sqlx::query_scalar(
            "SELECT m.encrypted FROM nodes n JOIN media m ON m.id = n.media_id WHERE n.id = ?1",
        )
        .bind(node)
        .fetch_one(&state.db)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn a_sealed_folder_stays_sealed() {
        let w = world().await;
        let file = plain_file(&w.state, w.folder, "a.txt").await.unwrap();

        let done = seal_node(&w.state, w.folder).await.unwrap();
        assert!(done.sealed);
        assert_eq!(done.files, 1);
        assert!(encrypted(&w.state, file).await);
        assert!(sealed(&w.state, w.folder).await.unwrap());
        assert!(!sealed(&w.state, w.space).await.unwrap());

        // A folder made in it later is sealed from the start.
        let inner = node(&w.state, Some(w.folder), "folder", "inner").await;
        assert!(sealed(&w.state, inner).await.unwrap());

        // And no path puts plaintext in it: the database refuses.
        let refused = plain_file(&w.state, inner, "b.txt").await.unwrap_err();
        assert!(
            matches!(&refused, AppError::BadRequest(m) if m.contains("sealed")),
            "{refused:?}"
        );
    }

    #[tokio::test]
    async fn an_empty_folder_can_be_sealed() {
        let w = world().await;
        let done = seal_node(&w.state, w.folder).await.unwrap();
        assert_eq!((done.files, done.sealed), (0, true));
    }

    #[tokio::test]
    async fn moving_in_seals_and_a_shut_vault_refuses() {
        let w = world().await;
        seal_node(&w.state, w.folder).await.unwrap();
        let outside = node(&w.state, Some(w.space), "folder", "outside").await;
        let file = plain_file(&w.state, outside, "c.txt").await.unwrap();

        w.state.vault.lock();
        assert!(matches!(
            enter(&w.state, file, w.folder).await,
            Err(AppError::Locked)
        ));
        assert!(!encrypted(&w.state, file).await);

        w.state.vault.open_with(Zeroizing::new([7u8; 32]));
        enter(&w.state, outside, w.folder).await.unwrap();
        assert!(encrypted(&w.state, file).await);
        assert!(sealed(&w.state, outside).await.unwrap());

        // Already sealed, it moves about inside without the vault.
        w.state.vault.lock();
        enter(&w.state, file, w.folder).await.unwrap();
        // A document is no file and goes in as it is.
        let doc = node(&w.state, Some(w.space), "document", "notes").await;
        enter(&w.state, doc, w.folder).await.unwrap();
    }

    #[tokio::test]
    async fn only_the_sealed_folder_itself_opens() {
        let w = world().await;
        let inner = node(&w.state, Some(w.folder), "folder", "inner").await;
        let file = plain_file(&w.state, inner, "d.txt").await.unwrap();
        seal_node(&w.state, w.folder).await.unwrap();

        for id in [inner, file] {
            assert!(matches!(
                open_node(&w.state, id).await,
                Err(AppError::BadRequest(m)) if m.contains("inside a sealed folder")
            ));
        }

        let done = open_node(&w.state, w.folder).await.unwrap();
        assert_eq!(done.files, 1);
        assert!(!encrypted(&w.state, file).await);
        assert!(!sealed(&w.state, w.folder).await.unwrap());
        assert!(!sealed(&w.state, inner).await.unwrap());
        plain_file(&w.state, inner, "e.txt").await.unwrap();
    }
}
