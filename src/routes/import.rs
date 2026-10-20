//! Importing a folder as a tree of documents.
//!
//! A directory of markdown notes is the format writing arrives in — a repo's
//! `docs/`, a folder of lecture notes, an export of this very diary. Storing it
//! as opaque files would make it unsearchable and uneditable, so the importer
//! reproduces the directory as nodes: directories become folders, markdown
//! becomes documents, everything else becomes a file. The links between the
//! files are rewritten as they land, because a set of documents that can no
//! longer reach each other is a worse copy of what was uploaded.

use std::collections::{BTreeSet, HashMap};

use axum::{
    extract::{Multipart, Path, State},
    Json,
};
use regex::Regex;
use serde::Serialize;
use std::sync::LazyLock;
use uuid::Uuid;

use crate::{
    auth::Session,
    error::{AppError, AppResult},
    routes::{
        boards, links,
        manifest::{self, Manifest, NodeMeta},
        media,
        nodes::{self, next_slug, slugify, DOCUMENT, FILE, FOLDER, SPACE},
    },
    state::AppState,
    vault,
};

/// Markdown inline links and images: `[text](target)`, `![alt](target "title")`,
/// `[text](<target with spaces>)`. Reference-style definitions are left alone —
/// they are rare in the wild and rewriting them wrongly is worse than not.
static LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(!?)\[([^\]\n]*)\]\(\s*<?([^)\s<>]*)>?([^)\n]*)\)").expect("static regex is valid")
});

/// What the client is told happened. Counts rather than a list of ids: the
/// status line has one line, and the tree is right there to look at.
#[derive(Serialize)]
pub struct ImportReport {
    /// New spaces, made by an import at the root.
    pub spaces: usize,
    pub folders: usize,
    pub documents: usize,
    pub files: usize,
    pub links: usize,
    pub cards: usize,
    /// Paths that were refused, so a silently missing file is never a surprise.
    pub skipped: Vec<String>,
}

/// One file of the upload, with the path it had inside the folder.
struct Upload {
    path: String,
    content: Content,
}

enum Content {
    /// A markdown file, read into memory: it is about to become a document body.
    Markdown(String),
    /// The `.workspace.json` of an export, read into memory to be parsed.
    Manifest(String),
    /// Anything else, already streamed to `uploads/<id>`.
    File { id: String, mime: String, size: u64 },
}

/// Where a path inside the import ended up, as a URL a document body can use.
#[derive(Clone)]
enum Landed {
    Node(String),
    Media(String),
}

/// A path that is safe to build a tree from: relative, no `.` or `..` segment,
/// no empty segment, no backslash separators smuggled in from Windows.
fn normalize_path(raw: &str) -> Option<String> {
    let raw = raw.trim().replace('\\', "/");
    if raw.is_empty() || raw.starts_with('/') || raw.contains('\0') {
        return None;
    }
    // A drive letter is an absolute path wearing a disguise.
    if raw.len() > 1 && raw.as_bytes()[1] == b':' {
        return None;
    }

    let mut parts = Vec::new();
    for part in raw.split('/') {
        match part {
            "" | "." => continue,
            ".." => return None,
            _ => parts.push(part),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// The directory a path sits in, `""` for the top level.
fn parent_of(path: &str) -> &str {
    path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("")
}

fn file_name(path: &str) -> &str {
    path.rsplit_once('/').map(|(_, name)| name).unwrap_or(path)
}

/// Read into memory rather than streamed to disk: markdown, and the manifest.
fn is_text(path: &str) -> bool {
    is_markdown(path) || file_name(path) == manifest::FILE_NAME
}

fn text_content(path: &str, text: String) -> Content {
    if file_name(path) == manifest::FILE_NAME {
        Content::Manifest(text)
    } else {
        Content::Markdown(text)
    }
}

fn is_markdown(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".markdown")
}

/// The title of a document and the body left over. A markdown file usually
/// opens with the heading that names it; a node keeps its name in its own
/// column, so carrying the heading in the body too would show it twice and
/// export it twice. Without a heading the file name has to do.
fn split_title(body: &str, path: &str) -> (String, String) {
    let mut lines = body.lines();
    let heading = lines
        .next()
        .and_then(|line| line.trim().strip_prefix("# "))
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_string);

    match heading {
        Some(title) => (
            title,
            lines
                .collect::<Vec<_>>()
                .join("\n")
                .trim_start()
                .to_string(),
        ),
        None => {
            let name = file_name(path);
            let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
            (stem.to_string(), body.to_string())
        }
    }
}

/// `%20` and friends, because a link to a file with a space in its name is
/// written escaped but the file arrives named with the space.
fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        // Hex digits are read as bytes, never as a slice of the string: the
        // two after a `%` may well be the first half of a multi-byte character.
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(hi << 4 | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| value.to_string())
}

fn hex(digit: u8) -> Option<u8> {
    (digit as char).to_digit(16).map(|d| d as u8)
}

