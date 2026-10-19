use std::{collections::BTreeSet, sync::LazyLock};

use axum::{
    body::Body,
    extract::{multipart::Field, Multipart, Path, State},
    http::header,
    response::{IntoResponse, Response},
    Json,
};
use regex::Regex;
use serde::Serialize;
use sqlx::{Row, SqlitePool};
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::{
    auth::Session,
    error::{AppError, AppResult},
    routes::nodes,
    state::AppState,
};

/// Media referenced from a document body always looks like `/api/media/<uuid>`,
/// which is how a document claims ownership of the files it embeds.
pub(crate) static MEDIA_REF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"/api/media/([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})")
        .expect("static regex is valid")
});

/// An upload is attacker-controlled content served from our own origin, so the
/// browser is only ever told it is a type that cannot execute script. Anything
/// else — `text/html`, `image/svg+xml`, an unrecognised type — is stored and
/// served as an opaque download instead.
pub(crate) fn sanitize_mime(raw: &str) -> String {
    let base = raw
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    let (top, sub) = base.split_once('/').unwrap_or_default();
    let inlineable = match top {
        // SVG is a document: it can carry <script> and same-origin markup.
        "image" => sub != "svg+xml" && !sub.is_empty(),
        "video" | "audio" => !sub.is_empty(),
        "text" => sub == "plain",
        "application" => sub == "pdf",
        _ => false,
    };

    if inlineable {
        base
    } else {
        "application/octet-stream".to_string()
    }
}

fn is_inline(mime: &str) -> bool {
    mime != "application/octet-stream"
}

#[derive(Serialize)]
pub struct Media {
    pub id: String,
    pub filename: String,
    pub mime: String,
    pub size: i64,
    pub created_at: i64,
    pub url: String,
    /// Every document that embeds this file, which may be none or several.
    pub node_ids: Vec<i64>,
}

fn row_to_media(row: &sqlx::sqlite::SqliteRow) -> Media {
    let id: String = row.get("id");
    // `group_concat` gives "3,7" or NULL; queries that do not ask for it at all
    // (a fresh upload) get an empty list.
    let node_ids = row
        .try_get::<Option<String>, _>("node_ids")
        .ok()
        .flatten()
        .map(|joined| joined.split(',').filter_map(|n| n.parse().ok()).collect())
        .unwrap_or_default();

    Media {
        url: format!("/api/media/{id}"),
        id,
        filename: row.get("filename"),
        mime: row.get("mime"),
        size: row.get("size"),
        created_at: row.get("created_at"),
        node_ids,
    }
}

/// The type an upload is stored as: what the browser declared, or what the
/// name suggests when it declared nothing, narrowed by [`sanitize_mime`].
pub(crate) fn mime_of(filename: &str, declared: Option<&str>) -> String {
    let raw = declared
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            mime_guess::from_path(filename)
                .first_or_octet_stream()
                .to_string()
        });
    sanitize_mime(&raw)
}

/// Stream one multipart field to `uploads/<id>`, a chunk at a time, so a file
/// as large as the limit never has to fit in memory. `budget` is what is left
/// of the request's allowance and shrinks by what was written.
///
/// The bytes land in `<id>.part` and are renamed into place only once the
/// field has arrived whole, so a dropped connection never leaves a truncated
/// file under a name the database might come to know. Returns the size; an
/// empty field writes nothing and returns 0.
pub(crate) async fn store_field(
    state: &AppState,
    field: &mut Field<'_>,
    id: &str,
    budget: &mut u64,
) -> AppResult<u64> {
    let path = state.config.uploads_dir().join(id);
    let partial = path.with_extension("part");

    let result = async {
        let mut file = tokio::fs::File::create(&partial).await?;
        let mut size = 0u64;
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|e| AppError::BadRequest(format!("could not read upload: {e}")))?
        {
            size += chunk.len() as u64;
            if size > *budget {
                return Err(too_large(state));
            }
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        Ok(size)
    }
    .await;

    match result {
        Ok(size) if size > 0 => {
            tokio::fs::rename(&partial, &path).await?;
            *budget -= size;
            Ok(size)
        }
        other => {
            let _ = tokio::fs::remove_file(&partial).await;
            other
        }
    }
}

pub(crate) fn too_large(state: &AppState) -> AppError {
    AppError::BadRequest(format!(
        "the upload is larger than the {} MB limit",
        state.config.max_upload_bytes / 1024 / 1024
    ))
}

