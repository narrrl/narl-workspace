//! The text of files in the tree, for search.
//!
//! A file's bytes live in `uploads/`, out of reach of SQLite's full-text
//! index, so the text of the ones that are text is copied into `file_text`
//! beside them. There is one way in, [`catch_up`]: it reads every file that
//! has no row yet. Whatever puts a file in the tree — an upload, an import, an
//! SFTP save, a decrypt — calls it once its own transaction is done, and the
//! server calls it at start for anything that slipped past. The triggers of
//! 0024 take care of the other direction: new bytes or sealing drop the row,
//! and a sealed file never gets one.
//!
//! PDF and office files are not read. Their text needs a parser this binary
//! does not carry, and it waits until it is missed.

use std::path::Path;

use sqlx::{Row, SqlitePool};
use tokio::io::AsyncReadExt;

use crate::error::AppResult;

/// As much of a file as is indexed — the same as the viewer shows.
pub const LIMIT: usize = 256 * 1024;

/// How much of a file of no telling type is looked at to decide it is text.
const SNIFF: usize = 8 * 1024;

/// Types whose bytes are text whether or not they say `text/`.
const TEXT_TYPES: [&str; 8] = [
    "application/json",
    "application/xml",
    "application/javascript",
    "application/x-sh",
    "application/toml",
    "application/yaml",
    "application/x-yaml",
    "application/sql",
];

/// The text of the start of a file, or `None` when it is not text: its type
/// says so, or its first bytes are valid UTF-8 without a NUL among them. A
/// cut at `LIMIT` that splits a character drops the half.
pub fn text_of(mime: &str, bytes: &[u8]) -> Option<String> {
    let bytes = &bytes[..bytes.len().min(LIMIT)];
    let start = &bytes[..bytes.len().min(SNIFF)];
    if start.contains(&0) {
        return None;
    }
    let mime = mime.split(';').next().unwrap_or(mime).trim();
    let typed = mime.starts_with("text/") || TEXT_TYPES.contains(&mime) || mime.ends_with("+xml");
    if !typed && !utf8_but_for_a_cut(start) {
        return None;
    }
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(e) if e.error_len().is_none() => {
            String::from_utf8_lossy(&bytes[..e.valid_up_to()]).into_owned()
        }
        // Text by its type but not UTF-8 — Latin-1, most likely. The words in
        // ASCII are still worth finding.
        Err(_) if typed => String::from_utf8_lossy(bytes).into_owned(),
        Err(_) => return None,
    };
    Some(text)
}

/// Valid UTF-8, allowing for a character cut in half at the end.
fn utf8_but_for_a_cut(bytes: &[u8]) -> bool {
    match std::str::from_utf8(bytes) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none(),
    }
}

/// Read every unsealed file in the tree that has no row yet. Returns how many
/// were read. A file whose bytes are missing is skipped and asked about again
/// next time; one that is not text gets an empty row and is not.
pub async fn catch_up(db: &SqlitePool, uploads: &Path) -> AppResult<usize> {
    let missing = sqlx::query(
        "SELECT n.id, n.media_id, m.mime
         FROM nodes n
         JOIN media m ON m.id = n.media_id
         LEFT JOIN file_text t ON t.node_id = n.id
         WHERE n.kind = 'file' AND m.encrypted = 0 AND t.node_id IS NULL",
    )
    .fetch_all(db)
    .await?;

    let mut read = 0;
    for row in &missing {
        let (id, media_id, mime): (i64, String, String) =
            (row.get("id"), row.get("media_id"), row.get("mime"));
        let bytes = match head(&uploads.join(&media_id)).await {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::warn!(node = id, "could not read a file for search: {e}");
                continue;
            }
        };
        let text = text_of(&mime, &bytes).unwrap_or_default();
        // Only while the node still has these bytes: an SFTP save that came
        // in meanwhile has newer ones, and the next pass reads those. The
        // trigger refuses a file sealed meanwhile; that is no reason to stop.
        let stored = sqlx::query(
            "INSERT OR IGNORE INTO file_text (node_id, media_id, text)
             SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM nodes WHERE id = ?1 AND media_id = ?2)",
        )
        .bind(id)
        .bind(&media_id)
        .bind(&text)
        .execute(db)
        .await;
        match stored {
            Ok(done) if done.rows_affected() > 0 => read += 1,
            Ok(_) => {}
            Err(e) => tracing::warn!(node = id, "could not index a file for search: {e}"),
        }
    }
    Ok(read)
}