/// Split a link target into the part that names a file and the `#fragment` or
/// `?query` that follows it. `None` for anything that does not point inside the
/// import: a URL, an absolute path, a bare anchor, a mail address.
fn split_target(raw: &str) -> Option<(String, &str)> {
    let raw = raw.trim();
    if raw.is_empty() || raw.starts_with('#') || raw.starts_with('/') || raw.contains("://") {
        return None;
    }
    if raw.starts_with("mailto:") || raw.starts_with("tel:") || raw.starts_with("data:") {
        return None;
    }

    let cut = raw.find(['#', '?']).unwrap_or(raw.len());
    let (path, suffix) = raw.split_at(cut);
    (!path.is_empty()).then(|| (percent_decode(path), suffix))
}

/// Resolve a relative link against the directory its document sits in. `None`
/// when it climbs out of the import — that file was not uploaded, so there is
/// nothing to point at and the link is better left as it was written.
fn join_relative(dir: &str, target: &str) -> Option<String> {
    let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
    for part in target.split('/') {
        match part {
            "" | "." => continue,
            ".." => {
                parts.pop()?;
            }
            _ => parts.push(part),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Point every link that names an imported file at where that file now lives.
/// Returns the new body and how many links moved; anything unresolvable is left
/// exactly as written, so a link to the wider internet survives untouched.
fn rewrite_links(body: &str, dir: &str, landed: &HashMap<String, Landed>) -> (String, usize) {
    let mut count = 0;

    let out = LINK.replace_all(body, |caps: &regex::Captures| {
        let whole = caps[0].to_string();
        let Some((path, suffix)) = split_target(&caps[3]) else {
            return whole;
        };
        let Some(key) = join_relative(dir, &path) else {
            return whole;
        };

        // A link written without the extension (`see [notes](notes)`) still
        // names the markdown file next to it.
        let target = landed
            .get(&key)
            .or_else(|| landed.get(&format!("{key}.md")))
            .or_else(|| landed.get(&format!("{key}.markdown")));

        match target {
            Some(Landed::Node(url)) => {
                count += 1;
                format!("{}[{}]({url}{suffix})", &caps[1], &caps[2])
            }
            // A fragment means nothing to a file download, so it is dropped.
            Some(Landed::Media(url)) => {
                count += 1;
                format!("{}[{}]({url})", &caps[1], &caps[2])
            }
            None => whole,
        }
    });

    (out.into_owned(), count)
}

/// Read the multipart body into uploads. A field whose file name ends in `.zip`
/// is expanded, so dragging in an archive and picking a folder arrive the same
/// way — the exporter writes one of those archives.
///
/// Nothing but markdown is held in memory: every other file is streamed to
/// `uploads/` as it arrives, and its id goes into `written` at once so a
/// failure further on can sweep it up.
async fn collect(
    state: &AppState,
    mut multipart: Multipart,
    skipped: &mut Vec<String>,
    written: &mut Vec<String>,
) -> AppResult<Vec<Upload>> {
    let mut uploads: Vec<Upload> = Vec::new();
    let mut budget = state.config.max_upload_bytes as u64;

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("malformed upload: {e}")))?
    {
        let name = field
            .file_name()
            .map(str::to_string)
            .unwrap_or_else(|| "upload".to_string());

        if name.to_ascii_lowercase().ends_with(".zip") {
            // The archive itself is only passing through: it lands under a
            // dotted name no media row can carry, and goes once it is unpacked.
            let id = format!(".import-{}", Uuid::new_v4());
            let size = media::store_field(state, &mut field, &id, &mut budget, None).await?;
            if size == 0 {
                skipped.push(name);
                continue;
            }

            let uploads_dir = state.config.uploads_dir();
            let limit = state.config.max_upload_bytes as u64;
            let archive = uploads_dir.join(&id);
            let opened = archive.clone();
            let expanded =
                tokio::task::spawn_blocking(move || expand_zip(&opened, &uploads_dir, limit))
                    .await
                    .map_err(anyhow::Error::from);
            let _ = tokio::fs::remove_file(&archive).await;

            let (mut files, mut refused, ids) = match expanded? {
                Ok(done) => done,
                Err((e, ids)) => {
                    written.extend(ids);
                    return Err(e);
                }
            };
            written.extend(ids);
            uploads.append(&mut files);
            skipped.append(&mut refused);
            continue;
        }

        let Some(path) = normalize_path(&name) else {
            skipped.push(name);
            continue;
        };

        if is_text(&path) {
            let mut text = Vec::new();
            while let Some(chunk) = field
                .chunk()
                .await
                .map_err(|e| AppError::BadRequest(format!("could not read upload: {e}")))?
            {
                if chunk.len() as u64 > budget {
                    return Err(media::too_large(state));
                }
                budget -= chunk.len() as u64;
                text.extend_from_slice(&chunk);
            }
            if text.is_empty() {
                skipped.push(path);
                continue;
            }
            let text = String::from_utf8_lossy(&text).to_string();
            uploads.push(Upload {
                content: text_content(&path, text),
                path,
            });
            continue;
        }

        let mime = media::mime_of(&path, field.content_type());
        let id = Uuid::new_v4().to_string();
        let size = media::store_field(state, &mut field, &id, &mut budget, None).await?;
        if size == 0 {
            skipped.push(path);
            continue;
        }
        written.push(id.clone());
        uploads.push(Upload {
            path,
            content: Content::File { id, mime, size },
        });
    }

    Ok(uploads)
}

/// What [`expand_zip`] made of an archive: the files, the refused paths, and the
/// ids written to `uploads/` — which on failure come back with the error, so
/// they can still be swept up.
type Expanded = (Vec<Upload>, Vec<String>, Vec<String>);

/// Unpack an archive from disk, on the blocking pool. Markdown is read into
/// memory, everything else is written straight to `uploads/`. What it unpacks
/// to is held to `limit` as well, because a zip's compressed size says nothing
/// about how much disk it takes.
fn expand_zip(
    archive: &std::path::Path,
    uploads_dir: &std::path::Path,
    limit: u64,
) -> Result<Expanded, (AppError, Vec<String>)> {
    let mut ids = Vec::new();
    match unpack(archive, uploads_dir, limit, &mut ids) {
        Ok((files, skipped)) => Ok((files, skipped, ids)),
        Err(e) => Err((e, ids)),
    }
}

fn unpack(
    archive: &std::path::Path,
    uploads_dir: &std::path::Path,
    limit: u64,
    ids: &mut Vec<String>,
) -> AppResult<(Vec<Upload>, Vec<String>)> {
    let mut uploads = Vec::new();
    let mut skipped = Vec::new();
    let mut unpacked = 0u64;

    let mut archive = zip::ZipArchive::new(std::fs::File::open(archive)?)
        .map_err(|e| AppError::BadRequest(format!("not a readable zip: {e}")))?;

    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|e| AppError::BadRequest(format!("damaged zip entry: {e}")))?;
        if file.is_dir() {
            continue;
        }

        let raw = file.name().to_string();
        // `enclosed_name` is the zip crate's own refusal of absolute paths,
        // `..` and symlink-style escapes; `normalize_path` then re-checks what
        // the tree builder relies on.
        let name = file
            .enclosed_name()
            .map(|path| path.to_string_lossy().to_string())
            .and_then(|path| normalize_path(&path));
        let Some(path) = name else {
            skipped.push(raw);
            continue;
        };

        // The entry is read through a cap one byte past what is left, so a
        // header that lies about the size cannot run past the limit either.
        let mut capped = std::io::Read::take(&mut file, limit - unpacked + 1);

        if is_text(&path) {
            let mut text = Vec::new();
            std::io::copy(&mut capped, &mut text)?;
            unpacked += text.len() as u64;
            if unpacked > limit {
                return Err(unpacked_too_large(limit));
            }
            if text.is_empty() {
                skipped.push(path);
                continue;
            }
            let text = String::from_utf8_lossy(&text).to_string();
            uploads.push(Upload {
                content: text_content(&path, text),
                path,
            });
            continue;
        }

        let id = Uuid::new_v4().to_string();
        let target = uploads_dir.join(&id);
        let partial = target.with_extension("part");
        ids.push(id.clone());
        // Written aside and renamed into place whole, like any other upload,
        // so a name the database may come to know never holds half a file.
        let mut out = std::fs::File::create(&partial)?;
        let copied = std::io::copy(&mut capped, &mut out).and_then(|size| {
            out.sync_all()?;
            Ok(size)
        });
        let size = match copied {
            Ok(size) => size,
            Err(e) => {
                let _ = std::fs::remove_file(&partial);
                return Err(e.into());
            }
        };
        std::fs::rename(&partial, &target)?;
        unpacked += size;
        if unpacked > limit {
            return Err(unpacked_too_large(limit));
        }
        if size == 0 {
            skipped.push(path);
            continue;
        }

        uploads.push(Upload {
            content: Content::File {
                id,
                mime: media::mime_of(&path, None),
                size,
            },
            path,
        });
    }

    Ok((uploads, skipped))
}

