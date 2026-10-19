//! The virtual filesystem an SFTP client sees: `/<space>/<folder>/.../<doc>.md`.
//! Directories are spaces and folders; the files that persist are documents,
//! always `.md`. File nodes are not shown here yet: a folder holding only those
//! lists as empty, and refuses `rmdir` because it is not. There is no on-disk mirror — every call reads or writes
//! `nodes` directly, so a save here is a save in the browser too.
//!
//! A document reads as `# <title>` followed by its body, the same shape the
//! export writes, and a leading heading written back becomes the title again.
//!
//! Anything else an editor writes next to a document — vim's `.swp` and `4913`
//! probe, a `foo.md~` backup, a `.tmp` it renames over the original — lives in
//! memory for the length of the session. Renaming one of those onto a `.md`
//! name is what saves it into the workspace.

use std::{collections::HashMap, sync::Arc};

use russh_sftp::protocol::{
    File, FileAttributes, Handle, Name, OpenFlags, StatusCode, Version,
};
use sqlx::{Row, SqlitePool};

use crate::{backup::Backup, routes::media, routes::nodes, state::AppState};

type SftpResult<T> = Result<T, StatusCode>;

/// The largest file this filesystem holds, document or scratch. Everything is
/// buffered in memory until it is closed, so there has to be a ceiling.
const MAX_FILE: usize = 16 * 1024 * 1024;

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
    /// A session-only file, keyed by its normalised path.
    Scratch(String),
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
    /// and the buffer is saved into it when the handle closes.
    Write { id: i64, buf: Vec<u8> },
    /// A scratch file being written, stored back under `path` on close.
    Scratch { path: String, buf: Vec<u8> },
}

pub struct SpaceFs {
    db: SqlitePool,
    /// `None` only in tests, where there is nothing to mirror to.
    backup: Option<Arc<Backup>>,
    handles: HashMap<String, OpenHandle>,
    next_handle: u64,
    scratch: HashMap<String, Scratch>,
}

impl SpaceFs {
    pub fn new(state: AppState) -> Self {
        Self::with_db(state.db, Some(state.backup))
    }

    fn with_db(db: SqlitePool, backup: Option<Arc<Backup>>) -> Self {
        Self {
            db,
            backup,
            handles: HashMap::new(),
            next_handle: 0,
            scratch: HashMap::new(),
        }
    }

    fn changed(&self) {
        if let Some(backup) = &self.backup {
            backup.signal();
        }
    }

    fn allocate(&mut self, handle: OpenHandle) -> String {
        self.next_handle += 1;
        let key = self.next_handle.to_string();
        self.handles.insert(key.clone(), handle);
        key
    }

