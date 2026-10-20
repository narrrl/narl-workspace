//! The virtual filesystem an SFTP client sees: `/<space>/<folder>/.../<doc>.md`
//! and the files beside them. Directories are spaces and folders; documents
//! are `<slug>.md`, and file nodes appear under their own name — the one a
//! download of them gets — with a later file that shares it told apart by its
//! id, `slides (42).pdf`. There is no on-disk mirror — every call reads or
//! writes `nodes` directly, so a save here is a save in the browser too.
//!
//! A document reads as `# <title>` followed by its body, the same shape the
//! export writes, and a leading heading written back becomes the title again.
//! A file is streamed from `uploads/` and written back to it through a
//! `.part`, never held in memory. A sealed file opens here while the vault is
//! open in the browser and the session's key has been given vault access,
//! decrypted a record at a time as it is read, and is sealed again when it is
//! written over; otherwise it is listed but refused, and locking the vault
//! cuts off a read already under way. A file written into a sealed folder, or
//! moved into one, is sealed while the vault is open and refused while it is
//! shut. Nothing a mount does keeps the vault open: only the browser does.
//!
//! What an editor writes next to a document — vim's `.swp` and `4913` probe,
//! a `foo.md~` backup, a dotted temp file it renames over the original — lives
//! in memory for the length of the session. Renaming one of those onto a
//! `.md` name saves it as a document, onto any other name as a file.

use std::{collections::HashMap, sync::Arc};

use russh_sftp::protocol::{File, FileAttributes, Handle, Name, OpenFlags, StatusCode, Version};
use sqlx::{Row, SqlitePool};

use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use zeroize::Zeroizing;

use crate::{
    backup::{
        view::{clean, disambiguate},
        Backup,
    },
    error::{AppError, AppResult},
    routes::{media, nodes, revisions, seal, trash},
    state::AppState,
    vault,
};

type SftpResult<T> = Result<T, StatusCode>;

/// The largest file this filesystem holds, document or scratch. Everything is
/// buffered in memory until it is closed, so there has to be a ceiling.
const MAX_FILE: usize = 16 * 1024 * 1024;

/// What one session may hold in memory at once — documents and scratch files
/// open for writing, and the scratch files kept — and how many handles it may
/// have open. Far more than an editor or a sync tool needs; a runaway client
/// is stopped before it takes the server's memory with it.
const MAX_HELD: usize = 64 * 1024 * 1024;
const MAX_HANDLES: usize = 256;

/// The byte size of a document as [`render_doc`] lays it out, computed in SQL
/// so a directory listing does not have to fetch every body.
const DOC_SIZE: &str = "length(CAST(body AS BLOB))
    + CASE WHEN trim(name) <> '' THEN length(CAST(trim(name) AS BLOB)) + 4 ELSE 0 END";

/// What a path names.
enum Target {
    /// `/`, which holds the spaces and nothing else.
    Root,
    /// A space or a folder.
    Dir {
        id: i64,
        kind: String,
        slug: String,
        updated_at: i64,
    },
    /// A document, `<slug>.md`.
    Doc(Doc),
    /// A file node.
    File(FileNode),
    /// A session-only file, keyed by its normalised path.
    Scratch(String),
}

#[derive(Clone)]
struct FileNode {
    id: i64,
    media_id: String,
    size: i64,
    updated_at: i64,
    encrypted: bool,
}

/// Whether a name is one an editor or a sync tool writes for a moment —
/// a swap file, a backup, a write probe, a temp file to rename over the real
/// one — rather than a file anyone means to keep.
fn is_ephemeral(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    name.starts_with('.')
        || name.ends_with('~')
        || name.chars().all(|c| c.is_ascii_digit())
        || [".swp", ".swx", ".swo", ".tmp", ".part", ".crdownload"]
            .iter()
            .any(|ext| lower.ends_with(ext))
}

struct Doc {
    id: i64,
    name: String,
    body: String,
    updated_at: i64,
}

/// A file that is not a document: kept for as long as the session lasts.
struct Scratch {
    data: Vec<u8>,
    mtime: i64,
}

fn dir_attrs(mtime: i64) -> FileAttributes {
    let mut attrs = FileAttributes::empty();
    attrs.set_dir(true);
    attrs.permissions = Some(0o755 | attrs.permissions.unwrap_or(0));
    attrs.mtime = Some(mtime as u32);
    attrs.atime = Some(mtime as u32);
    attrs.size = Some(0);
    attrs
}

fn file_attrs(size: u64, mtime: i64) -> FileAttributes {
    let mut attrs = FileAttributes::empty();
    attrs.set_regular(true);
    attrs.permissions = Some(0o644 | attrs.permissions.unwrap_or(0));
    attrs.mtime = Some(mtime as u32);
    attrs.atime = Some(mtime as u32);
    attrs.size = Some(size);
    attrs
}

/// A path split into segments, `/` and `.` collapsed away and `..` taking the
/// previous segment with it. `..` at the root stays at the root.
fn segments(path: &str) -> Vec<&str> {
    let mut segs = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                segs.pop();
            }
            seg => segs.push(seg),
        }
    }
    segs
}

/// The one spelling of a path that scratch files are keyed by.
fn normalize(path: &str) -> String {
    format!("/{}", segments(path).join("/"))
}

/// Split a path into the directory that must already exist and the trailing
/// segment, still unresolved.
fn split_path(path: &str) -> (Vec<&str>, Option<&str>) {
    let mut segs = segments(path);
    let leaf = segs.pop();
    (segs, leaf)
}

/// A document's slug is its filename without the trailing `.md`. A dotfile is
/// never a document, whatever it ends in: that is where editors hide the
/// files they write beside one (`.notes.md.swp`, `.#notes.md`).
fn doc_slug(filename: &str) -> Option<&str> {
    if filename.starts_with('.') {
        return None;
    }
    filename.strip_suffix(".md").filter(|base| !base.is_empty())
}

/// A document as a file: its title as a heading, then its body. A document
/// with no title is just its body.
fn render_doc(name: &str, body: &str) -> String {
    match name.trim() {
        "" => body.to_string(),
        title => format!("# {title}\n\n{body}"),
    }
}

/// The inverse of [`render_doc`]: a leading `# heading` is the title, the
/// rest is the body exactly as written, trailing newline and all. Without a
/// heading there is no title to take — the caller decides what to keep.
fn split_heading(text: &str) -> (Option<String>, String) {
    let (first, rest) = text.split_once('\n').unwrap_or((text, ""));
    match first.trim().strip_prefix("# ").map(str::trim) {
        Some(title) if !title.is_empty() => (
            Some(title.to_string()),
            rest.trim_start_matches(['\r', '\n']).to_string(),
        ),
        _ => (None, text.to_string()),
    }
}

fn write_at(buf: &mut Vec<u8>, offset: u64, data: &[u8]) -> SftpResult<()> {
    // Checked before anything is cast: an offset is the client's to choose,
    // and one near `u64::MAX` must not wrap past the size limit.
    let end = offset
        .checked_add(data.len() as u64)
        .filter(|&end| end <= MAX_FILE as u64)
        .ok_or(StatusCode::Failure)? as usize;
    let offset = offset as usize;
    if buf.len() < end {
        buf.resize(end, 0);
    }
    buf[offset..end].copy_from_slice(data);
    Ok(())
}

enum OpenHandle {
    /// Directory listing, already fetched: `readdir` just drains it.
    Dir(Vec<File>),
    /// A file's contents, fetched once at `open`. Read-only handles never
    /// touch the database again after this.
    Read(Vec<u8>),
    /// A document being written. The node already exists — it is created at
    /// `open`, so a client that stats the file before closing it finds it —
    /// and the buffer is saved into it when the handle closes. `base` is the
    /// document's `updated_at` when it was opened, so a save from the browser
    /// in between is noticed rather than overwritten.
    Write { id: i64, buf: Vec<u8>, base: i64 },
    /// A file node's bytes, read from disk as asked for.
    ReadFile { file: tokio::fs::File, size: u64 },
    /// A sealed file node's plaintext, decrypted as asked for.
    ReadSealed(Box<SealedRead>),
    /// A file being written to `uploads/<id>.part`, stored on close.
    Upload(Upload),
    /// A scratch file being written, stored back under `path` on close.
    Scratch { path: String, buf: Vec<u8> },
}

/// A sealed file open for reading. Reads come at any offset, so each opens
/// the record it falls in; the last one is kept for the reads after it,
/// which usually ask for the rest of it.
struct SealedRead {
    file: tokio::fs::File,
    opener: vault::Opener,
    /// The plaintext's size, which is what the listing shows.
    size: u64,
    record: Option<(u64, Zeroizing<Vec<u8>>)>,
}

impl SealedRead {
    async fn read(&mut self, offset: u64, len: u32) -> SftpResult<Vec<u8>> {
        if offset >= self.size {
            return Err(StatusCode::Eof);
        }
        let end = offset + (len as u64).min(self.size - offset);
        let mut data = Vec::with_capacity((end - offset) as usize);
        let mut at = offset;
        while at < end {
            let index = at / vault::CHUNK as u64;
            if !matches!(&self.record, Some((open, _)) if *open == index) {
                self.record = Some((index, self.open_record(index).await?));
            }
            let Some((_, plain)) = &self.record else {
                unreachable!("just opened")
            };
            let start = (at - index * vault::CHUNK as u64) as usize;
            let take = ((end - at) as usize).min(plain.len().saturating_sub(start));
            if take == 0 {
                // Shorter than the size it is listed with.
                return Err(StatusCode::Failure);
            }
            data.extend_from_slice(&plain[start..start + take]);
            at += take as u64;
        }
        Ok(data)
    }

    async fn open_record(&mut self, index: u64) -> SftpResult<Zeroizing<Vec<u8>>> {
        self.opener
            .skip_to(index)
            .map_err(|_| StatusCode::Failure)?;
        let mut record = vec![0u8; self.opener.next_len()];
        self.file
            .seek(std::io::SeekFrom::Start(
                vault::HEADER as u64 + index * vault::RECORD as u64,
            ))
            .await
            .map_err(|_| StatusCode::Failure)?;
        self.file
            .read_exact(&mut record)
            .await
            .map_err(|_| StatusCode::Failure)?;
        match self.opener.open(&record) {
            Ok(plain) => Ok(Zeroizing::new(plain)),
            Err(e) => {
                tracing::warn!("sftp: could not open a sealed file: {e:?}");
                Err(StatusCode::Failure)
            }
        }
    }
}

struct Upload {
    parent: i64,
    name: String,
    /// The file node it replaces, when it was opened over one.
    replaces: Option<i64>,
    /// Whether it is stored sealed: written into a sealed folder, or over a
    /// sealed file.
    sealed: bool,
    media_id: String,
    partial: Partial,
    file: tokio::fs::File,
    size: u64,
}