fn unpacked_too_large(limit: u64) -> AppError {
    AppError::BadRequest(format!(
        "the archive unpacks to more than the {} MB limit",
        limit / 1024 / 1024
    ))
}

/// Where an import goes: under a space or folder, or at the root, where the
/// top-level directories of the upload are spaces.
enum Into {
    Node(Box<nodes::Node>),
    Root,
}

/// `POST /api/nodes/{id}/import` — a folder, or a zip of one, becomes a tree
/// under the given space or folder.
pub async fn import(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    multipart: Multipart,
) -> AppResult<Json<ImportReport>> {
    let target = nodes::load(&state, id).await?;
    if !nodes::is_container(&target.kind) {
        return Err(AppError::BadRequest(
            "a folder can only be imported into a space or a folder".into(),
        ));
    }
    run(&state, Into::Node(Box::new(target)), multipart)
        .await
        .map(Json)
}

/// `POST /api/import` — an export poured back in whole: each top-level
/// directory becomes a space, or goes into the space that already has its
/// name, and the boards, dates and names come back from `.workspace.json`.
pub async fn import_root(
    _: Session,
    State(state): State<AppState>,
    multipart: Multipart,
) -> AppResult<Json<ImportReport>> {
    run(&state, Into::Root, multipart).await.map(Json)
}