/// [`catch_up`] after a write, where a failure is worth a line in the log
/// but not the write's answer: the file is there either way, and the next
/// start reads it.
pub async fn refresh(db: &SqlitePool, uploads: &Path) {
    if let Err(e) = catch_up(db, uploads).await {
        tracing::warn!("could not index files for search: {e:?}");
    }
}

async fn head(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    tokio::fs::File::open(path)
        .await?
        .take(LIMIT as u64)
        .read_to_end(&mut bytes)
        .await?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::{catch_up, text_of, LIMIT};

    #[test]
    fn text_is_told_by_its_type_or_its_bytes() {
        assert_eq!(text_of("text/plain", b"hello").as_deref(), Some("hello"));
        assert_eq!(text_of("application/json", b"{}").as_deref(), Some("{}"));
        assert_eq!(
            text_of("image/svg+xml", b"<svg/>").as_deref(),
            Some("<svg/>")
        );
        // No telling type, but it reads as UTF-8: a `.log`, a `Makefile`.
        assert_eq!(
            text_of("application/octet-stream", "grüße".as_bytes()).as_deref(),
            Some("grüße")
        );
        assert_eq!(
            text_of("application/octet-stream", b"\x89PNG\r\n\x1a\n\0\0"),
            None
        );
        assert_eq!(
            text_of("application/pdf", b"%PDF-1.7\n\xe2\xe3\xcf\xd3"),
            None
        );
        assert_eq!(text_of("text/plain", b"a\0b"), None);
        // Latin-1 that says it is text keeps its ASCII words.
        assert!(text_of("text/plain", b"caf\xe9 au lait")
            .unwrap()
            .contains("au lait"));
    }

    #[test]
    fn a_long_file_is_cut_at_the_limit_on_a_character() {
        let mut bytes = vec![b'a'; LIMIT - 1];
        bytes.extend("ü and more".as_bytes());
        let text = text_of("text/plain", &bytes).unwrap();
        assert_eq!(text.len(), LIMIT - 1);
        assert!(text.bytes().all(|b| b == b'a'));
    }

    async fn file(state: &crate::state::AppState, name: &str, bytes: &[u8], sealed: bool) -> i64 {
        let space: i64 = sqlx::query_scalar(
            "SELECT id FROM nodes WHERE parent_id IS NULL AND role IS NULL ORDER BY id LIMIT 1",
        )
        .fetch_one(&state.db)
        .await
        .unwrap();
        let media_id = uuid::Uuid::new_v4().to_string();
        std::fs::write(state.config.uploads_dir().join(&media_id), bytes).unwrap();
        let mime = crate::routes::media::mime_of(name, None);
        crate::routes::media::insert_file(
            &state.db,
            space,
            &media_id,
            name,
            &mime,
            bytes.len() as u64,
            sealed,
        )
        .await
        .unwrap()
    }

    async fn found(state: &crate::state::AppState, q: &str) -> Vec<(String, bool, String)> {
        use axum::extract::{Query, State};
        let query = serde_json::from_value(serde_json::json!({ "q": q })).unwrap();
        crate::routes::nodes::list(
            crate::auth::Session { id: "test".into() },
            State(state.clone()),
            Query(query),
        )
        .await
        .unwrap()
        .0
        .into_iter()
        .map(|node| (node.name, node.in_text, node.excerpt))
        .collect()
    }

    /// A text file is found by what it says, marked as found in its text and
    /// shown with the words around the match; a binary file is not read.
    #[tokio::test]
    async fn a_text_file_is_found_by_what_it_says() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::routes::tests::state_in(dir.path().into()).await;
        std::fs::create_dir_all(state.config.uploads_dir()).unwrap();
        let notes = file(&state, "notes.txt", b"the otter sleeps by the river", false).await;
        file(
            &state,
            "photo.jpg",
            b"\xff\xd8\xff\xe0\0\x10JFIF otter",
            false,
        )
        .await;

        assert_eq!(
            catch_up(&state.db, &state.config.uploads_dir())
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            catch_up(&state.db, &state.config.uploads_dir())
                .await
                .unwrap(),
            0
        );
        let hits = found(&state, "otter").await;
        assert_eq!(hits.len(), 1);
        let (name, in_text, excerpt) = &hits[0];
        assert_eq!((name.as_str(), *in_text), ("notes.txt", true));
        assert!(excerpt.contains("otter sleeps"), "{excerpt}");
        // Found by its name, it is not marked.
        assert!(!found(&state, "notes").await[0].1);

        // Deleted, it is gone from the index with it.
        sqlx::query("DELETE FROM nodes WHERE id = ?1")
            .bind(notes)
            .execute(&state.db)
            .await
            .unwrap();
        assert!(found(&state, "otter").await.is_empty());
    }

    /// A sealed file is never read, a file that gets sealed drops out at
    /// once, and the trigger refuses its text whatever tries to store it.
    #[tokio::test]
    async fn a_sealed_file_is_never_found() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::routes::tests::state_in(dir.path().into()).await;
        std::fs::create_dir_all(state.config.uploads_dir()).unwrap();
        file(&state, "secret.txt", b"badger plans", true).await;
        let open = file(&state, "open.txt", b"heron plans", false).await;
        catch_up(&state.db, &state.config.uploads_dir())
            .await
            .unwrap();
        assert!(found(&state, "badger").await.is_empty());
        assert_eq!(found(&state, "heron").await.len(), 1);

        // What `:encrypt` does to the row: the node gets new, sealed bytes.
        let old: String = sqlx::query_scalar("SELECT media_id FROM nodes WHERE id = ?1")
            .bind(open)
            .fetch_one(&state.db)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO media (id, filename, mime, size, created_at, encrypted)
             SELECT 'sealed-heron', filename, mime, size, created_at, 1 FROM media WHERE id = ?1",
        )
        .bind(&old)
        .execute(&state.db)
        .await
        .unwrap();
        sqlx::query("UPDATE nodes SET media_id = 'sealed-heron' WHERE id = ?1")
            .bind(open)
            .execute(&state.db)
            .await
            .unwrap();
        assert!(found(&state, "heron").await.is_empty());
        assert_eq!(
            catch_up(&state.db, &state.config.uploads_dir())
                .await
                .unwrap(),
            0
        );

        let refused = sqlx::query(
            "INSERT INTO file_text (node_id, media_id, text) VALUES (?1, 'sealed-heron', 'heron')",
        )
        .bind(open)
        .execute(&state.db)
        .await;
        assert!(refused.is_err());
    }

    /// New bytes over SFTP: the old text goes with the old bytes, and the
    /// next pass reads the new ones.
    #[tokio::test]
    async fn new_bytes_are_read_again() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::routes::tests::state_in(dir.path().into()).await;
        let uploads = state.config.uploads_dir();
        std::fs::create_dir_all(&uploads).unwrap();
        let id = file(&state, "todo.md", b"buy lentils", false).await;
        catch_up(&state.db, &uploads).await.unwrap();

        std::fs::write(uploads.join("v2"), b"buy chickpeas").unwrap();
        sqlx::query(
            "INSERT INTO media (id, filename, mime, size, created_at) VALUES ('v2', 'todo.md', 'text/markdown', 13, 0)",
        )
        .execute(&state.db)
        .await
        .unwrap();
        sqlx::query("UPDATE nodes SET media_id = 'v2' WHERE id = ?1")
            .bind(id)
            .execute(&state.db)
            .await
            .unwrap();
        assert!(found(&state, "lentils").await.is_empty());
        catch_up(&state.db, &uploads).await.unwrap();
        assert_eq!(found(&state, "chickpeas").await.len(), 1);
    }
}