/// The `.part` an upload is written to. Removed when the upload is dropped
/// unfinished — a session that ends, a client that goes away mid-write — so
/// the plaintext of a sealed file opened for writing does not sit in
/// `uploads/` until the next sweep. After a finished upload it is gone
/// already, renamed or sealed into place.
struct Partial(std::path::PathBuf);

impl std::ops::Deref for Partial {
    type Target = std::path::Path;

    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Partial {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub struct SpaceFs {
    db: SqlitePool,
    /// The key this session logged in with; `None` only in tests.
    key_id: Option<i64>,
    /// The ceiling on one file, the same as for an upload in the browser.
    max_upload: u64,
    /// The `updated_at` each document got from this session's own last
    /// save, so that saving twice through one handle is not taken for an
    /// edit from somewhere else.
    own_writes: HashMap<i64, i64>,
    /// Where the bytes of uploads live, so a delete here removes them too.
    uploads: std::path::PathBuf,
    /// `None` only in tests, where there is nothing to mirror to.
    backup: Option<Arc<Backup>>,
    handles: HashMap<String, OpenHandle>,
    next_handle: u64,
    scratch: HashMap<String, Scratch>,
    /// For the vault: a file written into a sealed folder is sealed with its
    /// key. `None` only in tests, where a sealed folder takes nothing.
    state: Option<AppState>,
}

impl SpaceFs {
    pub fn new(state: AppState, key_id: Option<i64>) -> Self {
        let mut fs = Self::with_db(
            state.db.clone(),
            state.config.uploads_dir(),
            Some(state.backup.clone()),
        );
        fs.key_id = key_id;
        fs.max_upload = state.config.max_upload_bytes as u64;
        fs.state = Some(state);
        fs
    }

    fn with_db(db: SqlitePool, uploads: std::path::PathBuf, backup: Option<Arc<Backup>>) -> Self {
        Self {
            db,
            key_id: None,
            max_upload: 64 * 1024 * 1024,
            own_writes: HashMap::new(),
            uploads,
            backup,
            handles: HashMap::new(),
            next_handle: 0,
            scratch: HashMap::new(),
            state: None,
        }
    }

    /// The key to seal a file written into `parent` with: none when neither
    /// the folder nor the file it is written over is sealed, and a refusal
    /// when one of them is and the vault is shut.
    async fn seal_key(
        &self,
        parent: i64,
        over_sealed: bool,
    ) -> SftpResult<Option<Arc<vault::Keyring>>> {
        let sealed: bool = sqlx::query_scalar("SELECT sealed FROM nodes WHERE id = ?1")
            .bind(parent)
            .fetch_optional(&self.db)
            .await
            .map_err(|_| StatusCode::Failure)?
            .unwrap_or(false);
        if !sealed && !over_sealed {
            return Ok(None);
        }
        self.vault_key().map(Some)
    }

    /// The vault's key, while it is open; a refusal while it is shut. Taken
    /// without counting as a use, so a mount never holds the vault open.
    fn vault_key(&self) -> SftpResult<Arc<vault::Keyring>> {
        self.state
            .as_ref()
            .and_then(|state| state.vault.peek_key())
            .ok_or(StatusCode::PermissionDenied)
    }

    /// Refuse unless the session's key may read sealed files. A key is a
    /// standing credential on some other machine; while the vault is open,
    /// only the keys given vault access in the browser read plaintext.
    /// Asked afresh each time, so taking the access away takes effect at once.
    async fn vault_access(&self) -> SftpResult<()> {
        let Some(key) = self.key_id else {
            return Ok(());
        };
        let allowed: Option<bool> = sqlx::query_scalar("SELECT vault FROM ssh_keys WHERE id = ?1")
            .bind(key)
            .fetch_optional(&self.db)
            .await
            .map_err(|_| StatusCode::Failure)?;
        match allowed {
            Some(true) => Ok(()),
            Some(false) => {
                tracing::info!(
                    key,
                    "sftp: this key has no vault access, a sealed file stays shut"
                );
                Err(StatusCode::PermissionDenied)
            }
            None => {
                tracing::warn!(key, "sftp: the session's key has been removed, refusing");
                Err(StatusCode::PermissionDenied)
            }
        }
    }

    /// Plaintext of a sealed file leaves only while the vault is open and the
    /// key has vault access — checked on every read, not only at the open.
    async fn may_read_sealed(&self) -> SftpResult<()> {
        if !self.vault_open() {
            return Err(StatusCode::PermissionDenied);
        }
        self.vault_access().await
    }

    /// The bytes this session holds in memory: scratch files, and what is
    /// open for writing or was read into a buffer.
    fn held(&self) -> usize {
        let open: usize = self
            .handles
            .values()
            .map(|handle| match handle {
                OpenHandle::Read(data)
                | OpenHandle::Write { buf: data, .. }
                | OpenHandle::Scratch { buf: data, .. } => data.len(),
                _ => 0,
            })
            .sum();
        open + self
            .scratch
            .values()
            .map(|file| file.data.len())
            .sum::<usize>()
    }

    /// Whether the vault is open, asked without holding it open: a mount's
    /// listings and reads alone do not keep the key in memory.
    fn vault_open(&self) -> bool {
        self.state
            .as_ref()
            .is_some_and(|state| state.vault.is_open())
    }

    /// Seal what moves into a sealed folder before it moves; see `seal::enter`.
    async fn enter(&self, node: i64, parent: i64) -> SftpResult<()> {
        match &self.state {
            Some(state) => seal::enter(state, node, parent).await.map_err(|e| match e {
                AppError::Locked => StatusCode::PermissionDenied,
                e => {
                    tracing::warn!("sftp: could not seal what moved into a sealed folder: {e:?}");
                    StatusCode::Failure
                }
            }),
            None => self.seal_key(parent, false).await.map(|_| ()),
        }
    }

    fn changed(&self) {
        if let Some(backup) = &self.backup {
            backup.signal();
        }
    }

    /// Whether the key this session logged in with is still registered.
    /// Asked whenever a directory or a file is opened, written or closed, so a
    /// key deleted in the browser locks out a mount that is already up, not
    /// only the next.
    async fn authorized(&self) -> SftpResult<()> {
        let Some(key) = self.key_id else {
            return Ok(());
        };
        let known = sqlx::query("SELECT 1 FROM ssh_keys WHERE id = ?1")
            .bind(key)
            .fetch_optional(&self.db)
            .await
            .map_err(|_| StatusCode::Failure)?
            .is_some();
        if known {
            Ok(())
        } else {
            tracing::warn!(key, "sftp: the session's key has been removed, refusing");
            Err(StatusCode::PermissionDenied)
        }
    }

    /// The file nodes under `parent`, each by the name it shows as here.
    /// A name a document or a folder already has, or an earlier file, makes
    /// the later file carry its id.
    async fn files_in(&self, parent: i64) -> SftpResult<Vec<(String, FileNode)>> {
        let others =
            sqlx::query("SELECT kind, slug FROM nodes WHERE parent_id = ?1 AND kind <> 'file'")
                .bind(parent)
                .fetch_all(&self.db)
                .await
                .map_err(|_| StatusCode::Failure)?;
        let mut taken: std::collections::HashSet<String> = others
            .iter()
            .map(|row| {
                let slug: String = row.get("slug");
                if row.get::<String, _>("kind") == nodes::DOCUMENT {
                    format!("{slug}.md")
                } else {
                    slug
                }
            })
            .collect();

        let rows = sqlx::query(
            "SELECT n.id, n.name, n.slug, n.updated_at, n.media_id, m.size, m.encrypted
             FROM nodes n JOIN media m ON m.id = n.media_id
             WHERE n.parent_id = ?1 AND n.kind = 'file' ORDER BY n.id",
        )
        .bind(parent)
        .fetch_all(&self.db)
        .await
        .map_err(|_| StatusCode::Failure)?;

        Ok(rows
            .iter()
            .map(|row| {
                let id: i64 = row.get("id");
                let name = clean(&row.get::<String, _>("name"), &row.get::<String, _>("slug"));
                let shown = if taken.insert(name.clone()) {
                    name
                } else {
                    let other = disambiguate(&name, id);
                    taken.insert(other.clone());
                    other
                };
                let file = FileNode {
                    id,
                    media_id: row.get("media_id"),
                    size: row.get("size"),
                    updated_at: row.get("updated_at"),
                    encrypted: row.get("encrypted"),
                };
                (shown, file)
            })
            .collect())
    }

    async fn file_in(&self, parent: i64, leaf: &str) -> SftpResult<Option<FileNode>> {
        Ok(self
            .files_in(parent)
            .await?
            .into_iter()
            .find(|(shown, _)| shown == leaf)
            .map(|(_, file)| file))
    }

    fn allocate(&mut self, handle: OpenHandle) -> SftpResult<String> {
        if self.handles.len() >= MAX_HANDLES {
            tracing::warn!("sftp: {MAX_HANDLES} handles open in one session, refusing another");
            return Err(StatusCode::Failure);
        }
        self.next_handle += 1;
        let key = self.next_handle.to_string();
        self.handles.insert(key.clone(), handle);
        Ok(key)
    }

    /// A space or folder directly under `parent`, by slug — or by the slug its
    /// name would get, so `mkdir "My Folder"` can be stat-ed by that name.
    async fn dir_in(&self, parent: Option<i64>, leaf: &str) -> SftpResult<Option<Target>> {
        let slugged = nodes::slugify(leaf);
        for slug in [leaf, slugged.as_str()] {
            let row = sqlx::query(
                "SELECT id, kind, slug, updated_at FROM nodes
                 WHERE parent_id IS ?1 AND slug = ?2 AND kind IN ('space', 'folder')
                   AND role IS NOT 'trash'",
            )
            .bind(parent)
            .bind(slug)
            .fetch_optional(&self.db)
            .await
            .map_err(|_| StatusCode::Failure)?;
            if let Some(row) = row {
                return Ok(Some(Target::Dir {
                    id: row.get("id"),
                    kind: row.get("kind"),
                    slug: row.get("slug"),
                    updated_at: row.get("updated_at"),
                }));
            }
        }
        Ok(None)
    }

    /// A document under `parent` named by `leaf`. As with directories, a
    /// file created as `My Notes.md` lands as `my-notes.md` and still answers
    /// to the name it was created with.
    async fn doc_in(&self, parent: i64, leaf: &str) -> SftpResult<Option<Doc>> {
        let Some(stem) = doc_slug(leaf) else {
            return Ok(None);
        };
        let slugged = nodes::slugify(stem);
        for slug in [stem, slugged.as_str()] {
            let row = sqlx::query(
                "SELECT id, name, body, updated_at FROM nodes
                 WHERE parent_id = ?1 AND slug = ?2 AND kind = 'document'",
            )
            .bind(parent)
            .bind(slug)
            .fetch_optional(&self.db)
            .await
            .map_err(|_| StatusCode::Failure)?;
            if let Some(row) = row {
                return Ok(Some(Doc {
                    id: row.get("id"),
                    name: row.get("name"),
                    body: row.get("body"),
                    updated_at: row.get("updated_at"),
                }));
            }
        }
        Ok(None)
    }

    /// Resolve a path of directory segments to a node id, `None` meaning the
    /// root.
    async fn resolve_dir(&self, segs: &[&str]) -> SftpResult<Option<i64>> {
        let mut current: Option<i64> = None;
        for seg in segs {
            match self.dir_in(current, seg).await? {
                Some(Target::Dir { id, .. }) => current = Some(id),
                _ => return Err(StatusCode::NoSuchFile),
            }
        }
        Ok(current)
    }

    async fn target(&self, path: &str) -> SftpResult<Target> {
        let (dir, leaf) = split_path(path);
        let Some(leaf) = leaf else {
            return Ok(Target::Root);
        };
        let parent = self.resolve_dir(&dir).await?;
        if let Some(found) = self.dir_in(parent, leaf).await? {
            return Ok(found);
        }
        // The root holds spaces and nothing else.
        let Some(parent) = parent else {
            return Err(StatusCode::NoSuchFile);
        };
        if let Some(doc) = self.doc_in(parent, leaf).await? {
            return Ok(Target::Doc(doc));
        }
        if let Some(file) = self.file_in(parent, leaf).await? {
            return Ok(Target::File(file));
        }
        let key = normalize(path);
        if self.scratch.contains_key(&key) {
            return Ok(Target::Scratch(key));
        }
        Err(StatusCode::NoSuchFile)
    }

    /// Like [`target`](Self::target), with "nothing there" as `None` rather
    /// than an error — what `rename` and `mkdir` need to ask.
    async fn target_opt(&self, path: &str) -> SftpResult<Option<Target>> {
        match self.target(path).await {
            Ok(found) => Ok(Some(found)),
            Err(StatusCode::NoSuchFile) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn attrs_of(&self, target: &Target) -> FileAttributes {
        match target {
            Target::Root => dir_attrs(crate::now()),
            Target::Dir { updated_at, .. } => dir_attrs(*updated_at),
            Target::Doc(doc) => file_attrs(
                render_doc(&doc.name, &doc.body).len() as u64,
                doc.updated_at,
            ),
            Target::File(file) => {
                let mut attrs = file_attrs(file.size as u64, file.updated_at);
                // Listed, but not to be opened while the vault is shut.
                if file.encrypted && !self.vault_open() {
                    attrs.permissions = Some(0o100000);
                }
                attrs
            }
            Target::Scratch(key) => {
                let file = &self.scratch[key];
                file_attrs(file.data.len() as u64, file.mtime)
            }
        }
    }

    /// An empty document under `parent`, named after the file it was created
    /// as until a heading says otherwise.
    async fn create_doc(&mut self, parent: i64, stem: &str) -> SftpResult<i64> {
        let slug = nodes::unique_slug(
            &mut *self.conn().await?,
            Some(parent),
            &nodes::slugify(stem),
            None,
        )
        .await
        .map_err(|_| StatusCode::Failure)?;
        let id: i64 = sqlx::query(
            "INSERT INTO nodes (parent_id, kind, name, slug, body, created_at, updated_at)
             VALUES (?1, 'document', ?2, ?3, '', ?4, ?4) RETURNING id",
        )
        .bind(parent)
        .bind(stem)
        .bind(&slug)
        .bind(crate::now())
        .fetch_one(&self.db)
        .await
        .map_err(|_| StatusCode::Failure)?
        .get("id");
        self.own_writes.insert(id, crate::now());
        self.changed();
        Ok(id)
    }

    /// Save a file's bytes into a document. A leading heading renames it;
    /// without one the document keeps the title it had.
    async fn save_doc(&mut self, id: i64, bytes: Vec<u8>) -> SftpResult<()> {
        let text = String::from_utf8(bytes).map_err(|_| StatusCode::Failure)?;
        let (title, body) = split_heading(&text);
        let now = crate::now();
        self.own_writes.insert(id, now);
        // One transaction, so the text this replaces is kept as a revision
        // exactly when it is replaced.
        let mut tx = crate::db::begin_write(&self.db)
            .await
            .map_err(|_| StatusCode::Failure)?;
        revisions::keep(&mut tx, id, &body, revisions::Reason::Sftp)
            .await
            .map_err(|_| StatusCode::Failure)?;
        sqlx::query(
            "UPDATE nodes SET name = COALESCE(?1, name), body = ?2, updated_at = ?3 WHERE id = ?4",
        )
        .bind(title)
        .bind(&body)
        .bind(now)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|_| StatusCode::Failure)?;
        media::link_to_node(&mut tx, id, &body)
            .await
            .map_err(|_| StatusCode::Failure)?;
        tx.commit().await.map_err(|_| StatusCode::Failure)?;
        self.changed();
        Ok(())
    }

    /// `rm` and `rmdir`: into the trash, as a delete in the browser does.
    async fn delete_node(&self, id: i64) -> SftpResult<()> {
        trash::trash_subtree(&self.db, id)
            .await
            .map_err(|e| match e {
                AppError::NotFound => StatusCode::NoSuchFile,
                AppError::BadRequest(_) => StatusCode::PermissionDenied,
                _ => StatusCode::Failure,
            })?;
        self.changed();
        Ok(())
    }

    /// A rename onto a name that is taken: what was there is replaced, as
    /// `rename(2)` replaces it, and not kept in the trash — an editor or
    /// `rsync` saving that way would fill it with every save.
    async fn replace_node(&self, id: i64) -> SftpResult<()> {
        nodes::purge_subtree(&self.db, &self.uploads, id)
            .await
            .map_err(|e| match e {
                AppError::NotFound => StatusCode::NoSuchFile,
                AppError::BadRequest(_) => StatusCode::PermissionDenied,
                _ => StatusCode::Failure,
            })?;
        self.changed();
        Ok(())
    }

    async fn conn(&self) -> SftpResult<sqlx::pool::PoolConnection<sqlx::Sqlite>> {
        self.db.acquire().await.map_err(|_| StatusCode::Failure)
    }

    /// Close a document handle. If the document was saved from somewhere
    /// else while the handle was open, this write would undo that save
    /// without either side knowing — so it goes into a copy beside it
    /// instead, and both versions are kept.
    async fn finish_doc(&mut self, id: i64, buf: Vec<u8>, base: i64) -> SftpResult<()> {
        let row = sqlx::query("SELECT parent_id, slug, updated_at FROM nodes WHERE id = ?1")
            .bind(id)
            .fetch_optional(&self.db)
            .await
            .map_err(|_| StatusCode::Failure)?;
        let Some(row) = row else {
            // Deleted while open: like an unlinked file, the write is gone.
            return Ok(());
        };
        let current: i64 = row.get("updated_at");
        let ours = self.own_writes.get(&id).copied();
        if current > base && Some(current) != ours {
            let parent: i64 = row.get("parent_id");
            let slug: String = row.get("slug");
            let copy = self
                .create_doc(parent, &format!("{slug} (conflict)"))
                .await?;
            tracing::warn!(
                id,
                copy,
                "sftp: the document changed elsewhere while open; kept this write as a copy"
            );
            return self.save_doc(copy, buf).await;
        }
        self.save_doc(id, buf).await
    }

    /// Start writing a file: into a `.part` of a new upload, with what the
    /// file held already copied in unless the open truncates it.
    async fn begin_upload(
        &self,
        parent: i64,
        name: &str,
        replaces: Option<&FileNode>,
        truncate: bool,
    ) -> SftpResult<Upload> {
        // Refused at the open, not at the close, when the vault is shut.
        let over_sealed = replaces.is_some_and(|old| old.encrypted);
        let key = self.seal_key(parent, over_sealed).await?;
        // Written into rather than over, a sealed file is decrypted into the
        // `.part`, and what the handle holds can be read back through it.
        if over_sealed && !truncate {
            self.vault_access().await?;
        }
        let media_id = uuid::Uuid::new_v4().to_string();
        let partial = Partial(self.uploads.join(format!("{media_id}.part")));
        let mut size = 0;
        if let (Some(old), false) = (replaces, truncate) {
            let source = self.uploads.join(&old.media_id);
            size = match (&key, old.encrypted) {
                // Written into, not over: the plaintext goes into the
                // `.part`, which is sealed again when it is closed.
                (Some(key), true) => {
                    let (key, id, part) =
                        (key.clone(), old.media_id.clone(), partial.to_path_buf());
                    tokio::task::spawn_blocking(move || -> anyhow::Result<u64> {
                        let mut plain = key.reader(&id, &source)?;
                        Ok(std::io::copy(
                            &mut plain,
                            &mut std::fs::File::create(&part)?,
                        )?)
                    })
                    .await
                    .map_err(|_| StatusCode::Failure)?
                    .map_err(|e| {
                        tracing::warn!("sftp: could not open {name}: {e:?}");
                        StatusCode::Failure
                    })?
                }
                _ => tokio::fs::copy(source, &*partial)
                    .await
                    .map_err(|_| StatusCode::Failure)?,
            };
        }
        let file = tokio::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(truncate || replaces.is_none())
            .open(&*partial)
            .await
            .map_err(|_| StatusCode::Failure)?;
        Ok(Upload {
            parent,
            name: name.to_string(),
            replaces: replaces.map(|old| old.id),
            sealed: key.is_some(),
            media_id,
            partial,
            file,
            size,
        })
    }

    /// Put a finished upload in the tree: a new file node, or new bytes for
    /// the one it was opened over — under a new id, because the mirror treats
    /// an upload as never changing once written.
    async fn finish_upload(&self, upload: Upload) -> SftpResult<()> {
        let Upload {
            parent,
            name,
            replaces,
            sealed,
            media_id,
            partial,
            file,
            size,
        } = upload;
        // Asked again: the vault may have locked while the bytes came in.
        // Refused, the `.part` goes with `partial`.
        let key = self.seal_key(parent, sealed).await?;
        let landed = async {
            file.sync_all().await?;
            drop(file);
            let target = self.uploads.join(&media_id);
            match &key {
                // The plaintext sat only in the `.part` file, which no row
                // names and the mirror never reads.
                Some(key) => {
                    let (key, id, plain, sealed) =
                        (key.clone(), media_id.clone(), partial.to_path_buf(), target);
                    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                        vault::seal_file(
                            key.current(),
                            &id,
                            &mut std::fs::File::open(&plain)?,
                            &sealed,
                        )?;
                        std::fs::remove_file(&plain)?;
                        Ok(())
                    })
                    .await
                    .map_err(anyhow::Error::from)??;
                }
                None => tokio::fs::rename(&*partial, target).await?,
            }
            let mime = media::mime_of(&name, None);
            let sealed = key.is_some();
            match replaces {
                None => {
                    media::insert_file(&self.db, parent, &media_id, &name, &mime, size, sealed)
                        .await?;
                }
                Some(node) => {
                    self.replace_bytes(node, &media_id, &mime, size, sealed)
                        .await?
                }
            }
            AppResult::Ok(())
        }
        .await;
        if let Err(e) = landed {
            tracing::warn!("sftp: could not store {name}: {e:?}");
            drop(partial);
            let _ = tokio::fs::remove_file(self.uploads.join(&media_id)).await;
            return Err(StatusCode::Failure);
        }
        self.changed();
        Ok(())
    }

    async fn replace_bytes(
        &self,
        node: i64,
        media_id: &str,
        mime: &str,
        size: u64,
        encrypted: bool,
    ) -> AppResult<()> {
        let mut tx = crate::db::begin_write(&self.db).await?;
        let old: String = sqlx::query_scalar("SELECT media_id FROM nodes WHERE id = ?1")
            .bind(node)
            .fetch_one(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO media (id, filename, mime, size, created_at, encrypted)
             SELECT ?1, filename, ?2, ?3, ?4, ?6 FROM media WHERE id = ?5",
        )
        .bind(media_id)
        .bind(mime)
        .bind(size as i64)
        .bind(crate::now())
        .bind(&old)
        .bind(encrypted)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE nodes SET media_id = ?1, updated_at = ?2 WHERE id = ?3")
            .bind(media_id)
            .bind(crate::now())
            .bind(node)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE node_media SET media_id = ?1 WHERE node_id = ?2 AND media_id = ?3")
            .bind(media_id)
            .bind(node)
            .bind(&old)
            .execute(&mut *tx)
            .await?;
        // The old bytes stay while a document still embeds them.
        let embedded = sqlx::query("SELECT 1 FROM node_media WHERE media_id = ?1")
            .bind(&old)
            .fetch_optional(&mut *tx)
            .await?
            .is_some();
        if !embedded {
            sqlx::query("DELETE FROM media WHERE id = ?1")
                .bind(&old)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        if !embedded {
            media::remove_upload(&self.uploads, &old).await;
        }
        Ok(())
    }

    /// A scratch file renamed onto a name worth keeping becomes a file node.
    async fn keep_scratch(
        &self,
        parent: i64,
        name: &str,
        data: Vec<u8>,
        replaces: Option<&FileNode>,
    ) -> SftpResult<()> {
        let mut upload = self.begin_upload(parent, name, replaces, true).await?;
        upload
            .file
            .write_all(&data)
            .await
            .map_err(|_| StatusCode::Failure)?;
        upload.size = data.len() as u64;
        self.finish_upload(upload).await
    }

    /// Move a file node, and rename it when the name it lands under is not
    /// the one it already shows as.
    async fn move_file(
        &self,
        file: &FileNode,
        parent: i64,
        leaf: &str,
        shown: &str,
    ) -> SftpResult<()> {
        self.enter(file.id, parent).await?;
        let mut tx = crate::db::begin_write(&self.db)
            .await
            .map_err(|_| StatusCode::Failure)?;
        let name: String = if leaf == shown {
            sqlx::query_scalar("SELECT name FROM nodes WHERE id = ?1")
                .bind(file.id)
                .fetch_one(&mut *tx)
                .await
                .map_err(|_| StatusCode::Failure)?
        } else {
            leaf.to_string()
        };
        let slug = nodes::unique_slug(&mut tx, Some(parent), &nodes::slugify(&name), Some(file.id))
            .await
            .map_err(|_| StatusCode::Failure)?;
        sqlx::query(
            "UPDATE nodes SET parent_id = ?1, name = ?2, slug = ?3, updated_at = ?4 WHERE id = ?5",
        )
        .bind(parent)
        .bind(&name)
        .bind(&slug)
        .bind(crate::now())
        .bind(file.id)
        .execute(&mut *tx)
        .await
        .map_err(|_| StatusCode::Failure)?;
        sqlx::query("UPDATE media SET filename = ?1 WHERE id = ?2")
            .bind(&name)
            .bind(&file.media_id)
            .execute(&mut *tx)
            .await
            .map_err(|_| StatusCode::Failure)?;
        tx.commit().await.map_err(|_| StatusCode::Failure)?;
        self.changed();
        Ok(())
    }

    async fn listing(&self, path: &str) -> SftpResult<Vec<File>> {
        let segs = segments(path);
        let parent = self.resolve_dir(&segs).await?;
        let rows = sqlx::query(&format!(
            "SELECT kind, slug, updated_at, {DOC_SIZE} AS size FROM nodes
             WHERE parent_id IS ?1 AND kind <> 'file' AND role IS NOT 'trash'
             ORDER BY kind, slug"
        ))
        .bind(parent)
        .fetch_all(&self.db)
        .await
        .map_err(|_| StatusCode::Failure)?;

        let mut files: Vec<File> = rows
            .iter()
            .map(|row| {
                let kind: String = row.get("kind");
                let slug: String = row.get("slug");
                let mtime: i64 = row.get("updated_at");
                if kind == nodes::DOCUMENT {
                    let size: i64 = row.try_get("size").unwrap_or(0);
                    File::new(format!("{slug}.md"), file_attrs(size as u64, mtime))
                } else {
                    File::new(slug, dir_attrs(mtime))
                }
            })
            .collect();

        if let Some(parent) = parent {
            for (shown, file) in self.files_in(parent).await? {
                let attrs = self.attrs_of(&Target::File(file));
                files.push(File::new(shown, attrs));
            }
        }

        let dir = normalize(path);
        for (key, file) in &self.scratch {
            if let Some((parent, name)) = key.rsplit_once('/') {
                if parent == dir || (parent.is_empty() && dir == "/") {
                    files.push(File::new(
                        name,
                        file_attrs(file.data.len() as u64, file.mtime),
                    ));
                }
            }
        }
        Ok(files)
    }
}

impl russh_sftp::server::Handler for SpaceFs {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn init(
        &mut self,
        _version: u32,
        _extensions: HashMap<String, String>,
    ) -> SftpResult<Version> {
        Ok(Version::new())
    }

    async fn realpath(&mut self, id: u32, path: String) -> SftpResult<Name> {
        Ok(Name {
            id,
            files: vec![File::dummy(normalize(&path))],
        })
    }

    async fn lstat(&mut self, id: u32, path: String) -> SftpResult<russh_sftp::protocol::Attrs> {
        self.stat(id, path).await
    }

    async fn stat(&mut self, id: u32, path: String) -> SftpResult<russh_sftp::protocol::Attrs> {
        let target = self.target(&path).await?;
        Ok(russh_sftp::protocol::Attrs {
            id,
            attrs: self.attrs_of(&target),
        })
    }

    async fn fstat(&mut self, id: u32, handle: String) -> SftpResult<russh_sftp::protocol::Attrs> {
        let attrs = match self.handles.get(&handle) {
            Some(OpenHandle::Read(data)) => file_attrs(data.len() as u64, crate::now()),
            Some(OpenHandle::ReadFile { size, .. }) => file_attrs(*size, crate::now()),
            Some(OpenHandle::ReadSealed(sealed)) => file_attrs(sealed.size, crate::now()),
            Some(OpenHandle::Upload(upload)) => file_attrs(upload.size, crate::now()),
            Some(OpenHandle::Write { buf, .. } | OpenHandle::Scratch { buf, .. }) => {
                file_attrs(buf.len() as u64, crate::now())
            }
            Some(OpenHandle::Dir(_)) => dir_attrs(crate::now()),
            None => return Err(StatusCode::Failure),
        };
        Ok(russh_sftp::protocol::Attrs { id, attrs })
    }

    /// Permissions and times have nowhere to live here, so they are accepted
    /// and dropped; a size truncates, because editors truncate this way.
    async fn setstat(
        &mut self,
        id: u32,
        path: String,
        attrs: FileAttributes,
    ) -> SftpResult<russh_sftp::protocol::Status> {
        self.authorized().await?;
        let target = self.target(&path).await?;
        if let Some(size) = attrs.size {
            let size = size as usize;
            if size > MAX_FILE {
                return Err(StatusCode::Failure);
            }
            match target {
                Target::Doc(doc) => {
                    let mut bytes = render_doc(&doc.name, &doc.body).into_bytes();
                    bytes.resize(size, 0);
                    self.save_doc(doc.id, bytes).await?;
                }
                Target::Scratch(key) => {
                    let held = self.held();
                    if let Some(file) = self.scratch.get_mut(&key) {
                        if held.saturating_add(size.saturating_sub(file.data.len())) > MAX_HELD {
                            return Err(StatusCode::Failure);
                        }
                        file.data.resize(size, 0);
                        file.mtime = crate::now();
                    }
                }
                // Truncating a file in place is a write; it is done by opening
                // it for one.
                Target::File(file) if size as i64 != file.size => {
                    return Err(StatusCode::OpUnsupported)
                }
                Target::Root | Target::Dir { .. } | Target::File(_) => {}
            }
        }
        Ok(ok_status(id))
    }

    async fn fsetstat(
        &mut self,
        id: u32,
        handle: String,
        attrs: FileAttributes,
    ) -> SftpResult<russh_sftp::protocol::Status> {
        self.authorized().await?;
        let held = self.held();
        match self.handles.get_mut(&handle) {
            Some(OpenHandle::Write { buf, .. } | OpenHandle::Scratch { buf, .. }) => {
                if let Some(size) = attrs.size {
                    let grows = (size as usize).saturating_sub(buf.len());
                    if size as usize > MAX_FILE || held.saturating_add(grows) > MAX_HELD {
                        return Err(StatusCode::Failure);
                    }
                    buf.resize(size as usize, 0);
                }
                Ok(ok_status(id))
            }
            Some(OpenHandle::Upload(upload)) => {
                if let Some(size) = attrs.size {
                    if size > self.max_upload {
                        return Err(StatusCode::Failure);
                    }
                    upload
                        .file
                        .set_len(size)
                        .await
                        .map_err(|_| StatusCode::Failure)?;
                    upload.size = size;
                }
                Ok(ok_status(id))
            }
            Some(_) => Ok(ok_status(id)),
            None => Err(StatusCode::Failure),
        }
    }

    async fn opendir(&mut self, id: u32, path: String) -> SftpResult<Handle> {
        self.authorized().await?;
        let files = self.listing(&path).await?;
        let handle = self.allocate(OpenHandle::Dir(files))?;
        Ok(Handle { id, handle })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> SftpResult<Name> {
        match self.handles.get_mut(&handle) {
            Some(OpenHandle::Dir(files)) if !files.is_empty() => Ok(Name {
                id,
                files: std::mem::take(files),
            }),
            Some(OpenHandle::Dir(_)) => Err(StatusCode::Eof),
            _ => Err(StatusCode::Failure),
        }
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> SftpResult<Handle> {
        self.authorized().await?;
        let writing = pflags.contains(OpenFlags::WRITE);
        let (dir, leaf) = split_path(&filename);
        let leaf = leaf.ok_or(StatusCode::PermissionDenied)?;
        let parent = self.resolve_dir(&dir).await?;
        let Some(parent) = parent else {
            // A file cannot live loose at the root — only spaces do.
            return Err(if writing {
                StatusCode::PermissionDenied
            } else {
                StatusCode::NoSuchFile
            });
        };
        if self.dir_in(Some(parent), leaf).await?.is_some() {
            return Err(StatusCode::Failure);
        }

        let handle = match doc_slug(leaf) {
            Some(stem) => {
                let existing = self.doc_in(parent, leaf).await?;
                if writing {
                    if existing.is_none() && !pflags.contains(OpenFlags::CREATE) {
                        return Err(StatusCode::NoSuchFile);
                    }
                    if existing.is_some() && pflags.contains(OpenFlags::EXCLUDE) {
                        return Err(StatusCode::Failure);
                    }
                    let (id, buf, base) = match existing {
                        Some(doc) if pflags.contains(OpenFlags::TRUNCATE) => {
                            (doc.id, Vec::new(), doc.updated_at)
                        }
                        Some(doc) => (
                            doc.id,
                            render_doc(&doc.name, &doc.body).into_bytes(),
                            doc.updated_at,
                        ),
                        None => (
                            self.create_doc(parent, stem).await?,
                            Vec::new(),
                            crate::now(),
                        ),
                    };
                    OpenHandle::Write { id, buf, base }
                } else {
                    let doc = existing.ok_or(StatusCode::NoSuchFile)?;
                    OpenHandle::Read(render_doc(&doc.name, &doc.body).into_bytes())
                }
            }
            None if self.scratch.contains_key(&normalize(&filename)) || is_ephemeral(leaf) => {
                let key = normalize(&filename);
                let existing = self.scratch.get(&key).map(|file| file.data.clone());
                if writing {
                    if existing.is_none() && !pflags.contains(OpenFlags::CREATE) {
                        return Err(StatusCode::NoSuchFile);
                    }
                    if existing.is_some() && pflags.contains(OpenFlags::EXCLUDE) {
                        return Err(StatusCode::Failure);
                    }
                    let buf = match existing {
                        Some(data) if !pflags.contains(OpenFlags::TRUNCATE) => data,
                        _ => Vec::new(),
                    };
                    self.scratch.entry(key.clone()).or_insert_with(|| Scratch {
                        data: Vec::new(),
                        mtime: crate::now(),
                    });
                    OpenHandle::Scratch { path: key, buf }
                } else {
                    OpenHandle::Read(existing.ok_or(StatusCode::NoSuchFile)?)
                }
            }
            // Anything else is a file node, or about to be one.
            None => {
                let existing = self.file_in(parent, leaf).await?;
                if writing {
                    if existing.is_none() && !pflags.contains(OpenFlags::CREATE) {
                        return Err(StatusCode::NoSuchFile);
                    }
                    if existing.is_some() && pflags.contains(OpenFlags::EXCLUDE) {
                        return Err(StatusCode::Failure);
                    }
                    let truncate = pflags.contains(OpenFlags::TRUNCATE);
                    OpenHandle::Upload(
                        self.begin_upload(parent, leaf, existing.as_ref(), truncate)
                            .await?,
                    )
                } else {
                    let file = existing.ok_or(StatusCode::NoSuchFile)?;
                    let key = if file.encrypted {
                        self.vault_access().await?;
                        Some(self.vault_key()?)
                    } else {
                        None
                    };
                    let mut opened = tokio::fs::File::open(self.uploads.join(&file.media_id))
                        .await
                        .map_err(|_| StatusCode::NoSuchFile)?;
                    match key {
                        Some(key) => {
                            let len = opened
                                .metadata()
                                .await
                                .map_err(|_| StatusCode::Failure)?
                                .len();
                            let mut header = [0u8; vault::HEADER];
                            opened
                                .read_exact(&mut header)
                                .await
                                .map_err(|_| StatusCode::Failure)?;
                            let opener = key.opener(&file.media_id, &header, len).map_err(|e| {
                                tracing::warn!("sftp: could not open {leaf}: {e:?}");
                                StatusCode::Failure
                            })?;
                            OpenHandle::ReadSealed(Box::new(SealedRead {
                                file: opened,
                                opener,
                                size: file.size as u64,
                                record: None,
                            }))
                        }
                        None => OpenHandle::ReadFile {
                            file: opened,
                            size: file.size as u64,
                        },
                    }
                }
            }
        };

        let handle = self.allocate(handle)?;
        Ok(Handle { id, handle })
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> SftpResult<russh_sftp::protocol::Data> {
        // Locking the vault, or taking the key's access away, shuts what is
        // open as well as what opens next — a sealed file written into
        // included, whose handle reads back its plaintext.
        let sealed = match self.handles.get(&handle) {
            Some(OpenHandle::ReadSealed(_)) => true,
            Some(OpenHandle::Upload(upload)) => upload.sealed,
            _ => false,
        };
        if sealed {
            self.may_read_sealed().await?;
        }
        if let Some(OpenHandle::ReadSealed(sealed)) = self.handles.get_mut(&handle) {
            let data = sealed.read(offset, len).await?;
            return Ok(russh_sftp::protocol::Data { id, data });
        }

        // Files are read off the disk, a piece at a time, whether they are
        // being read or written.
        let disk = match self.handles.get_mut(&handle) {
            Some(OpenHandle::ReadFile { file, size }) => Some((file, *size)),
            Some(OpenHandle::Upload(upload)) => Some((&mut upload.file, upload.size)),
            _ => None,
        };
        if let Some((file, size)) = disk {
            if offset >= size {
                return Err(StatusCode::Eof);
            }
            let want = (len as u64).min(size - offset) as usize;
            let mut data = vec![0u8; want];
            file.seek(std::io::SeekFrom::Start(offset))
                .await
                .map_err(|_| StatusCode::Failure)?;
            file.read_exact(&mut data)
                .await
                .map_err(|_| StatusCode::Failure)?;
            return Ok(russh_sftp::protocol::Data { id, data });
        }

        // sshfs opens read-write and reads back through the same handle, so a
        // write handle reads its own buffer.
        let data = match self.handles.get(&handle) {
            Some(
                OpenHandle::Read(data)
                | OpenHandle::Write { buf: data, .. }
                | OpenHandle::Scratch { buf: data, .. },
            ) => data,
            _ => return Err(StatusCode::Failure),
        };
        let offset = offset as usize;
        if offset >= data.len() {
            return Err(StatusCode::Eof);
        }
        let end = (offset + len as usize).min(data.len());
        Ok(russh_sftp::protocol::Data {
            id,
            data: data[offset..end].to_vec(),
        })
    }

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> SftpResult<russh_sftp::protocol::Status> {
        self.authorized().await?;
        let held = self.held();
        match self.handles.get_mut(&handle) {
            Some(OpenHandle::Write { buf, .. } | OpenHandle::Scratch { buf, .. }) => {
                let grows = (offset as usize)
                    .saturating_add(data.len())
                    .saturating_sub(buf.len());
                if held.saturating_add(grows) > MAX_HELD {
                    return Err(StatusCode::Failure);
                }
                write_at(buf, offset, &data)?;
                Ok(ok_status(id))
            }
            Some(OpenHandle::Upload(upload)) => {
                let end = offset
                    .checked_add(data.len() as u64)
                    .filter(|&end| end <= self.max_upload)
                    .ok_or(StatusCode::Failure)?;
                upload
                    .file
                    .seek(std::io::SeekFrom::Start(offset))
                    .await
                    .map_err(|_| StatusCode::Failure)?;
                upload
                    .file
                    .write_all(&data)
                    .await
                    .map_err(|_| StatusCode::Failure)?;
                upload.size = upload.size.max(end);
                Ok(ok_status(id))
            }
            _ => Err(StatusCode::Failure),
        }
    }

    async fn close(&mut self, id: u32, handle: String) -> SftpResult<russh_sftp::protocol::Status> {
        let handle = self.handles.remove(&handle);
        // A removed key's writes are dropped rather than saved.
        self.authorized().await?;
        match handle {
            Some(OpenHandle::Write {
                id: node,
                buf,
                base,
            }) => self.finish_doc(node, buf, base).await?,
            Some(OpenHandle::Upload(upload)) => self.finish_upload(upload).await?,
            Some(OpenHandle::Scratch { path, buf }) => {
                // Removed while open: like an unlinked file, it is simply gone.
                if let Some(file) = self.scratch.get_mut(&path) {
                    file.data = buf;
                    file.mtime = crate::now();
                }
            }
            Some(
                OpenHandle::Read(_)
                | OpenHandle::ReadFile { .. }
                | OpenHandle::ReadSealed(_)
                | OpenHandle::Dir(_),
            ) => {}
            None => return Err(StatusCode::Failure),
        }
        Ok(ok_status(id))
    }

    async fn remove(
        &mut self,
        id: u32,
        filename: String,
    ) -> SftpResult<russh_sftp::protocol::Status> {
        self.authorized().await?;
        match self.target(&filename).await? {
            Target::Doc(doc) => self.delete_node(doc.id).await?,
            Target::File(file) => self.delete_node(file.id).await?,
            Target::Scratch(key) => {
                self.scratch.remove(&key);
            }
            Target::Root | Target::Dir { .. } => return Err(StatusCode::PermissionDenied),
        }
        Ok(ok_status(id))
    }

    /// A directory at the root is a new space; anywhere else it is a folder.
    async fn mkdir(
        &mut self,
        id: u32,
        path: String,
        _attrs: FileAttributes,
    ) -> SftpResult<russh_sftp::protocol::Status> {
        let (dir, leaf) = split_path(&path);
        let name = leaf.ok_or(StatusCode::PermissionDenied)?;
        self.authorized().await?;
        if self.target_opt(&path).await?.is_some() {
            return Err(StatusCode::Failure);
        }
        let Some(parent) = self.resolve_dir(&dir).await? else {
            nodes::insert_space(&self.db, name, true)
                .await
                .map_err(|_| StatusCode::Failure)?;
            self.changed();
            return Ok(ok_status(id));
        };
        let slug = nodes::unique_slug(
            &mut *self.conn().await?,
            Some(parent),
            &nodes::slugify(name),
            None,
        )
        .await
        .map_err(|_| StatusCode::Failure)?;
        sqlx::query(
            "INSERT INTO nodes (parent_id, kind, name, slug, created_at, updated_at)
             VALUES (?1, 'folder', ?2, ?3, ?4, ?4)",
        )
        .bind(parent)
        .bind(name)
        .bind(&slug)
        .bind(crate::now())
        .execute(&self.db)
        .await
        .map_err(|_| StatusCode::Failure)?;
        self.changed();
        Ok(ok_status(id))
    }

    async fn rmdir(&mut self, id: u32, path: String) -> SftpResult<russh_sftp::protocol::Status> {
        self.authorized().await?;
        let Target::Dir { id: node, .. } = self.target(&path).await? else {
            return Err(StatusCode::PermissionDenied);
        };
        let children: i64 = sqlx::query("SELECT count(*) AS n FROM nodes WHERE parent_id = ?1")
            .bind(node)
            .fetch_one(&self.db)
            .await
            .map_err(|_| StatusCode::Failure)?
            .get("n");
        if children > 0 {
            return Err(StatusCode::Failure);
        }
        let prefix = format!("{}/", normalize(&path));
        self.scratch.retain(|key, _| !key.starts_with(&prefix));
        self.delete_node(node).await?;
        Ok(ok_status(id))
    }

    /// Renaming onto an existing file replaces it, as `rename(2)` does — that
    /// is how editors save atomically. Directories are never replaced.
    async fn rename(
        &mut self,
        id: u32,
        oldpath: String,
        newpath: String,
    ) -> SftpResult<russh_sftp::protocol::Status> {
        self.authorized().await?;
        let source = self.target(&oldpath).await?;
        let (_, old_leaf) = split_path(&oldpath);
        let old_leaf = old_leaf.unwrap_or_default().to_string();
        let (new_dir, new_leaf) = split_path(&newpath);
        let new_leaf = new_leaf.ok_or(StatusCode::PermissionDenied)?;
        let new_parent = self.resolve_dir(&new_dir).await?;
        let dest = self.target_opt(&newpath).await?;
        let now = crate::now();

        match source {
            Target::Root => return Err(StatusCode::PermissionDenied),

            Target::Dir {
                id: node,
                kind,
                slug,
                ..
            } => {
                match dest {
                    Some(Target::Dir { id: other, .. }) if other == node => {}
                    Some(_) => return Err(StatusCode::Failure),
                    None => {}
                }
                // Spaces stay at the root and nothing else goes there, so a
                // rename never makes a nested space or a loose folder.
                match (kind.as_str(), new_parent) {
                    (nodes::SPACE, None) => {}
                    (nodes::SPACE, Some(_)) | (_, None) => {
                        return Err(StatusCode::PermissionDenied)
                    }
                    (_, Some(parent)) => {
                        let below = nodes::descendants(&mut *self.conn().await?, node)
                            .await
                            .map_err(|_| StatusCode::Failure)?;
                        if below.contains(&parent) {
                            return Err(StatusCode::Failure);
                        }
                        self.enter(node, parent).await?;
                    }
                }
                let new_slug = nodes::unique_slug(
                    &mut *self.conn().await?,
                    new_parent,
                    &nodes::slugify(new_leaf),
                    Some(node),
                )
                .await
                .map_err(|_| StatusCode::Failure)?;
                // A move that keeps the name keeps the title too.
                let name = (new_leaf != slug).then_some(new_leaf);
                sqlx::query(
                    "UPDATE nodes SET parent_id = ?1, name = COALESCE(?2, name), slug = ?3,
                     updated_at = ?4 WHERE id = ?5",
                )
                .bind(new_parent)
                .bind(name)
                .bind(&new_slug)
                .bind(now)
                .bind(node)
                .execute(&self.db)
                .await
                .map_err(|_| StatusCode::Failure)?;
                self.changed();
            }

            Target::Doc(doc) => {
                let Some(new_parent) = new_parent else {
                    return Err(StatusCode::PermissionDenied);
                };
                match (doc_slug(new_leaf), dest) {
                    (_, Some(Target::Dir { .. } | Target::File(_))) => {
                        return Err(StatusCode::Failure)
                    }
                    // Onto another document: that one takes this one's text
                    // and keeps its own id, so its share link survives.
                    (Some(_), Some(Target::Doc(other))) if other.id != doc.id => {
                        self.save_doc(other.id, render_doc(&doc.name, &doc.body).into_bytes())
                            .await?;
                        self.replace_node(doc.id).await?;
                    }
                    (Some(stem), _) => {
                        let new_slug = nodes::unique_slug(
                            &mut *self.conn().await?,
                            Some(new_parent),
                            &nodes::slugify(stem),
                            Some(doc.id),
                        )
                        .await
                        .map_err(|_| StatusCode::Failure)?;
                        sqlx::query(
                            "UPDATE nodes SET parent_id = ?1, slug = ?2, updated_at = ?3 WHERE id = ?4",
                        )
                        .bind(new_parent)
                        .bind(&new_slug)
                        .bind(now)
                        .bind(doc.id)
                        .execute(&self.db)
                        .await
                        .map_err(|_| StatusCode::Failure)?;
                        self.changed();
                    }
                    // Onto a name that is not a document: an editor moving the
                    // original aside as a backup before writing the new one.
                    // The backup is a copy; the document stays where it is, so
                    // the write that follows lands in the same node.
                    (None, _) => {
                        self.scratch.insert(
                            normalize(&newpath),
                            Scratch {
                                data: render_doc(&doc.name, &doc.body).into_bytes(),
                                mtime: now,
                            },
                        );
                    }
                }
            }

            Target::File(file) => {
                let Some(new_parent) = new_parent else {
                    return Err(StatusCode::PermissionDenied);
                };
                match dest {
                    Some(Target::Dir { .. } | Target::Doc(_)) => return Err(StatusCode::Failure),
                    Some(Target::File(other)) if other.id == file.id => {}
                    // Onto another file, as rename(2) does: that one goes.
                    Some(Target::File(other)) => self.replace_node(other.id).await?,
                    Some(Target::Scratch(key)) => {
                        self.scratch.remove(&key);
                    }
                    Some(Target::Root) | None => {}
                }
                self.move_file(&file, new_parent, new_leaf, &old_leaf)
                    .await?;
            }

            Target::Scratch(key) => {
                let Some(new_parent) = new_parent else {
                    return Err(StatusCode::PermissionDenied);
                };
                match (doc_slug(new_leaf), dest) {
                    (_, Some(Target::Dir { .. })) => return Err(StatusCode::Failure),
                    // A temp file renamed over a file, or onto a name worth
                    // keeping: what a sync tool's atomic write looks like.
                    (None, Some(Target::File(other))) => {
                        let data = self
                            .scratch
                            .remove(&key)
                            .map(|f| f.data)
                            .unwrap_or_default();
                        self.keep_scratch(new_parent, new_leaf, data, Some(&other))
                            .await?;
                    }
                    (None, None) if !is_ephemeral(new_leaf) => {
                        let data = self
                            .scratch
                            .remove(&key)
                            .map(|f| f.data)
                            .unwrap_or_default();
                        self.keep_scratch(new_parent, new_leaf, data, None).await?;
                    }
                    (Some(_), Some(Target::File(_))) => return Err(StatusCode::Failure),
                    // The atomic save: a temp file renamed over the document.
                    (Some(stem), dest) => {
                        let data = self.scratch[&key].data.clone();
                        let node = match dest {
                            Some(Target::Doc(other)) => other.id,
                            _ => self.create_doc(new_parent, stem).await?,
                        };
                        self.save_doc(node, data).await?;
                        self.scratch.remove(&key);
                    }
                    (None, _) => {
                        if let Some(file) = self.scratch.remove(&key) {
                            self.scratch.insert(normalize(&newpath), file);
                        }
                    }
                }
            }
        }

        Ok(ok_status(id))
    }
}

fn ok_status(id: u32) -> russh_sftp::protocol::Status {
    russh_sftp::protocol::Status {
        id,
        status_code: StatusCode::Ok,
        error_message: "Ok".to_string(),
        language_tag: "en-US".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use russh_sftp::{
        protocol::{FileAttributes, OpenFlags, StatusCode},
        server::Handler,
    };
    use sqlx::sqlite::SqlitePoolOptions;

    use super::{
        doc_slug, normalize, render_doc, segments, split_heading, write_at, SpaceFs, MAX_FILE,
    };

    #[test]
    fn writes_past_the_limit_fail_instead_of_wrapping() {
        let mut buf = Vec::new();
        assert!(write_at(&mut buf, u64::MAX - 1, b"abc").is_err());
        assert!(write_at(&mut buf, MAX_FILE as u64, b"a").is_err());
        assert!(buf.is_empty());
        write_at(&mut buf, 2, b"ab").unwrap();
        assert_eq!(buf, b"\0\0ab");
    }

    #[test]
    fn dot_dot_climbs_and_stops_at_the_root() {
        assert_eq!(
            segments("/proj/notes/../ideas/./a.md"),
            ["proj", "ideas", "a.md"]
        );
        assert_eq!(normalize("/../../proj"), "/proj");
        assert_eq!(normalize(""), "/");
    }

    #[test]
    fn only_plain_md_files_are_documents() {
        assert_eq!(doc_slug("notes.md"), Some("notes"));
        assert_eq!(doc_slug(".notes.md"), None);
        assert_eq!(doc_slug("notes.md~"), None);
        assert_eq!(doc_slug(".notes.md.swp"), None);
        assert_eq!(doc_slug("4913"), None);
        assert_eq!(doc_slug(".md"), None);
    }

    #[test]
    fn a_document_survives_the_round_trip_through_a_file() {
        let file = render_doc("Was ist EEBUS?", "text\n\nmore\n");
        assert_eq!(file, "# Was ist EEBUS?\n\ntext\n\nmore\n");
        assert_eq!(
            split_heading(&file),
            (Some("Was ist EEBUS?".into()), "text\n\nmore\n".into())
        );
        assert_eq!(render_doc("  ", "body"), "body");
        assert_eq!(split_heading("#nope\nbody"), (None, "#nope\nbody".into()));
    }

    async fn fs() -> SpaceFs {
        fs_in(std::env::temp_dir().join("narl-sftp-test-uploads")).await
    }

    async fn fs_in(uploads: std::path::PathBuf) -> SpaceFs {
        // One connection: every connection to `:memory:` is its own database.
        let db = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("an in-memory database");
        sqlx::migrate!("./migrations")
            .run(&db)
            .await
            .expect("migrations apply");
        SpaceFs::with_db(db, uploads, None)
    }

    /// A file put through the mount is a file node, read back from disk, and
    /// renamed, replaced and removed the way any file would be.
    #[tokio::test]
    async fn files_are_nodes_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let mut fs = fs_in(dir.path().to_path_buf()).await;
        fs.mkdir(0, "/proj".into(), FileAttributes::empty())
            .await
            .expect("mkdir");
        put(&mut fs, "/proj/slides.pdf", "%PDF one").await;
        put(&mut fs, "/proj/notes.md", "# Notes\n\ntext").await;
        assert_eq!(get(&mut fs, "/proj/slides.pdf").await, "%PDF one");

        async fn names(fs: &SpaceFs) -> Vec<String> {
            let mut names: Vec<String> = fs
                .listing("/proj")
                .await
                .unwrap()
                .into_iter()
                .map(|f| f.filename)
                .collect();
            names.sort();
            names
        }
        assert_eq!(names(&fs).await, ["notes.md", "slides.pdf"]);

        // Written again: new bytes under the same node.
        put(&mut fs, "/proj/slides.pdf", "%PDF two").await;
        assert_eq!(get(&mut fs, "/proj/slides.pdf").await, "%PDF two");
        let uploads = std::fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(uploads, 1, "the old bytes went with the old upload");

        fs.rename(0, "/proj/slides.pdf".into(), "/proj/Lecture 1.pdf".into())
            .await
            .expect("rename");
        assert_eq!(names(&fs).await, ["Lecture 1.pdf", "notes.md"]);

        // A sync tool's temp file, renamed onto the name it was for.
        put(&mut fs, "/proj/.handout.pdf.Xy12", "%PDF three").await;
        fs.rename(
            0,
            "/proj/.handout.pdf.Xy12".into(),
            "/proj/handout.pdf".into(),
        )
        .await
        .expect("rename");
        assert_eq!(get(&mut fs, "/proj/handout.pdf").await, "%PDF three");

        fs.remove(0, "/proj/Lecture 1.pdf".into())
            .await
            .expect("remove");
        fs.remove(0, "/proj/handout.pdf".into())
            .await
            .expect("remove");
        assert_eq!(names(&fs).await, ["notes.md"]);
        // In the trash, with their bytes, until it is emptied.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
        crate::routes::trash::purge_older(&fs.db, dir.path(), i64::MAX)
            .await
            .expect("empty the trash");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    /// A save from the browser while a document is open here is not undone
    /// by closing it: this write goes into a copy beside it.
    #[tokio::test]
    async fn a_write_over_a_newer_edit_is_kept_as_a_copy() {
        let mut fs = fs().await;
        fs.mkdir(0, "/proj".into(), FileAttributes::empty())
            .await
            .expect("mkdir");
        put(&mut fs, "/proj/plan.md", "# Plan\n\nfirst").await;

        let handle = fs
            .open(0, "/proj/plan.md".into(), CREATE, FileAttributes::empty())
            .await
            .unwrap()
            .handle;
        sqlx::query("UPDATE nodes SET body = 'from the browser', updated_at = updated_at + 60 WHERE slug = 'plan'")
            .execute(&fs.db)
            .await
            .unwrap();
        fs.write(0, handle.clone(), 0, b"# Plan\n\nfrom sftp".to_vec())
            .await
            .unwrap();
        fs.close(0, handle).await.unwrap();

        assert_eq!(
            get(&mut fs, "/proj/plan.md").await,
            "# Plan\n\nfrom the browser"
        );
        assert_eq!(
            get(&mut fs, "/proj/plan-conflict.md").await,
            "# Plan\n\nfrom sftp"
        );
    }

    #[tokio::test]
    async fn a_removed_key_ends_the_mount() {
        let mut fs = fs().await;
        fs.key_id = Some(42);
        assert_eq!(
            fs.opendir(0, "/".into()).await.err(),
            Some(StatusCode::PermissionDenied)
        );
    }

    /// A write into a sealed folder lands sealed while the vault is open, and
    /// is refused at the open while it is shut; a file moved in is sealed.
    #[tokio::test]
    async fn a_sealed_folder_takes_only_sealed_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("uploads")).unwrap();
        let state = crate::routes::tests::state_in(dir.path().into()).await;
        state.vault.open_with(zeroize::Zeroizing::new([9u8; 32]));
        let mut fs = SpaceFs::new(state.clone(), None);
        fs.max_upload = 1024;
        for path in ["/proj", "/proj/vault", "/proj/open"] {
            fs.mkdir(0, path.into(), FileAttributes::empty())
                .await
                .expect("mkdir");
        }
        sqlx::query("UPDATE nodes SET sealed = 1 WHERE slug = 'vault'")
            .execute(&state.db)
            .await
            .unwrap();
        let sealed_files = || async {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM nodes n JOIN media m ON m.id = n.media_id
                 WHERE m.encrypted = 1",
            )
            .fetch_one(&state.db)
            .await
            .unwrap()
        };

        put(&mut fs, "/proj/vault/key.pem", "-----BEGIN-----").await;
        assert_eq!(sealed_files().await, 1);
        let on_disk = std::fs::read_dir(dir.path().join("uploads"))
            .unwrap()
            .map(|e| std::fs::read(e.unwrap().path()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(on_disk.len(), 1);
        assert!(!on_disk[0].windows(5).any(|w| w == b"BEGIN"));

        put(&mut fs, "/proj/open/notes.txt", "plain").await;
        fs.rename(
            0,
            "/proj/open/notes.txt".into(),
            "/proj/vault/notes.txt".into(),
        )
        .await
        .expect("move in");
        assert_eq!(sealed_files().await, 2);

        state.vault.lock();
        let refused = fs
            .open(
                0,
                "/proj/vault/more.txt".into(),
                CREATE,
                FileAttributes::empty(),
            )
            .await;
        assert!(matches!(refused, Err(StatusCode::PermissionDenied)));
    }

    /// A sealed file reads as its plaintext while the vault is open, from any
    /// offset, is still sealed after it is written into or over — outside a
    /// sealed folder too — and stops reading the moment the vault locks.
    #[tokio::test]
    async fn a_sealed_file_opens_while_the_vault_is_open() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("uploads")).unwrap();
        let state = crate::routes::tests::state_in(dir.path().into()).await;
        state.vault.open_with(zeroize::Zeroizing::new([9u8; 32]));
        let mut fs = SpaceFs::new(state.clone(), None);
        fs.max_upload = 1024 * 1024;
        for path in ["/proj", "/proj/vault", "/proj/open"] {
            fs.mkdir(0, path.into(), FileAttributes::empty())
                .await
                .expect("mkdir");
        }
        sqlx::query("UPDATE nodes SET sealed = 1 WHERE slug = 'vault'")
            .execute(&state.db)
            .await
            .unwrap();
        let sealed_files = || async {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM nodes n JOIN media m ON m.id = n.media_id
                 WHERE m.encrypted = 1",
            )
            .fetch_one(&state.db)
            .await
            .unwrap()
        };

        // Three records and a bit, so reads cross from one into the next.
        let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let handle = fs
            .open(
                0,
                "/proj/vault/data.bin".into(),
                CREATE,
                FileAttributes::empty(),
            )
            .await
            .unwrap()
            .handle;
        fs.write(0, handle.clone(), 0, data.clone()).await.unwrap();
        fs.close(0, handle).await.unwrap();
        assert_eq!(sealed_files().await, 1);

        let handle = fs
            .open(
                0,
                "/proj/vault/data.bin".into(),
                OpenFlags::READ,
                FileAttributes::empty(),
            )
            .await
            .expect("opens while the vault is open")
            .handle;
        let mut whole = Vec::new();
        while let Ok(piece) = read_at(&mut fs, handle.clone(), whole.len() as u64, 32 * 1024).await
        {
            whole.extend(piece);
        }
        assert!(whole == data, "reads back as written");
        let across = read_at(&mut fs, handle.clone(), 65_000, 2_000)
            .await
            .unwrap();
        assert_eq!(across, data[65_000..67_000]);

        state.vault.lock();
        assert!(matches!(
            read_at(&mut fs, handle.clone(), 0, 10).await,
            Err(StatusCode::PermissionDenied)
        ));
        fs.close(0, handle).await.unwrap();
        assert!(matches!(
            fs.open(
                0,
                "/proj/vault/data.bin".into(),
                OpenFlags::READ,
                FileAttributes::empty()
            )
            .await,
            Err(StatusCode::PermissionDenied)
        ));
        state.vault.open_with(zeroize::Zeroizing::new([9u8; 32]));

        // Moved out, it stays sealed, and written into, it is sealed again.
        fs.rename(
            0,
            "/proj/vault/data.bin".into(),
            "/proj/open/data.bin".into(),
        )
        .await
        .expect("move out");
        let handle = fs
            .open(
                0,
                "/proj/open/data.bin".into(),
                OpenFlags::READ | OpenFlags::WRITE,
                FileAttributes::empty(),
            )
            .await
            .expect("opens for writing")
            .handle;
        fs.write(0, handle.clone(), 0, b"XYZ".to_vec())
            .await
            .unwrap();
        fs.close(0, handle).await.unwrap();
        assert_eq!(sealed_files().await, 1);
        let mut expected = data.clone();
        expected[..3].copy_from_slice(b"XYZ");
        let handle = fs
            .open(
                0,
                "/proj/open/data.bin".into(),
                OpenFlags::READ,
                FileAttributes::empty(),
            )
            .await
            .unwrap()
            .handle;
        let head = read_at(&mut fs, handle.clone(), 0, 8).await.unwrap();
        assert_eq!(head, expected[..8]);
        fs.close(0, handle).await.unwrap();
        let on_disk: Vec<Vec<u8>> = std::fs::read_dir(dir.path().join("uploads"))
            .unwrap()
            .map(|e| std::fs::read(e.unwrap().path()).unwrap())
            .collect();
        assert_eq!(on_disk.len(), 1);
        assert!(on_disk[0].starts_with(crate::vault::MAGIC));

        put(&mut fs, "/proj/open/data.bin", "over").await;
        assert_eq!(sealed_files().await, 1);
        assert_eq!(get(&mut fs, "/proj/open/data.bin").await, "over");
    }

    /// A key reads sealed files only once given vault access, and taking it
    /// away cuts off a read under way. Nothing it does holds the vault open.
    #[tokio::test]
    async fn only_a_key_with_vault_access_reads_sealed_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("uploads")).unwrap();
        let state = crate::routes::tests::state_in(dir.path().into()).await;
        state.vault.open_with(zeroize::Zeroizing::new([9u8; 32]));
        let key: i64 = sqlx::query_scalar(
            "INSERT INTO ssh_keys (name, public_key, fingerprint, created_at)
             VALUES ('laptop', 'ssh-ed25519 AAAA', 'SHA256:x', 1) RETURNING id",
        )
        .fetch_one(&state.db)
        .await
        .unwrap();
        let set_access = |on: bool| {
            sqlx::query("UPDATE ssh_keys SET vault = ?1 WHERE id = ?2")
                .bind(on)
                .bind(key)
                .execute(&state.db)
        };
        let mut fs = SpaceFs::new(state.clone(), Some(key));
        fs.max_upload = 1024;
        for path in ["/proj", "/proj/vault"] {
            fs.mkdir(0, path.into(), FileAttributes::empty())
                .await
                .expect("mkdir");
        }
        sqlx::query("UPDATE nodes SET sealed = 1 WHERE slug = 'vault'")
            .execute(&state.db)
            .await
            .unwrap();
        const NOTE: &str = "/proj/vault/note.txt";

        // Writing in seals, and needs no access: nothing comes back out.
        put(&mut fs, NOTE, "secret").await;

        for flags in [OpenFlags::READ, OpenFlags::READ | OpenFlags::WRITE] {
            assert!(
                matches!(
                    open_at(&mut fs, NOTE, flags).await,
                    Err(StatusCode::PermissionDenied)
                ),
                "{flags:?} without access"
            );
        }
        // No `.part` of the plaintext stays behind a refused or dropped open.
        let parts = || {
            std::fs::read_dir(dir.path().join("uploads"))
                .unwrap()
                .filter(|e| {
                    e.as_ref()
                        .unwrap()
                        .path()
                        .extension()
                        .is_some_and(|x| x == "part")
                })
                .count()
        };
        assert_eq!(parts(), 0);

        set_access(true).await.unwrap();
        assert_eq!(get(&mut fs, NOTE).await, "secret");
        let handle = open_at(&mut fs, NOTE, OpenFlags::READ).await.unwrap();
        set_access(false).await.unwrap();
        assert!(matches!(
            read_at(&mut fs, handle.clone(), 0, 6).await,
            Err(StatusCode::PermissionDenied)
        ));
        fs.close(0, handle).await.unwrap();

        // Opened for writing over it, the decrypted copy goes when the
        // session does.
        set_access(true).await.unwrap();
        let handle = open_at(&mut fs, NOTE, OpenFlags::READ | OpenFlags::WRITE)
            .await
            .unwrap();
        assert_eq!(read_at(&mut fs, handle, 0, 6).await.unwrap(), b"secret");
        assert_eq!(parts(), 1);
        drop(fs);
        assert_eq!(parts(), 0);
    }