async fn run(state: &AppState, into: Into, multipart: Multipart) -> AppResult<ImportReport> {
    // The files land on disk before the transaction so a failed import writes
    // nothing to the database; the orphans are swept up on the way out.
    let mut skipped = Vec::new();
    let mut written: Vec<String> = Vec::new();
    // Into a sealed folder, everything that is not a document goes sealed,
    // and with the vault shut nothing is read at all.
    let key = match &into {
        Into::Node(target) if target.sealed => Some(state.vault.key().ok_or(AppError::Locked)?),
        _ => None,
    };
    let result = async {
        let mut uploads = collect(state, multipart, &mut skipped, &mut written).await?;
        let manifest = take_manifest(&mut uploads)?;
        if let Some(manifest) = &manifest {
            files_as_they_were(state, manifest, &mut uploads, &mut written).await?;
        }
        if let Some(key) = &key {
            seal_in_place(state, key.current(), &uploads).await?;
        }
        if uploads.is_empty() {
            return Err(AppError::BadRequest(
                "nothing importable in that upload".into(),
            ));
        }
        build(state, &into, uploads, skipped, manifest).await
    }
    .await;
    if result.is_err() {
        for media_id in &written {
            media::remove_upload(&state.config.uploads_dir(), media_id).await;
        }
    }

    let report = result?;
    state.backup.signal();
    Ok(report)
}

