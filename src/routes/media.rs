use std::{collections::BTreeSet, sync::LazyLock};

use axum::{
    body::Body,
    extract::{multipart::Field, Multipart, Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sqlx::{Connection, Row, SqliteConnection, SqlitePool};
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::{
    auth::Session,
    error::{AppError, AppResult},
    routes::nodes,
    state::AppState,
    vault,
};

/// Media referenced from a document body always looks like `/api/media/<uuid>`,
/// which is how a document claims ownership of the files it embeds.
pub(crate) static MEDIA_REF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"/api/media/([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})",
    )
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
    /// Sealed in the vault: its bytes on disk are ciphertext.
    pub encrypted: bool,
    /// Every document that embeds this file, which may be none or several.
    pub node_ids: Vec<i64>,
    /// The file node it is, when it sits in the tree: deleted from there, not
    /// from the media list.
    pub file_node: Option<i64>,
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
        encrypted: row.try_get("encrypted").unwrap_or(false),
        node_ids,
        file_node: row.try_get("file_node").ok().flatten(),
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
///
/// With `seal`, the bytes are encrypted as they pass, so the plaintext never
/// touches the disk at all; the size returned is still the plaintext's.
pub(crate) async fn store_field(
    state: &AppState,
    field: &mut Field<'_>,
    id: &str,
    budget: &mut u64,
    seal: Option<&vault::Key>,
) -> AppResult<u64> {
    let path = state.config.uploads_dir().join(id);
    let partial = path.with_extension("part");

    let result = async {
        let mut file = tokio::fs::File::create(&partial).await?;
        let mut sealer = match seal {
            Some(key) => {
                let (sealer, header) = vault::Sealer::new(key, id)?;
                file.write_all(&header).await?;
                Some(sealer)
            }
            None => None,
        };
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
            match sealer.as_mut() {
                Some(sealer) => file.write_all(&sealer.push(&chunk)?).await?,
                None => file.write_all(&chunk).await?,
            }
        }
        if let Some(sealer) = sealer {
            file.write_all(&sealer.finish()?).await?;
        }
        file.flush().await?;
        // On disk before the rename, and so before any row names it: after a
        // power cut the database must not know a file whose bytes never landed.
        file.sync_all().await?;
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
        let size = store_field(&state, &mut field, &id, &mut budget, None).await?;
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

#[derive(Deserialize)]
pub struct UploadQuery {
    /// Seal the files into the vault as they arrive.
    #[serde(default)]
    pub encrypt: bool,
}

/// `POST /api/nodes/{id}/files` — every file in the body becomes a file node
/// in that space or folder, under the name it was uploaded with.
/// `?encrypt=true` seals them on the way in, which needs the vault open.
pub async fn upload_into(
    _: Session,
    State(state): State<AppState>,
    Path(parent): Path<i64>,
    Query(query): Query<UploadQuery>,
    mut multipart: Multipart,
) -> AppResult<Json<Vec<nodes::Node>>> {
    nodes::container(&mut *state.db.acquire().await?, parent).await?;
    let key = if query.encrypt {
        Some(state.vault.key().ok_or(AppError::Locked)?)
    } else {
        None
    };

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
            .map(|name| {
                name.rsplit(['/', '\\'])
                    .next()
                    .unwrap_or(name)
                    .trim()
                    .to_string()
            })
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "upload".to_string());
        let mime = mime_of(&filename, field.content_type());

        let id = Uuid::new_v4().to_string();
        let size = store_field(&state, &mut field, &id, &mut budget, key.as_deref()).await?;
        if size == 0 {
            continue;
        }

        match insert_file(
            &state.db,
            parent,
            &id,
            &filename,
            &mime,
            size,
            key.is_some(),
        )
        .await
        {
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
pub(crate) async fn insert_file(
    db: &SqlitePool,
    parent: i64,
    media_id: &str,
    filename: &str,
    mime: &str,
    size: u64,
    encrypted: bool,
) -> AppResult<i64> {
    let mut tx = crate::db::begin_write(db).await?;
    // Checked inside the transaction that takes it, so the slug is still
    // free when the node is written; the folder too, which may have gone
    // while the upload was streaming in.
    nodes::container(&mut tx, parent).await?;
    let slug = nodes::unique_slug(&mut tx, Some(parent), &nodes::slugify(filename), None).await?;
    let now = crate::now();

    sqlx::query(
        "INSERT INTO media (id, filename, mime, size, created_at, encrypted)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(media_id)
    .bind(filename)
    .bind(mime)
    .bind(size as i64)
    .bind(now)
    .bind(encrypted)
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

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

/// A page of the media list, newest first; the client asks for pages until
/// one comes back short.
pub async fn list(
    _: Session,
    State(state): State<AppState>,
    Query(page): Query<ListQuery>,
) -> AppResult<Json<Vec<Media>>> {
    let rows = sqlx::query(
        "SELECT m.id, m.filename, m.mime, m.size, m.created_at, m.encrypted,
                (SELECT f.id FROM nodes f WHERE f.media_id = m.id) AS file_node,
                (SELECT group_concat(nm.node_id)
                 FROM node_media nm JOIN nodes d ON d.id = nm.node_id
                 WHERE nm.media_id = m.id AND d.kind <> 'file') AS node_ids
         FROM media m ORDER BY m.created_at DESC, m.id LIMIT ?1 OFFSET ?2",
    )
    .bind(page.limit.unwrap_or(500).clamp(1, 2000))
    .bind(page.offset.unwrap_or(0).max(0))
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.iter().map(row_to_media).collect()))
}

pub async fn serve(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
) -> AppResult<Response> {
    stream_media(&state, &id, true, headers.get(header::RANGE)).await
}

/// The one `bytes=` range of a `Range` header, inclusive, against a file of
/// `size` bytes. `Ok(None)` serves the whole file: no header, one that cannot
/// be read, or several ranges — a whole file is always a valid answer.
/// `Err` is a range that starts past the end.
fn byte_range(
    header: Option<&axum::http::HeaderValue>,
    size: u64,
) -> Result<Option<(u64, u64)>, ()> {
    let Some(spec) = header
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.trim().strip_prefix("bytes="))
    else {
        return Ok(None);
    };
    if spec.contains(',') {
        return Ok(None);
    }
    let Some((from, to)) = spec.split_once('-') else {
        return Ok(None);
    };
    let (from, to) = (from.trim(), to.trim());
    let range = match (from.parse::<u64>(), to.parse::<u64>()) {
        // `bytes=-500`: the last 500.
        (Err(_), Ok(suffix)) if from.is_empty() => {
            if suffix == 0 || size == 0 {
                return Err(());
            }
            (size.saturating_sub(suffix), size - 1)
        }
        (Ok(start), Err(_)) if to.is_empty() => (start, size.saturating_sub(1)),
        (Ok(start), Ok(end)) if start <= end => (start, end.min(size.saturating_sub(1))),
        _ => return Ok(None),
    };
    if range.0 >= size {
        return Err(());
    }
    Ok(Some(range))
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

/// Shared by the private and the public-share media handlers. A sealed file
/// is only ever served to the owner, and only while the vault is open — a
/// share link is a door for strangers, and the vault is not behind it.
pub async fn stream_media(
    state: &AppState,
    id: &str,
    private: bool,
    range: Option<&axum::http::HeaderValue>,
) -> AppResult<Response> {
    let row = sqlx::query("SELECT mime, filename, size, encrypted FROM media WHERE id = ?1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::NotFound)?;

    let filename: String = row.get("filename");
    let size: i64 = row.get("size");
    let encrypted: bool = row.get("encrypted");
    // Re-checked on the way out, not just on the way in, so rows written before
    // the allowlist existed cannot be served as something executable either.
    let mime = sanitize_mime(&row.get::<String, _>("mime"));
    let disposition = if is_inline(&mime) {
        "inline"
    } else {
        "attachment"
    };

    // A PDF is shown in a frame of the file pane, by the browser's own viewer,
    // and that viewer will not start inside a sandbox. It runs no script of
    // the origin's either way, and nothing but the app may frame it.
    let (policy, frames) = if mime == "application/pdf" {
        ("default-src 'none'; frame-ancestors 'self'", "SAMEORIGIN")
    } else {
        ("sandbox; default-src 'none'", "DENY")
    };

    let key = match (encrypted, private) {
        (false, _) => None,
        (true, false) => return Err(AppError::NotFound),
        (true, true) => Some(state.vault.key().ok_or(AppError::Locked)?),
    };

    let size = size.max(0) as u64;
    // A video is seeked by asking for the bytes from a point on; without an
    // answer to that, seeking fails, and Safari will not play it at all.
    let (status, start, end) = match byte_range(range, size) {
        Ok(None) => (StatusCode::OK, 0, size.saturating_sub(1)),
        Ok(Some((start, end))) => (StatusCode::PARTIAL_CONTENT, start, end),
        Err(()) => {
            return Ok((
                StatusCode::RANGE_NOT_SATISFIABLE,
                [(header::CONTENT_RANGE, format!("bytes */{size}"))],
            )
                .into_response())
        }
    };
    let length = if size == 0 { 0 } else { end - start + 1 };

    let mut file = tokio::fs::File::open(state.config.uploads_dir().join(id))
        .await
        .map_err(|_| AppError::NotFound)?;
    let body = match key {
        None => {
            use tokio::io::{AsyncReadExt, AsyncSeekExt};
            file.seek(std::io::SeekFrom::Start(start)).await?;
            Body::from_stream(ReaderStream::new(file.take(length)))
        }
        Some(key) => Body::from_stream(open_stream(file, &key, id, start, length).await?),
    };

    let mut response = (
        status,
        [
            (header::CONTENT_TYPE, mime),
            (header::CONTENT_LENGTH, length.to_string()),
            (header::ACCEPT_RANGES, "bytes".to_string()),
            // An upload never changes under its id, so the browser may keep
            // it — unless it was sealed: the plaintext would then sit in the
            // browser's cache long after the vault had been shut.
            (
                header::CACHE_CONTROL,
                if encrypted {
                    "no-store"
                } else {
                    "private, max-age=31536000"
                }
                .to_string(),
            ),
            (
                header::CONTENT_DISPOSITION,
                format!("{disposition}; filename*=UTF-8''{}", urlencode(&filename)),
            ),
            // Belt and braces: never let the browser sniff past the type above,
            // and strip the ambient authority of the origin from the response.
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
            (header::CONTENT_SECURITY_POLICY, policy.to_string()),
            (header::X_FRAME_OPTIONS, frames.to_string()),
        ],
        body,
    )
        .into_response();
    if status == StatusCode::PARTIAL_CONTENT {
        let value = format!("bytes {start}-{end}/{size}");
        if let Ok(value) = axum::http::HeaderValue::from_str(&value) {
            response.headers_mut().insert(header::CONTENT_RANGE, value);
        }
    }
    Ok(response)
}

/// A sealed file as a stream of its plaintext, a record at a time, from
/// byte `start` for `length` bytes. A range starts at the record that holds
/// its first byte, so seeking into a sealed video costs one record, not the
/// whole file before it. The header is checked before the first byte goes
/// out, so a file that will not open is an error response rather than a
/// download that breaks off.
async fn open_stream(
    mut file: tokio::fs::File,
    key: &vault::Key,
    id: &str,
    start: u64,
    length: u64,
) -> AppResult<impl futures_util::Stream<Item = std::io::Result<axum::body::Bytes>>> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let len = file.metadata().await?.len();
    let mut header = [0u8; vault::HEADER];
    file.read_exact(&mut header).await?;
    let mut opener = vault::Opener::new(key, id, &header, len)?;
    let first = start / vault::CHUNK as u64;
    if first > 0 {
        opener.skip_to(first)?;
        file.seek(std::io::SeekFrom::Start(
            vault::HEADER as u64 + first * vault::RECORD as u64,
        ))
        .await?;
    }
    let skip = (start - first * vault::CHUNK as u64) as usize;

    Ok(futures_util::stream::try_unfold(
        (file, opener, skip, length),
        |(mut file, mut opener, skip, left)| async move {
            let len = opener.next_len();
            if len == 0 || left == 0 {
                return Ok(None);
            }
            let mut record = vec![0u8; len];
            file.read_exact(&mut record).await?;
            let plain = opener.open(&record).map_err(std::io::Error::other)?;
            let piece = &plain[skip.min(plain.len())..];
            let piece = &piece[..piece.len().min(usize::try_from(left).unwrap_or(usize::MAX))];
            let left = left - piece.len() as u64;
            Ok(Some((
                axum::body::Bytes::copy_from_slice(piece),
                (file, opener, 0, left),
            )))
        },
    ))
}

/// The id as it is stored and as it names the file on disk, or `None` for
/// anything that is not a uuid. Every id that reaches the filesystem from a
/// request goes through here first: `Path` is percent-decoded, so an id is
/// otherwise free to be `../diary.db`.
pub(crate) fn canonical_id(id: &str) -> Option<String> {
    Uuid::try_parse(id).ok().map(|uuid| uuid.to_string())
}

pub async fn delete_media(state: &AppState, id: &str) -> AppResult<()> {
    let id = canonical_id(id).ok_or(AppError::NotFound)?;
    let mut tx = crate::db::begin_write(&state.db).await?;
    // A file in the tree is its node's to delete: going round it here would
    // take the node out of its folder without the tree having been asked.
    let in_tree = sqlx::query("SELECT 1 FROM nodes WHERE media_id = ?1")
        .bind(&id)
        .fetch_optional(&mut *tx)
        .await?
        .is_some();
    if in_tree {
        return Err(AppError::BadRequest(
            "that is a file in the tree — delete it from its folder".into(),
        ));
    }
    let deleted = sqlx::query("DELETE FROM media WHERE id = ?1")
        .bind(&id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    // No row, no file of ours: nothing on disk is removed on the word of a
    // request alone.
    if deleted == 0 {
        return Err(AppError::NotFound);
    }
    tx.commit().await?;
    remove_upload(&state.config.uploads_dir(), &id).await;
    Ok(())
}

/// The bytes of an upload whose row is already gone. Called after the
/// commit, never before: a file a crash leaves behind is swept up later as an
/// orphan, where a row whose file had gone first would be a broken link.
pub async fn remove_upload(uploads: &std::path::Path, id: &str) {
    match tokio::fs::remove_file(uploads.join(id)).await {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!("could not remove upload {id}: {e}; the sweep will retry"),
    }
}

/// Attach every media file the body embeds to this document, and detach the
/// ones it no longer mentions so they stop being reachable through its share
/// link. Other documents keep whatever they embed; this rewrites one node's rows.
pub async fn link_to_node(db: &mut SqliteConnection, node_id: i64, body: &str) -> AppResult<()> {
    let referenced: BTreeSet<String> = MEDIA_REF
        .captures_iter(body)
        .map(|c| c[1].to_lowercase())
        .collect();

    // One transaction — a savepoint inside the caller's, when there is one —
    // so a document is never momentarily attached to nothing, which would
    // blank its images for anyone reading its share link.
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
pub async fn exclusive_media(
    db: &mut SqliteConnection,
    node_ids: &[i64],
) -> AppResult<Vec<String>> {
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
    .fetch_all(&mut *db)
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
    use super::{byte_range, canonical_id, sanitize_mime};

    #[test]
    fn ranges_are_read_the_way_browsers_send_them() {
        let range =
            |raw: &'static str| byte_range(Some(&axum::http::HeaderValue::from_static(raw)), 1000);
        assert_eq!(byte_range(None, 1000), Ok(None));
        assert_eq!(range("bytes=0-"), Ok(Some((0, 999))));
        assert_eq!(range("bytes=100-199"), Ok(Some((100, 199))));
        assert_eq!(range("bytes=900-5000"), Ok(Some((900, 999))));
        assert_eq!(range("bytes=-100"), Ok(Some((900, 999))));
        assert_eq!(range("bytes=1000-"), Err(()));
        // Not understood, or several at once: the whole file is the answer.
        assert_eq!(range("bytes=0-1,5-9"), Ok(None));
        assert_eq!(range("items=0-1"), Ok(None));
        assert_eq!(range("bytes=9-1"), Ok(None));
    }

    #[test]
    fn only_a_uuid_is_an_id() {
        let id = "0b9e3c1a-5f2d-4e7a-9c3b-1d2e3f4a5b6c";
        assert_eq!(canonical_id(id).as_deref(), Some(id));
        assert_eq!(canonical_id(&id.to_uppercase()).as_deref(), Some(id));
        for bad in ["../diary.db", "/etc/passwd", "", "..", "0b9e3c1a/../../x"] {
            assert_eq!(canonical_id(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn keeps_types_that_cannot_execute() {
        for mime in [
            "image/png",
            "image/jpeg",
            "video/mp4",
            "audio/ogg",
            "text/plain",
            "application/pdf",
        ] {
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