    /// One session cannot open handles without end.
    #[tokio::test]
    async fn handles_run_out_before_memory_does() {
        let dir = tempfile::tempdir().unwrap();
        let mut fs = fs_in(dir.path().to_path_buf()).await;
        fs.mkdir(0, "/proj".into(), FileAttributes::empty())
            .await
            .unwrap();
        for _ in 0..super::MAX_HANDLES {
            fs.opendir(0, "/proj".into()).await.unwrap();
        }
        assert!(fs.opendir(0, "/proj".into()).await.is_err());
    }

    const CREATE: OpenFlags = OpenFlags::WRITE
        .union(OpenFlags::CREATE)
        .union(OpenFlags::TRUNCATE);

    async fn put(fs: &mut SpaceFs, path: &str, text: &str) {
        let handle = fs
            .open(0, path.into(), CREATE, FileAttributes::empty())
            .await
            .expect("open for writing")
            .handle;
        fs.write(0, handle.clone(), 0, text.as_bytes().to_vec())
            .await
            .expect("write");
        fs.close(0, handle).await.expect("close");
    }

    async fn open_at(fs: &mut SpaceFs, path: &str, flags: OpenFlags) -> Result<String, StatusCode> {
        fs.open(0, path.into(), flags, FileAttributes::empty())
            .await
            .map(|h| h.handle)
    }