/// Seal every file of the upload where it lies, under the id it already has.
/// They were written plain, an archive's contents can only be — but no row
/// names them yet, so neither the app nor the mirror has seen them.
async fn seal_in_place(state: &AppState, key: &vault::Key, uploads: &[Upload]) -> AppResult<()> {
    let ids: Vec<String> = uploads
        .iter()
        .filter_map(|upload| match &upload.content {
            Content::File { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect();
    let (key, dir) = (key.clone(), state.config.uploads_dir());
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        for id in ids {
            let plain = dir.join(&id);
            let sealed = dir.join(format!("{id}.part"));
            vault::seal_file(&key, &id, &mut std::fs::File::open(&plain)?, &sealed)?;
            std::fs::rename(&sealed, &plain)?;
        }
        Ok(())
    })
    .await
    .map_err(anyhow::Error::from)??;
    Ok(())
}

/// A file that happens to end in `.md` was a file, not a document, when it
/// was exported, and the manifest says so: it goes back to being one.
async fn files_as_they_were(
    state: &AppState,
    manifest: &Manifest,
    uploads: &mut [Upload],
    written: &mut Vec<String>,
) -> AppResult<()> {
    for upload in uploads.iter_mut() {
        let was_file = manifest
            .nodes
            .get(&upload.path)
            .is_some_and(|m| m.kind == FILE);
        let Content::Markdown(text) = &upload.content else {
            continue;
        };
        if !was_file {
            continue;
        }
        let id = Uuid::new_v4().to_string();
        let path = state.config.uploads_dir().join(&id);
        let partial = path.with_extension("part");
        written.push(id.clone());
        tokio::fs::write(&partial, text.as_bytes()).await?;
        tokio::fs::File::open(&partial).await?.sync_all().await?;
        tokio::fs::rename(&partial, &path).await?;
        upload.content = Content::File {
            mime: media::mime_of(&upload.path, None),
            size: text.len() as u64,
            id,
        };
    }
    Ok(())
}

/// `a/b` from a prefix and a segment, where an empty prefix is the root.
fn join(prefix: &str, segment: &str) -> String {
    if prefix.is_empty() {
        segment.to_string()
    } else {
        format!("{prefix}/{segment}")
    }
}

/// Take the manifest out of the upload, if there is one, and make every
/// other path relative to where it was found: an export uploaded as a
/// folder arrives inside a directory of its own.
fn take_manifest(uploads: &mut Vec<Upload>) -> AppResult<Option<Manifest>> {
    let Some(at) = uploads
        .iter()
        .enumerate()
        .filter(|(_, u)| matches!(u.content, Content::Manifest(_)))
        .min_by_key(|(_, u)| u.path.matches('/').count())
        .map(|(i, _)| i)
    else {
        return Ok(None);
    };
    let found = uploads.remove(at);
    let Content::Manifest(text) = found.content else {
        unreachable!("filtered on the variant above");
    };
    let manifest: Manifest = serde_json::from_str(&text).map_err(|e| {
        AppError::BadRequest(format!("{} is not readable: {e}", manifest::FILE_NAME))
    })?;
    if manifest.format > manifest::FORMAT {
        return Err(AppError::BadRequest(format!(
            "{} is from a newer version (format {}); update this one first",
            manifest::FILE_NAME,
            manifest.format
        )));
    }

    let base = parent_of(&found.path);
    if !base.is_empty() {
        let prefix = format!("{base}/");
        for upload in uploads.iter_mut() {
            if let Some(rest) = upload.path.strip_prefix(&prefix) {
                upload.path = rest.to_string();
            }
        }
    }
    // Any other manifest further down is not one to read; it goes in as text.
    for upload in uploads.iter_mut() {
        if let Content::Manifest(text) = &mut upload.content {
            upload.content = Content::Markdown(std::mem::take(text));
        }
    }
    Ok(Some(manifest))
}

async fn build(
    state: &AppState,
    into: &Into,
    mut uploads: Vec<Upload>,
    mut skipped: Vec<String>,
    manifest: Option<Manifest>,
) -> AppResult<ImportReport> {
    let uploads_dir = state.config.uploads_dir();

    // The path of the target in slugs, which every rewritten link starts with.
    let prefix = match into {
        Into::Node(target) => target
            .path
            .iter()
            .map(|crumb| crumb.slug.as_str())
            .collect::<Vec<_>>()
            .join("/"),
        Into::Root => String::new(),
    };

    // Sorted so a parent directory is always created before its children, and
    // so two runs of the same folder produce the same slugs.
    // A duplicate's bytes are already on disk; the copy that loses goes too.
    uploads.sort_by(|a, b| a.path.cmp(&b.path));
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut unique = Vec::with_capacity(uploads.len());
    for upload in uploads {
        // An export's README describes the archive, not the workspace.
        let boilerplate = manifest.is_some() && upload.path == "README.txt";
        // At the root only directories can land: everything is in a space.
        let loose = matches!(into, Into::Root) && !upload.path.contains('/');
        if seen.insert(upload.path.clone()) && !boilerplate && !loose {
            unique.push(upload);
            continue;
        }
        if loose && !boilerplate {
            skipped.push(upload.path.clone());
        }
        if let Content::File { id, .. } = &upload.content {
            media::remove_upload(&uploads_dir, id).await;
        }
    }
    let uploads = unique;

    // An export keeps the files documents embed in `media/`, beside the spaces;
    // they come back as what they were, embedded files, not as a folder.
    let embedded = |path: &str| manifest.is_some() && parent_of(path) == manifest::MEDIA_DIR;
    let meta = |path: &str| -> Option<&NodeMeta> { manifest.as_ref()?.nodes.get(path) };

    let mut directories: BTreeSet<String> = BTreeSet::new();
    for upload in uploads.iter().filter(|u| !embedded(&u.path)) {
        let mut dir = parent_of(&upload.path);
        while !dir.is_empty() {
            directories.insert(dir.to_string());
            dir = parent_of(dir);
        }
    }
    // Empty folders exist in an export only as names in the manifest.
    if let Some(manifest) = &manifest {
        for (path, node) in &manifest.nodes {
            let is_dir = nodes::is_container(&node.kind);
            if is_dir && normalize_path(path).as_deref() == Some(path.as_str()) {
                let mut dir = path.as_str();
                while !dir.is_empty() {
                    directories.insert(dir.to_string());
                    dir = parent_of(dir);
                }
            }
        }
    }

    let now = crate::now();
    let mut tx = crate::db::begin_write(&state.db).await?;

    // Slugs already taken under each parent, extended as the tree is built —
    // the nodes being created are not visible to a query until the commit.
    let mut siblings: HashMap<i64, BTreeSet<String>> = HashMap::new();
    let mut dir_ids: HashMap<String, i64> = HashMap::new();
    let mut dir_paths: HashMap<String, String> = HashMap::new();
    if let Into::Node(target) = into {
        siblings.insert(
            target.id,
            nodes::sibling_slugs(&mut tx, Some(target.id), None).await?,
        );
        dir_ids.insert(String::new(), target.id);
        dir_paths.insert(String::new(), prefix);
    }
    let mut landed: HashMap<String, Landed> = HashMap::new();
    let mut spaces = 0usize;
    let mut folders = 0usize;

    for dir in &directories {
        let dated = meta(dir);
        let name = dated.map_or(file_name(dir), |m| m.name.as_str());
        let (created, updated) = dated.map_or((now, now), |m| (m.created_at, m.updated_at));

        let (id, slug) = if parent_of(dir).is_empty() && matches!(into, Into::Root) {
            // A space: the one already here by that name, or a new one.
            let slug = slugify(file_name(dir));
            let existing: Option<i64> =
                sqlx::query_scalar("SELECT id FROM nodes WHERE parent_id IS NULL AND slug = ?1")
                    .bind(&slug)
                    .fetch_optional(&mut *tx)
                    .await?;
            let id = match existing {
                Some(id) => id,
                None => {
                    let space = manifest.as_ref().and_then(|m| m.spaces.get(dir));
                    let position: i64 = sqlx::query_scalar(
                        "SELECT COALESCE(MAX(position), -1) + 1 FROM nodes WHERE parent_id IS NULL",
                    )
                    .fetch_one(&mut *tx)
                    .await?;
                    spaces += 1;
                    sqlx::query_scalar(
                        "INSERT INTO nodes (parent_id, kind, name, slug, position, has_board, created_at, updated_at)
                         VALUES (NULL, ?1, ?2, ?3, ?4, ?5, ?6, ?7)
                         RETURNING id",
                    )
                    .bind(SPACE)
                    .bind(name)
                    .bind(&slug)
                    .bind(position)
                    .bind(space.is_none_or(|s| s.has_board))
                    .bind(created)
                    .bind(updated)
                    .fetch_one(&mut *tx)
                    .await?
                }
            };
            siblings.insert(id, nodes::sibling_slugs(&mut tx, Some(id), None).await?);
            (id, slug)
        } else {
            let parent = dir_ids[parent_of(dir)];
            let taken = siblings.entry(parent).or_default();
            let slug = next_slug(taken, &slugify(file_name(dir)))?;
            taken.insert(slug.clone());
            folders += 1;
            let id = sqlx::query_scalar(
                "INSERT INTO nodes (parent_id, kind, name, slug, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 RETURNING id",
            )
            .bind(parent)
            .bind(FOLDER)
            .bind(name)
            .bind(&slug)
            .bind(created)
            .bind(updated)
            .fetch_one(&mut *tx)
            .await?;
            (id, slug)
        };

        let path = join(
            &dir_paths.get(parent_of(dir)).cloned().unwrap_or_default(),
            &slug,
        );
        dir_ids.insert(dir.clone(), id);
        dir_paths.insert(dir.clone(), path.clone());
        landed.insert(dir.clone(), Landed::Node(format!("/n/{path}")));
    }

    // Documents first, with their bodies as uploaded: the rewriting below needs
    // to know where every document landed before it can point anything at one.
    let mut documents: Vec<(i64, String, String)> = Vec::new();
    let mut by_path: HashMap<String, i64> = HashMap::new();
    let mut files = 0usize;

    for upload in &uploads {
        let dated = meta(&upload.path);
        let (created, updated) = dated.map_or((now, now), |m| (m.created_at, m.updated_at));

        if let Content::File {
            id: media_id,
            mime,
            size,
        } = &upload.content
        {
            let original = manifest
                .as_ref()
                .and_then(|m| m.media.get(&upload.path))
                .map(String::as_str);
            let filename = original
                .or(dated.map(|m| m.name.as_str()))
                .unwrap_or(file_name(&upload.path));
            sqlx::query(
                "INSERT INTO media (id, filename, mime, size, created_at, encrypted)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .bind(media_id)
            .bind(filename)
            .bind(mime)
            .bind(*size as i64)
            .bind(created)
            .bind(matches!(into, Into::Node(target) if target.sealed))
            .execute(&mut *tx)
            .await?;
            landed.insert(
                upload.path.clone(),
                Landed::Media(format!("/api/media/{media_id}")),
            );
            if embedded(&upload.path) {
                continue;
            }

            // The file sits in the tree under the name it had, and owns its
            // bytes the way a document owns what it embeds — so deleting the
            // folder it landed in takes it along.
            let parent = dir_ids[parent_of(&upload.path)];
            let taken = siblings.entry(parent).or_default();
            let slug = next_slug(taken, &slugify(filename))?;
            taken.insert(slug.clone());

            let node: i64 = sqlx::query_scalar(
                "INSERT INTO nodes (parent_id, kind, name, slug, created_at, updated_at, media_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 RETURNING id",
            )
            .bind(parent)
            .bind(FILE)
            .bind(filename)
            .bind(&slug)
            .bind(created)
            .bind(updated)
            .bind(media_id)
            .fetch_one(&mut *tx)
            .await?;

            sqlx::query("INSERT INTO node_media (node_id, media_id) VALUES (?1, ?2)")
                .bind(node)
                .bind(media_id)
                .execute(&mut *tx)
                .await?;
            files += 1;
        } else if let Content::Markdown(raw) = &upload.content {
            let dir = parent_of(&upload.path);
            let parent = dir_ids[dir];
            let (title, body) = split_title(raw, &upload.path);
            // The manifest knows the name as it was typed, including none:
            // a document with no title was exported without a heading.
            let name = dated.map_or(title, |m| m.name.clone());
            // The slug follows the file name rather than the title, so a link
            // someone wrote to `01-was-ist-eebus.md` keeps reading that way.
            let stem = file_name(&upload.path);
            let stem = stem.rsplit_once('.').map_or(stem, |(stem, _)| stem);

            let taken = siblings.entry(parent).or_default();
            let slug = next_slug(taken, &slugify(stem))?;
            taken.insert(slug.clone());

            let id: i64 = sqlx::query_scalar(
                "INSERT INTO nodes (parent_id, kind, name, slug, body, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 RETURNING id",
            )
            .bind(parent)
            .bind(DOCUMENT)
            .bind(&name)
            .bind(&slug)
            .bind(&body)
            .bind(created)
            .bind(updated)
            .fetch_one(&mut *tx)
            .await?;

            let path = join(&dir_paths[dir], &slug);
            landed.insert(upload.path.clone(), Landed::Node(format!("/n/{path}")));
            by_path.insert(upload.path.clone(), id);
            documents.push((id, upload.path.clone(), body));
        }
    }

    // Second pass: point the links at where things landed, and attach every
    // file a document embeds to it the way a hand-written document does.
    let mut links = 0usize;

    for (id, path, body) in &documents {
        let (rewritten, count) = rewrite_links(body, parent_of(path), &landed);
        links += count;

        if rewritten != *body {
            sqlx::query("UPDATE nodes SET body = ?1 WHERE id = ?2")
                .bind(&rewritten)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        media::link_to_node(&mut tx, *id, &rewritten).await?;
        links::index(&mut tx, *id, &rewritten).await?;
    }

    // The boards come back only where spaces do.
    let mut cards = 0usize;
    if let (Into::Root, Some(manifest)) = (into, &manifest) {
        for (dir, space) in &manifest.spaces {
            let Some(&space_id) = dir_ids.get(dir) else {
                continue;
            };
            cards += boards::restore(&mut tx, space_id, &space.lists, &by_path).await?;
            if space.role.as_deref() == Some(nodes::DIARY) {
                // Only where no space has the role yet: there is one diary.
                sqlx::query(
                    "UPDATE nodes SET role = ?1, has_board = 0
                     WHERE id = ?2 AND NOT EXISTS (SELECT 1 FROM nodes WHERE role = ?1)",
                )
                .bind(nodes::DIARY)
                .bind(space_id)
                .execute(&mut *tx)
                .await?;
            }
        }
    }

    tx.commit().await?;

    Ok(ImportReport {
        spaces,
        folders,
        documents: documents.len(),
        files,
        links,
        cards,
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{
        expand_zip, is_markdown, join_relative, normalize_path, percent_decode, rewrite_links,
        split_target, split_title, take_manifest, Content, Landed, Upload,
    };

    /// Into a sealed folder, a file arrives sealed — readable back through
    /// the vault — and the folders made for it are sealed too.
    #[tokio::test]
    async fn an_import_into_a_sealed_folder_arrives_sealed() {
        use std::io::Read;

        let dir = tempfile::tempdir().unwrap();
        let uploads_dir = dir.path().join("uploads");
        std::fs::create_dir_all(&uploads_dir).unwrap();
        let state = crate::routes::tests::state_in(dir.path().into()).await;
        let key = zeroize::Zeroizing::new([5u8; 32]);
        let folder: i64 = sqlx::query_scalar(
            "INSERT INTO nodes (parent_id, kind, name, slug, created_at, updated_at, sealed)
             VALUES (NULL, 'space', 'locked', 'locked', 1, 1, 1) RETURNING id",
        )
        .fetch_one(&state.db)
        .await
        .unwrap();

        std::fs::write(uploads_dir.join("m1"), b"private bytes").unwrap();
        let uploads = vec![
            text(
                "sub/a.bin",
                Content::File {
                    id: "m1".into(),
                    mime: "application/octet-stream".into(),
                    size: 13,
                },
            ),
            text("notes.md", Content::Markdown("# Notes".into())),
        ];
        super::seal_in_place(&state, &key, &uploads).await.unwrap();
        let target = crate::routes::nodes::load(&state, folder).await.unwrap();
        super::build(
            &state,
            &super::Into::Node(Box::new(target)),
            uploads,
            Vec::new(),
            None,
        )
        .await
        .unwrap();

        let (encrypted, sub_sealed): (bool, bool) = sqlx::query_as(
            "SELECT m.encrypted, p.sealed FROM nodes n
             JOIN media m ON m.id = n.media_id JOIN nodes p ON p.id = n.parent_id
             WHERE n.kind = 'file'",
        )
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert!(encrypted && sub_sealed);
        let mut back = String::new();
        crate::vault::Keyring::new(key.clone())
            .reader("m1", &uploads_dir.join("m1"))
            .unwrap()
            .read_to_string(&mut back)
            .unwrap();
        assert_eq!(back, "private bytes");
    }

    fn text(path: &str, content: Content) -> Upload {
        Upload {
            path: path.into(),
            content,
        }
    }

    #[test]
    fn an_export_uploaded_as_a_folder_is_read_from_where_its_manifest_is() {
        let manifest = r#"{"format": 1, "nodes": {"work/a.md": {"kind": "document", "name": "A", "created_at": 1, "updated_at": 2}}}"#;
        let mut uploads = vec![
            text(
                "workspace-2026/.workspace.json",
                Content::Manifest(manifest.into()),
            ),
            text("workspace-2026/work/a.md", Content::Markdown("a".into())),
            text(
                "workspace-2026/work/deeper/.workspace.json",
                Content::Manifest("{}".into()),
            ),
        ];
        let found = take_manifest(&mut uploads).unwrap().unwrap();
        assert_eq!(found.nodes["work/a.md"].created_at, 1);
        let paths: Vec<&str> = uploads.iter().map(|u| u.path.as_str()).collect();
        assert_eq!(paths, ["work/a.md", "work/deeper/.workspace.json"]);
        // Only the outermost one is read; another is just a file.
        assert!(matches!(uploads[1].content, Content::Markdown(_)));
    }

    #[test]
    fn a_manifest_from_a_newer_version_is_refused() {
        let mut uploads = vec![text(
            ".workspace.json",
            Content::Manifest(r#"{"format": 99}"#.into()),
        )];
        assert!(take_manifest(&mut uploads).is_err());
    }

    /// A zip with a note and a 4 KiB blob in it, written to `dir/in.zip`.
    fn archive(dir: &std::path::Path) -> std::path::PathBuf {
        use std::io::Write;
        let path = dir.join("in.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).expect("zip file"));
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("notes/a.md", options).expect("entry");
        zip.write_all(b"# A\n\ntext").expect("write");
        zip.start_file("notes/blob.bin", options).expect("entry");
        zip.write_all(&[7u8; 4096]).expect("write");
        zip.start_file("../escape.txt", options).expect("entry");
        zip.write_all(b"nope").expect("write");
        zip.finish().expect("finish");
        path
    }

    #[test]
    fn a_zip_unpacks_markdown_to_memory_and_the_rest_to_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (uploads, skipped, ids) = expand_zip(&archive(dir.path()), dir.path(), 1 << 20)
            .unwrap_or_else(|(e, _)| panic!("{e:?}"));

        assert_eq!(skipped, ["../escape.txt"]);
        assert_eq!(ids.len(), 1);
        let blob = uploads
            .iter()
            .find(|u| u.path == "notes/blob.bin")
            .expect("the blob");
        let Content::File { id, size, .. } = &blob.content else {
            panic!("the blob is a file")
        };
        assert_eq!(*size, 4096);
        assert_eq!(
            std::fs::metadata(dir.path().join(id))
                .expect("on disk")
                .len(),
            4096
        );
        assert!(uploads.iter().any(|u| u.path == "notes/a.md"
            && matches!(&u.content, Content::Markdown(t) if t.contains("text"))));
    }

    #[test]
    fn a_zip_that_unpacks_past_the_limit_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let Err((_, ids)) = expand_zip(&archive(dir.path()), dir.path(), 1024) else {
            panic!("4 KiB unpacked must not fit in 1 KiB");
        };
        // Whatever was written before the limit hit is reported, to be swept up.
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn refuses_paths_that_escape_the_import() {
        assert_eq!(
            normalize_path("docs/readme.md").as_deref(),
            Some("docs/readme.md")
        );
        assert_eq!(
            normalize_path("./docs//readme.md").as_deref(),
            Some("docs/readme.md")
        );
        assert_eq!(
            normalize_path("docs\\readme.md").as_deref(),
            Some("docs/readme.md")
        );
        assert_eq!(normalize_path("../secrets"), None);
        assert_eq!(normalize_path("docs/../../etc/passwd"), None);
        assert_eq!(normalize_path("/etc/passwd"), None);
        assert_eq!(normalize_path("C:/windows/system32"), None);
        assert_eq!(normalize_path("   "), None);
    }

    #[test]
    fn a_title_comes_from_the_heading_then_the_file_name() {
        assert_eq!(
            split_title("# Was ist EEBUS?\n\ntext\n", "a/01.md"),
            ("Was ist EEBUS?".into(), "text".into())
        );
        // No heading, or one further down: the body is left exactly as it came.
        assert_eq!(
            split_title("no heading", "a/01-intro.md"),
            ("01-intro".into(), "no heading".into())
        );
        assert_eq!(
            split_title("#nope\n\n# Real\n", "x.md"),
            ("x".into(), "#nope\n\n# Real\n".into())
        );
    }

    #[test]
    fn markdown_is_recognised_by_extension() {
        assert!(is_markdown("a/README.MD"));
        assert!(is_markdown("notes.markdown"));
        assert!(!is_markdown("diagram.png"));
    }

    #[test]
    fn relative_links_resolve_against_their_document() {
        assert_eq!(
            join_relative("docs", "img/a.png").as_deref(),
            Some("docs/img/a.png")
        );
        assert_eq!(
            join_relative("docs/deep", "../a.md").as_deref(),
            Some("docs/a.md")
        );
        assert_eq!(join_relative("", "a.md").as_deref(), Some("a.md"));
        assert_eq!(join_relative("docs", "../../outside.md"), None);
    }

    #[test]
    fn external_targets_are_not_touched() {
        assert!(split_target("https://example.org/x").is_none());
        assert!(split_target("#section").is_none());
        assert!(split_target("/api/media/x").is_none());
        assert!(split_target("mailto:someone@example.org").is_none());
        assert_eq!(
            split_target("a%20b.md#top"),
            Some(("a b.md".into(), "#top"))
        );
    }

    #[test]
    fn escapes_next_to_multibyte_characters_do_not_panic() {
        assert_eq!(percent_decode("%aé"), "%aé");
        assert_eq!(percent_decode("é%"), "é%");
        assert_eq!(percent_decode("%é"), "%é");
        assert_eq!(percent_decode("a%20"), "a ");
        assert_eq!(percent_decode("%C3%A9"), "é");
        // Bytes that do not decode to UTF-8 leave the target as it was written.
        assert_eq!(percent_decode("%FF"), "%FF");
    }

    #[test]
    fn links_point_at_where_the_files_landed() {
        let landed = HashMap::from([
            (
                "einarbeiten/01.md".to_string(),
                Landed::Node("/n/work/einarbeiten/01".into()),
            ),
            (
                "einarbeiten/img/a.png".to_string(),
                Landed::Media("/api/media/uuid".into()),
            ),
        ]);

        let body = "see [one](01.md#ziele) and ![pic](img/a.png), \
                    plus [the web](https://example.org) and [gone](missing.md)";
        let (out, count) = rewrite_links(body, "einarbeiten", &landed);

        assert_eq!(count, 2);
        assert!(out.contains("[one](/n/work/einarbeiten/01#ziele)"), "{out}");
        assert!(out.contains("![pic](/api/media/uuid)"), "{out}");
        assert!(out.contains("[the web](https://example.org)"), "{out}");
        assert!(out.contains("[gone](missing.md)"), "{out}");
    }

    #[test]
    fn a_link_from_a_sibling_directory_still_resolves() {
        let landed =
            HashMap::from([("a/one.md".to_string(), Landed::Node("/n/work/a/one".into()))]);
        let (out, count) = rewrite_links("[x](../a/one.md)", "b", &landed);
        assert_eq!(count, 1);
        assert_eq!(out, "[x](/n/work/a/one)");
    }
}