pub async fn upload(
    _: Session,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> AppResult<Json<Vec<Media>>> {
    let mut uploaded = Vec::new();
    let mut budget = state.config.max_upload_bytes as u64;

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("malformed upload: {e}")))?
    {
        let filename = field
            .file_name()
            .map(str::to_string)
            .unwrap_or_else(|| "upload".to_string());
        let mime = mime_of(&filename, field.content_type());

        let id = Uuid::new_v4().to_string();
        let size = store_field(&state, &mut field, &id, &mut budget).await?;
        if size == 0 {
            continue;
        }

        let row = sqlx::query(
            "INSERT INTO media (id, filename, mime, size, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             RETURNING id, filename, mime, size, created_at",
        )
        .bind(&id)
        .bind(&filename)
        .bind(&mime)
        .bind(size as i64)
        .bind(crate::now())
        .fetch_one(&state.db)
        .await?;

        uploaded.push(row_to_media(&row));
    }

    if uploaded.is_empty() {
        return Err(AppError::BadRequest("no files in upload".into()));
    }
    state.backup.signal();
    Ok(Json(uploaded))
}

/// `POST /api/nodes/{id}/files` — every file in the body becomes a file node
/// in that space or folder, under the name it was uploaded with.
pub async fn upload_into(
    _: Session,
    State(state): State<AppState>,
    Path(parent): Path<i64>,
    mut multipart: Multipart,
) -> AppResult<Json<Vec<nodes::Node>>> {
    nodes::container(&state.db, parent).await?;

    let mut created = Vec::new();
    let mut budget = state.config.max_upload_bytes as u64;

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("malformed upload: {e}")))?
    {
        // A browser sends a bare name; anything path-shaped is cut to its last
        // segment, because the folder is the one in the URL.
        let filename = field
            .file_name()
            .map(|name| name.rsplit(['/', '\\']).next().unwrap_or(name).trim().to_string())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "upload".to_string());
        let mime = mime_of(&filename, field.content_type());

        let id = Uuid::new_v4().to_string();
        let size = store_field(&state, &mut field, &id, &mut budget).await?;
        if size == 0 {
            continue;
        }

        match insert_file(&state.db, parent, &id, &filename, &mime, size).await {
            Ok(node) => created.push(node),
            Err(e) => {
                let _ = tokio::fs::remove_file(state.config.uploads_dir().join(&id)).await;
                return Err(e);
            }
        }
    }

    if created.is_empty() {
        return Err(AppError::BadRequest("no files in upload".into()));
    }
    state.backup.signal();

    let mut out = Vec::with_capacity(created.len());
    for id in created {
        out.push(nodes::load(&state, id).await?);
    }
    Ok(Json(out))
}