    /// A space or folder directly under `parent`, by slug — or by the slug its
    /// name would get, so `mkdir "My Folder"` can be stat-ed by that name.
    async fn dir_in(&self, parent: Option<i64>, leaf: &str) -> SftpResult<Option<Target>> {
        let slugged = nodes::slugify(leaf);
        for slug in [leaf, slugged.as_str()] {
            let row = sqlx::query(
                "SELECT id, kind, slug, updated_at FROM nodes
                 WHERE parent_id IS ?1 AND slug = ?2 AND kind IN ('space', 'folder')",
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
            Target::Scratch(key) => {
                let file = &self.scratch[key];
                file_attrs(file.data.len() as u64, file.mtime)
            }
        }
    }

    /// An empty document under `parent`, named after the file it was created
    /// as until a heading says otherwise.
    async fn create_doc(&self, parent: i64, stem: &str) -> SftpResult<i64> {
        let slug = nodes::unique_slug(&self.db, Some(parent), &nodes::slugify(stem), None)
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
        self.changed();
        Ok(id)
    }

    /// Save a file's bytes into a document. A leading heading renames it;
    /// without one the document keeps the title it had.
    async fn save_doc(&self, id: i64, bytes: Vec<u8>) -> SftpResult<()> {
        let text = String::from_utf8(bytes).map_err(|_| StatusCode::Failure)?;
        let (title, body) = split_heading(&text);
        sqlx::query(
            "UPDATE nodes SET name = COALESCE(?1, name), body = ?2, updated_at = ?3 WHERE id = ?4",
        )
        .bind(title)
        .bind(&body)
        .bind(crate::now())
        .bind(id)
        .execute(&self.db)
        .await
        .map_err(|_| StatusCode::Failure)?;
        media::link_to_node(&self.db, id, &body)
            .await
            .map_err(|_| StatusCode::Failure)?;
        self.changed();
        Ok(())
    }

    async fn delete_node(&self, id: i64) -> SftpResult<()> {
        sqlx::query("DELETE FROM nodes WHERE id = ?1")
            .bind(id)
            .execute(&self.db)
            .await
            .map_err(|_| StatusCode::Failure)?;
        self.changed();
        Ok(())
    }

    async fn listing(&self, path: &str) -> SftpResult<Vec<File>> {
        let segs = segments(path);
        let parent = self.resolve_dir(&segs).await?;
        let rows = sqlx::query(&format!(
            "SELECT kind, slug, updated_at, {DOC_SIZE} AS size FROM nodes
             WHERE parent_id IS ?1 AND kind <> 'file' ORDER BY kind, slug"
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

        let dir = normalize(path);
        for (key, file) in &self.scratch {
            if let Some((parent, name)) = key.rsplit_once('/') {
                if parent == dir || (parent.is_empty() && dir == "/") {
                    files.push(File::new(name, file_attrs(file.data.len() as u64, file.mtime)));
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
                    if let Some(file) = self.scratch.get_mut(&key) {
                        file.data.resize(size, 0);
                        file.mtime = crate::now();
                    }
                }
                Target::Root | Target::Dir { .. } => {}
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
        match self.handles.get_mut(&handle) {
            Some(OpenHandle::Write { buf, .. } | OpenHandle::Scratch { buf, .. }) => {
                if let Some(size) = attrs.size {
                    if size as usize > MAX_FILE {
                        return Err(StatusCode::Failure);
                    }
                    buf.resize(size as usize, 0);
                }
                Ok(ok_status(id))
            }
            Some(_) => Ok(ok_status(id)),
            None => Err(StatusCode::Failure),
        }
    }

    async fn opendir(&mut self, id: u32, path: String) -> SftpResult<Handle> {
        let files = self.listing(&path).await?;
        let handle = self.allocate(OpenHandle::Dir(files));
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
                    let (id, buf) = match existing {
                        Some(doc) if pflags.contains(OpenFlags::TRUNCATE) => (doc.id, Vec::new()),
                        Some(doc) => (doc.id, render_doc(&doc.name, &doc.body).into_bytes()),
                        None => (self.create_doc(parent, stem).await?, Vec::new()),
                    };
                    OpenHandle::Write { id, buf }
                } else {
                    let doc = existing.ok_or(StatusCode::NoSuchFile)?;
                    OpenHandle::Read(render_doc(&doc.name, &doc.body).into_bytes())
                }
            }
            None => {
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
        };

        let handle = self.allocate(handle);
        Ok(Handle { id, handle })
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> SftpResult<russh_sftp::protocol::Data> {
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
        match self.handles.get_mut(&handle) {
            Some(OpenHandle::Write { buf, .. } | OpenHandle::Scratch { buf, .. }) => {
                write_at(buf, offset, &data)?;
                Ok(ok_status(id))
            }
            _ => Err(StatusCode::Failure),
        }
    }

    async fn close(&mut self, id: u32, handle: String) -> SftpResult<russh_sftp::protocol::Status> {
        match self.handles.remove(&handle) {
            Some(OpenHandle::Write { id: node, buf }) => self.save_doc(node, buf).await?,
            Some(OpenHandle::Scratch { path, buf }) => {
                // Removed while open: like an unlinked file, it is simply gone.
                if let Some(file) = self.scratch.get_mut(&path) {
                    file.data = buf;
                    file.mtime = crate::now();
                }
            }
            Some(OpenHandle::Read(_) | OpenHandle::Dir(_)) => {}
            None => return Err(StatusCode::Failure),
        }
        Ok(ok_status(id))
    }

    async fn remove(&mut self, id: u32, filename: String) -> SftpResult<russh_sftp::protocol::Status> {
        match self.target(&filename).await? {
            Target::Doc(doc) => self.delete_node(doc.id).await?,
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
        let slug = nodes::unique_slug(&self.db, Some(parent), &nodes::slugify(name), None)
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
        let source = self.target(&oldpath).await?;
        let (new_dir, new_leaf) = split_path(&newpath);
        let new_leaf = new_leaf.ok_or(StatusCode::PermissionDenied)?;
        let new_parent = self.resolve_dir(&new_dir).await?;
        let dest = self.target_opt(&newpath).await?;
        let now = crate::now();

        match source {
            Target::Root => return Err(StatusCode::PermissionDenied),

            Target::Dir { id: node, kind, slug, .. } => {
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
                        let below = nodes::descendants(&self.db, node)
                            .await
                            .map_err(|_| StatusCode::Failure)?;
                        if below.contains(&parent) {
                            return Err(StatusCode::Failure);
                        }
                    }
                }
                let new_slug =
                    nodes::unique_slug(&self.db, new_parent, &nodes::slugify(new_leaf), Some(node))
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
                    (_, Some(Target::Dir { .. })) => return Err(StatusCode::Failure),
                    // Onto another document: that one takes this one's text
                    // and keeps its own id, so its share link survives.
                    (Some(_), Some(Target::Doc(other))) if other.id != doc.id => {
                        self.save_doc(other.id, render_doc(&doc.name, &doc.body).into_bytes())
                            .await?;
                        self.delete_node(doc.id).await?;
                    }
                    (Some(stem), _) => {
                        let new_slug = nodes::unique_slug(
                            &self.db,
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

            Target::Scratch(key) => {
                let Some(new_parent) = new_parent else {
                    return Err(StatusCode::PermissionDenied);
                };
                match (doc_slug(new_leaf), dest) {
                    (_, Some(Target::Dir { .. })) => return Err(StatusCode::Failure),
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

    use super::{doc_slug, normalize, render_doc, segments, split_heading, write_at, SpaceFs, MAX_FILE};

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
        assert_eq!(segments("/proj/notes/../ideas/./a.md"), ["proj", "ideas", "a.md"]);
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
        // One connection: every connection to `:memory:` is its own database.
        let db = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("an in-memory database");
        sqlx::migrate!("./migrations").run(&db).await.expect("migrations apply");
        SpaceFs::with_db(db, None)
    }

    const CREATE: OpenFlags = OpenFlags::WRITE.union(OpenFlags::CREATE).union(OpenFlags::TRUNCATE);

    async fn put(fs: &mut SpaceFs, path: &str, text: &str) {
        let handle = fs
            .open(0, path.into(), CREATE, FileAttributes::empty())
            .await
            .expect("open for writing")
            .handle;
        fs.write(0, handle.clone(), 0, text.as_bytes().to_vec()).await.expect("write");
        fs.close(0, handle).await.expect("close");
    }

    async fn get(fs: &mut SpaceFs, path: &str) -> String {
        let handle = fs
            .open(0, path.into(), OpenFlags::READ, FileAttributes::empty())
            .await
            .expect("open for reading")
            .handle;
        let data = fs.read(0, handle.clone(), 0, u32::MAX).await.map(|d| d.data).unwrap_or_default();
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
        fs.mkdir(0, "/Side Project".into(), FileAttributes::empty()).await.expect("mkdir");
        let kind: String = sqlx::query_scalar("SELECT kind FROM nodes WHERE slug = 'side-project'")
            .fetch_one(&fs.db)
            .await
            .expect("the space exists");
        assert_eq!(kind, "space");

        // Both the name it was made with and the slug it got resolve.
        fs.stat(0, "/Side Project".into()).await.expect("stat by name");
        fs.mkdir(0, "/side-project/Ideas".into(), FileAttributes::empty()).await.expect("mkdir");
        assert_eq!(
            fs.mkdir(0, "/side-project/ideas".into(), FileAttributes::empty()).await.err(),
            Some(StatusCode::Failure)
        );

        put(&mut fs, "/side-project/ideas/My Notes.md", "# First thoughts\n\nhello\n").await;
        let (_, name, body) = node(&fs, "my-notes").await;
        assert_eq!(name, "First thoughts");
        assert_eq!(body, "hello\n");
        fs.stat(0, "/side-project/ideas/My Notes.md".into()).await.expect("stat by name");

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
        fs.mkdir(0, "/proj".into(), FileAttributes::empty()).await.expect("mkdir");
        put(&mut fs, "/proj/plan.md", "# Plan\n\nv1\n").await;
        let (id, ..) = node(&fs, "plan").await;

        // The sequence vim runs with `backupcopy=auto` on a mount.
        put(&mut fs, "/proj/4913", "").await;
        fs.remove(0, "/proj/4913".into()).await.expect("remove probe");
        put(&mut fs, "/proj/.plan.md.swp", "swap").await;
        fs.rename(0, "/proj/plan.md".into(), "/proj/plan.md~".into()).await.expect("backup");
        put(&mut fs, "/proj/plan.md", "# Plan\n\nv2\n").await;
        fs.remove(0, "/proj/plan.md~".into()).await.expect("drop backup");
        fs.remove(0, "/proj/.plan.md.swp".into()).await.expect("drop swap");

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
        fs.mkdir(0, "/proj".into(), FileAttributes::empty()).await.expect("mkdir");
        put(&mut fs, "/proj/plan.md", "# Plan\n\nv1\n").await;
        let (id, ..) = node(&fs, "plan").await;

        put(&mut fs, "/proj/plan.md.tmp", "# Renamed\n\nv2\n").await;
        let listed = fs.listing("/proj").await.expect("listing");
        assert!(listed.iter().any(|f| f.filename == "plan.md.tmp"));

        fs.rename(0, "/proj/plan.md.tmp".into(), "/proj/plan.md".into()).await.expect("rename");
        let (after, name, body) = node(&fs, "plan").await;
        assert_eq!((after, name.as_str(), body.as_str()), (id, "Renamed", "v2\n"));
        assert!(fs.scratch.is_empty());
    }

    #[tokio::test]
    async fn spaces_rename_at_the_root_and_folders_cannot_swallow_themselves() {
        let mut fs = fs().await;
        fs.mkdir(0, "/proj".into(), FileAttributes::empty()).await.expect("mkdir");
        fs.mkdir(0, "/proj/a".into(), FileAttributes::empty()).await.expect("mkdir");
        fs.mkdir(0, "/proj/a/b".into(), FileAttributes::empty()).await.expect("mkdir");

        fs.rename(0, "/proj".into(), "/Gig".into()).await.expect("rename space");
        let (_, name, _) = node(&fs, "gig").await;
        assert_eq!(name, "Gig");

        assert_eq!(
            fs.rename(0, "/gig/a".into(), "/gig/a/b/a".into()).await.err(),
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

        fs.mkdir(0, "/empty".into(), FileAttributes::empty()).await.expect("mkdir");
        fs.rmdir(0, "/empty".into()).await.expect("an empty space can go");
        assert_eq!(fs.rmdir(0, "/gig".into()).await.err(), Some(StatusCode::Failure));
    }
}