    async fn read_at(
        fs: &mut SpaceFs,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Vec<u8>, StatusCode> {
        fs.read(0, handle, offset, len).await.map(|d| d.data)
    }

    async fn get(fs: &mut SpaceFs, path: &str) -> String {
        let handle = fs
            .open(0, path.into(), OpenFlags::READ, FileAttributes::empty())
            .await
            .expect("open for reading")
            .handle;
        let data = fs
            .read(0, handle.clone(), 0, u32::MAX)
            .await
            .map(|d| d.data)
            .unwrap_or_default();
        fs.close(0, handle).await.expect("close");
        String::from_utf8(data).expect("utf-8")
    }

    async fn node(fs: &SpaceFs, slug: &str) -> (i64, String, String) {
        let row: (i64, String, String) =
            sqlx::query_as("SELECT id, name, body FROM nodes WHERE slug = ?1")
                .bind(slug)
                .fetch_one(&fs.db)
                .await
                .expect("the node exists");
        row
    }

    #[tokio::test]
    async fn mkdir_at_the_root_makes_a_space_and_a_document_lands_in_it() {
        let mut fs = fs().await;
        fs.mkdir(0, "/Side Project".into(), FileAttributes::empty())
            .await
            .expect("mkdir");
        let kind: String = sqlx::query_scalar("SELECT kind FROM nodes WHERE slug = 'side-project'")
            .fetch_one(&fs.db)
            .await
            .expect("the space exists");
        assert_eq!(kind, "space");

        // Both the name it was made with and the slug it got resolve.
        fs.stat(0, "/Side Project".into())
            .await
            .expect("stat by name");
        fs.mkdir(0, "/side-project/Ideas".into(), FileAttributes::empty())
            .await
            .expect("mkdir");
        assert_eq!(
            fs.mkdir(0, "/side-project/ideas".into(), FileAttributes::empty())
                .await
                .err(),
            Some(StatusCode::Failure)
        );

        put(
            &mut fs,
            "/side-project/ideas/My Notes.md",
            "# First thoughts\n\nhello\n",
        )
        .await;
        let (_, name, body) = node(&fs, "my-notes").await;
        assert_eq!(name, "First thoughts");
        assert_eq!(body, "hello\n");
        fs.stat(0, "/side-project/ideas/My Notes.md".into())
            .await
            .expect("stat by name");

        let text = get(&mut fs, "/side-project/ideas/my-notes.md").await;
        assert_eq!(text, "# First thoughts\n\nhello\n");
        let size = fs
            .stat(0, "/side-project/ideas/my-notes.md".into())
            .await
            .expect("stat")
            .attrs
            .size;
        assert_eq!(size, Some(text.len() as u64));
    }

    #[tokio::test]
    async fn a_vim_save_keeps_the_same_document() {
        let mut fs = fs().await;
        fs.mkdir(0, "/proj".into(), FileAttributes::empty())
            .await
            .expect("mkdir");
        put(&mut fs, "/proj/plan.md", "# Plan\n\nv1\n").await;
        let (id, ..) = node(&fs, "plan").await;

        // The sequence vim runs with `backupcopy=auto` on a mount.
        put(&mut fs, "/proj/4913", "").await;
        fs.remove(0, "/proj/4913".into())
            .await
            .expect("remove probe");
        put(&mut fs, "/proj/.plan.md.swp", "swap").await;
        fs.rename(0, "/proj/plan.md".into(), "/proj/plan.md~".into())
            .await
            .expect("backup");
        put(&mut fs, "/proj/plan.md", "# Plan\n\nv2\n").await;
        fs.remove(0, "/proj/plan.md~".into())
            .await
            .expect("drop backup");
        fs.remove(0, "/proj/.plan.md.swp".into())
            .await
            .expect("drop swap");

        let (after, name, body) = node(&fs, "plan").await;
        assert_eq!((after, name.as_str(), body.as_str()), (id, "Plan", "v2\n"));
        let docs: i64 = sqlx::query_scalar("SELECT count(*) FROM nodes WHERE kind = 'document'")
            .fetch_one(&fs.db)
            .await
            .expect("count");
        assert_eq!(docs, 1);
    }

    #[tokio::test]
    async fn a_temp_file_renamed_over_a_document_replaces_its_text() {
        let mut fs = fs().await;
        fs.mkdir(0, "/proj".into(), FileAttributes::empty())
            .await
            .expect("mkdir");
        put(&mut fs, "/proj/plan.md", "# Plan\n\nv1\n").await;
        let (id, ..) = node(&fs, "plan").await;

        put(&mut fs, "/proj/plan.md.tmp", "# Renamed\n\nv2\n").await;
        let listed = fs.listing("/proj").await.expect("listing");
        assert!(listed.iter().any(|f| f.filename == "plan.md.tmp"));

        fs.rename(0, "/proj/plan.md.tmp".into(), "/proj/plan.md".into())
            .await
            .expect("rename");
        let (after, name, body) = node(&fs, "plan").await;
        assert_eq!(
            (after, name.as_str(), body.as_str()),
            (id, "Renamed", "v2\n")
        );
        assert!(fs.scratch.is_empty());

        // The save over the mount kept what it replaced.
        let kept: Vec<(String, String)> =
            sqlx::query_as("SELECT body, reason FROM revisions WHERE node_id = ?1 ORDER BY id")
                .bind(id)
                .fetch_all(&fs.db)
                .await
                .expect("revisions");
        assert_eq!(kept, [("v1\n".to_string(), "sftp".to_string())]);
    }

    #[tokio::test]
    async fn spaces_rename_at_the_root_and_folders_cannot_swallow_themselves() {
        let mut fs = fs().await;
        fs.mkdir(0, "/proj".into(), FileAttributes::empty())
            .await
            .expect("mkdir");
        fs.mkdir(0, "/proj/a".into(), FileAttributes::empty())
            .await
            .expect("mkdir");
        fs.mkdir(0, "/proj/a/b".into(), FileAttributes::empty())
            .await
            .expect("mkdir");

        fs.rename(0, "/proj".into(), "/Gig".into())
            .await
            .expect("rename space");
        let (_, name, _) = node(&fs, "gig").await;
        assert_eq!(name, "Gig");

        assert_eq!(
            fs.rename(0, "/gig/a".into(), "/gig/a/b/a".into())
                .await
                .err(),
            Some(StatusCode::Failure)
        );
        assert_eq!(
            fs.rename(0, "/gig".into(), "/gig/a/gig".into()).await.err(),
            Some(StatusCode::PermissionDenied)
        );
        assert_eq!(
            fs.rename(0, "/gig/a".into(), "/a".into()).await.err(),
            Some(StatusCode::PermissionDenied)
        );

        fs.mkdir(0, "/empty".into(), FileAttributes::empty())
            .await
            .expect("mkdir");
        fs.rmdir(0, "/empty".into())
            .await
            .expect("an empty space can go");
        assert_eq!(
            fs.rmdir(0, "/gig".into()).await.err(),
            Some(StatusCode::Failure)
        );
    }
}
