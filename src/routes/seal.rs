//! Moving files that already exist into the vault, and back out of it.
//!
//! `POST /api/nodes/{id}/encrypt` seals a file node, or every file node under
//! a folder or space; `/decrypt` does the opposite. Either needs the vault open.
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

#[derive(Serialize)]
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
}

pub async fn encrypt(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<Resealed>> {
    reseal(&state, id, true).await.map(Json)
}

pub async fn decrypt(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<Resealed>> {
    reseal(&state, id, false).await.map(Json)
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