/// The media row and the file node that owns it, in one transaction so the
/// tree never shows a file whose bytes it does not know about.
async fn insert_file(
    db: &SqlitePool,
    parent: i64,
    media_id: &str,
    filename: &str,
    mime: &str,
    size: u64,
) -> AppResult<i64> {
    let slug = nodes::unique_slug(db, Some(parent), &nodes::slugify(filename), None).await?;
    let now = crate::now();
    let mut tx = db.begin().await?;

    sqlx::query(
        "INSERT INTO media (id, filename, mime, size, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(media_id)
    .bind(filename)
    .bind(mime)
    .bind(size as i64)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    let id: i64 = sqlx::query(
        "INSERT INTO nodes (parent_id, kind, name, slug, created_at, updated_at, media_id)
         VALUES (?1, 'file', ?2, ?3, ?4, ?4, ?5)
         RETURNING id",
    )
    .bind(parent)
    .bind(filename)
    .bind(&slug)
    .bind(now)
    .bind(media_id)
    .fetch_one(&mut *tx)
    .await?
    .get("id");

    sqlx::query("INSERT INTO node_media (node_id, media_id) VALUES (?1, ?2)")
        .bind(id)
        .bind(media_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(id)
}

pub async fn list(_: Session, State(state): State<AppState>) -> AppResult<Json<Vec<Media>>> {
    let rows = sqlx::query(
        "SELECT m.id, m.filename, m.mime, m.size, m.created_at,
                (SELECT group_concat(nm.node_id)
                 FROM node_media nm WHERE nm.media_id = m.id) AS node_ids
         FROM media m ORDER BY m.created_at DESC LIMIT 500",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.iter().map(row_to_media).collect()))
}

pub async fn serve(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Response> {
    stream_media(&state, &id).await
}

pub async fn remove(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    delete_media(&state, &id).await?;
    state.backup.signal();
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Shared by the private and the public-share media handlers.
pub async fn stream_media(state: &AppState, id: &str) -> AppResult<Response> {
    let row = sqlx::query("SELECT mime, filename, size FROM media WHERE id = ?1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::NotFound)?;

    let filename: String = row.get("filename");
    let size: i64 = row.get("size");
    // Re-checked on the way out, not just on the way in, so rows written before
    // the allowlist existed cannot be served as something executable either.
    let mime = sanitize_mime(&row.get::<String, _>("mime"));
    let disposition = if is_inline(&mime) { "inline" } else { "attachment" };

    let file = tokio::fs::File::open(state.config.uploads_dir().join(id))
        .await
        .map_err(|_| AppError::NotFound)?;

    Ok((
        [
            (header::CONTENT_TYPE, mime),
            (header::CONTENT_LENGTH, size.to_string()),
            (header::CACHE_CONTROL, "private, max-age=31536000".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!(
                    "{disposition}; filename*=UTF-8''{}",
                    urlencode(&filename)
                ),
            ),
            // Belt and braces: never let the browser sniff past the type above,
            // and strip the ambient authority of the origin from the response.
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
            (
                header::CONTENT_SECURITY_POLICY,
                "sandbox; default-src 'none'".to_string(),
            ),
        ],
        Body::from_stream(ReaderStream::new(file)),
    )
        .into_response())
}

pub async fn delete_media(state: &AppState, id: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM media WHERE id = ?1")
        .bind(id)
        .execute(&state.db)
        .await?;
    let path = state.config.uploads_dir().join(id);
    if let Err(e) = tokio::fs::remove_file(&path).await {
        if e.kind() != std::io::ErrorKind::NotFound {
            return Err(e.into());
        }
    }
    Ok(())
}

/// Attach every media file the body embeds to this document, and detach the
/// ones it no longer mentions so they stop being reachable through its share
/// link. Other documents keep whatever they embed; this rewrites one node's rows.
pub async fn link_to_node(db: &SqlitePool, node_id: i64, body: &str) -> AppResult<()> {
    let referenced: BTreeSet<String> = MEDIA_REF
        .captures_iter(body)
        .map(|c| c[1].to_lowercase())
        .collect();

    // One transaction, so a document is never momentarily attached to nothing —
    // which would blank its images for anyone reading its share link.
    let mut tx = db.begin().await?;

    sqlx::query("DELETE FROM node_media WHERE node_id = ?1")
        .bind(node_id)
        .execute(&mut *tx)
        .await?;

    for id in &referenced {
        // Selecting from `media` rather than binding the id directly means a
        // body that still mentions a since-deleted file saves fine.
        sqlx::query(
            "INSERT OR IGNORE INTO node_media (node_id, media_id)
             SELECT ?1, id FROM media WHERE id = ?2",
        )
        .bind(node_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

/// The files embedded somewhere in `node_ids` and nowhere outside it — the ones
/// that become unreachable once those nodes are gone. A whole subtree is asked
/// about at once, because deleting a folder deletes everything under it and a
/// file shared between two of its documents is still exclusive to the subtree.
pub async fn exclusive_media(db: &SqlitePool, node_ids: &[i64]) -> AppResult<Vec<String>> {
    if node_ids.is_empty() {
        return Ok(Vec::new());
    }

    // Ids are i64 straight out of the database, so there is nothing to bind:
    // the list is built as literals to keep it one statement of any length.
    let list = node_ids
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(",");

    let rows = sqlx::query(&format!(
        "SELECT DISTINCT nm.media_id AS id
         FROM node_media nm
         WHERE nm.node_id IN ({list})
           AND NOT EXISTS (
               SELECT 1 FROM node_media other
               WHERE other.media_id = nm.media_id AND other.node_id NOT IN ({list})
           )"
    ))
    .fetch_all(db)
    .await?;

    Ok(rows.iter().map(|row| row.get("id")).collect())
}

fn urlencode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::sanitize_mime;

    #[test]
    fn keeps_types_that_cannot_execute() {
        for mime in ["image/png", "image/jpeg", "video/mp4", "audio/ogg", "text/plain", "application/pdf"] {
            assert_eq!(sanitize_mime(mime), mime);
        }
    }

    #[test]
    fn neutralises_script_capable_types() {
        for mime in [
            "text/html",
            "image/svg+xml",
            "application/xhtml+xml",
            "application/javascript",
            "text/html; charset=utf-8",
            "",
            "nonsense",
        ] {
            assert_eq!(sanitize_mime(mime), "application/octet-stream", "{mime}");
        }
    }

    #[test]
    fn normalises_case_and_parameters() {
        assert_eq!(sanitize_mime("IMAGE/PNG; charset=binary"), "image/png");
    }
}
