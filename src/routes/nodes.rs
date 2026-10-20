use std::collections::{BTreeSet, HashSet};

use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqliteConnection, SqlitePool};

use crate::{
    auth::Session,
    error::{AppError, AppResult},
    routes::{media, trash},
    state::AppState,
};

/// What a node is. Spaces are the roots (`diary`, `work`, `uni`, ...), folders
/// group things inside them to any depth, documents are the leaves that hold
/// text and files the leaves that hold anything else. One table, one recursion,
/// so a space can nest exactly like a folder.
pub const SPACE: &str = "space";
pub const FOLDER: &str = "folder";
pub const DOCUMENT: &str = "document";
pub const FILE: &str = "file";

/// The `role` of the one space that is the diary: a journal, with no board,
/// whose empty days the reminder mail is about. Found by role rather than by
/// slug, so it stays the diary under any name.
pub const DIARY: &str = "diary";

/// The nodes in the tree proper — the spaces and everything under them, so
/// not the trash — for a query that reads every node rather than walking down
/// from one: `WITH RECURSIVE {LIVE} SELECT … WHERE id IN (SELECT id FROM live)`.
pub const LIVE: &str = "live (id) AS (
         SELECT id FROM nodes WHERE parent_id IS NULL AND kind = 'space'
         UNION ALL
         SELECT n.id FROM nodes n JOIN live ON n.parent_id = live.id
     )";

/// Whether a node of this kind can hold other nodes.
pub fn is_container(kind: &str) -> bool {
    kind == SPACE || kind == FOLDER
}

/// The bytes behind a file node, as the client needs them to show or fetch it.
#[derive(Serialize)]
pub struct FileInfo {
    pub media_id: String,
    pub mime: String,
    pub size: i64,
    pub url: String,
    /// Sealed in the vault: it opens only while the vault does.
    pub encrypted: bool,
}

#[derive(Serialize)]
pub struct NodeSummary {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub kind: String,
    pub name: String,
    pub slug: String,
    pub excerpt: String,
    pub position: i64,
    pub has_board: bool,
    /// Nodes the code has to tell apart: `diary`, `trash`, or nothing.
    pub role: Option<String>,
    pub child_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub shared: bool,
    pub share_token: Option<String>,
    /// Folders and spaces only: what lands here is sealed in the vault.
    pub sealed: bool,
    /// Files only.
    pub file: Option<FileInfo>,
    /// Search results only: where the match sits, `work/notes`, so two
    /// matches with the same name can be told apart.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

#[derive(Serialize)]
pub struct Node {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub kind: String,
    pub name: String,
    pub slug: String,
    pub body: String,
    /// The same preview the list shows, so a client that has just saved can
    /// patch its list row without re-reading the whole list.
    pub excerpt: String,
    pub position: i64,
    pub has_board: bool,
    pub role: Option<String>,
    pub child_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub shared: bool,
    pub share_token: Option<String>,
    pub sealed: bool,
    pub file: Option<FileInfo>,
    /// This node's ancestors, the space first and the node itself last — the
    /// breadcrumb, and everything needed to build its /n/... path.
    pub path: Vec<Crumb>,
}

#[derive(Serialize)]
pub struct Crumb {
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub slug: String,
}

#[derive(Deserialize)]
pub struct ListQuery {
    /// Children of this node. Omitted together with `q` means the spaces.
    #[serde(default)]
    pub parent: Option<i64>,
    #[serde(default)]
    pub q: Option<String>,
    /// Confine a search to one space.
    #[serde(default)]
    pub space: Option<i64>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

#[derive(Deserialize)]
pub struct CreateInput {
    pub parent_id: i64,
    /// `document` (the default) or `folder`.
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub body: String,
    /// Optional override so a document can be backdated to the day it is about.
    #[serde(default)]
    pub created_at: Option<i64>,
}

#[derive(Deserialize)]
pub struct UpdateInput {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub created_at: Option<i64>,
    /// Spaces only, and only when it is sent at all.
    #[serde(default)]
    pub has_board: Option<bool>,
    /// The `updated_at` the writer last saw. When the node has been saved
    /// since — from another tab, another device, the SFTP mount — the write
    /// is refused rather than silently undoing that save. Left out, the
    /// write goes through whatever happened in between.
    #[serde(default)]
    pub base_updated_at: Option<i64>,
}

#[derive(Deserialize)]
pub struct SpaceInput {
    pub name: String,
    /// Defaults to on: a space is usually a project, and the one place a board
    /// makes no sense — the diary — already exists.
    #[serde(default)]
    pub has_board: Option<bool>,
}

#[derive(Deserialize)]
pub struct MoveInput {
    pub parent_id: i64,
    #[serde(default)]
    pub position: Option<i64>,
}

#[derive(Deserialize)]
pub struct MoveManyInput {
    pub ids: Vec<i64>,
    pub parent_id: i64,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Moved {
    pub moved: usize,
    /// Already in the folder they were sent to.
    pub unchanged: usize,
}

/// How many nodes one move takes: a folder's worth picked up by hand, with
/// room to spare, and still one short transaction.
const MOVE_AT_ONCE: usize = 1000;

/// How much of a body the list query reads. Comfortably more than an excerpt
/// needs, and a rounding error next to documents that run to thousands of words.
const HEAD_CHARS: usize = 1024;
const EXCERPT_CHARS: usize = 180;

/// The columns every list query selects, so a row always converts the same way.
const SUMMARY_COLUMNS: &str =
    "n.id, n.parent_id, n.kind, n.name, n.slug, n.position, n.has_board, n.role,
     n.created_at, n.updated_at, n.share_token, n.media_id, n.sealed,
     (SELECT COUNT(*) FROM nodes c WHERE c.parent_id = n.id) AS child_count,
     (SELECT m.mime FROM media m WHERE m.id = n.media_id) AS file_mime,
     (SELECT m.size FROM media m WHERE m.id = n.media_id) AS file_size,
     (SELECT m.encrypted FROM media m WHERE m.id = n.media_id) AS file_encrypted";

/// A short, plain-ish preview of the body for the list pane. `head` is only the
/// leading `HEAD_CHARS` of the document, so truncation is inferred from its
/// length rather than measured against the whole body.
fn excerpt(head: &str) -> String {
    let flat: String = head
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("![") && !line.starts_with("```"))
        .map(|line| line.trim_start_matches(['#', '>', '-', '*', ' ']))
        .collect::<Vec<_>>()
        .join(" ");
    let mut out: String = flat.chars().take(EXCERPT_CHARS).collect();
    if flat.chars().count() > EXCERPT_CHARS || head.chars().count() >= HEAD_CHARS {
        out.push('…');
    }
    out
}

/// Turn free text into an FTS5 prefix query, discarding anything that could be
/// read as FTS syntax.
fn fts_query(raw: &str) -> Option<String> {
    let terms: Vec<String> = raw
        .split_whitespace()
        .map(|term| term.replace(|c: char| !c.is_alphanumeric(), " "))
        .flat_map(|term| {
            term.split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .map(|term| format!("\"{term}\"*"))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" AND "))
}

/// A name as a path segment: lowercase, ASCII-ish, no surprises in a URL. An
/// empty result becomes `untitled`, because every node needs a slug and a
/// document usually gets its name minutes after it is created.
pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut dash = false;
    for ch in name.trim().chars().flat_map(|c| c.to_lowercase()) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            dash = false;
        } else if matches!(ch, 'ä' | 'ö' | 'ü' | 'ß') {
            // The diary is written in two languages; transliterating rather
            // than dropping keeps a German heading readable in its URL.
            slug.push_str(match ch {
                'ä' => "ae",
                'ö' => "oe",
                'ü' => "ue",
                _ => "ss",
            });
            dash = false;
        } else if !slug.is_empty() && !dash {
            slug.push('-');
            dash = true;
        }
    }
    let slug = slug.trim_end_matches('-').to_string();
    let slug: String = slug.chars().take(80).collect();
    let slug = slug.trim_end_matches('-').to_string();
    if slug.is_empty() {
        "untitled".to_string()
    } else {
        slug
    }
}

/// `base`, or `base-2`, `base-3`… until one is free. Kept separate from the
/// database so the importer, which creates a whole tree of siblings before any
/// of them is queryable, can allocate against the set it is building.
pub fn next_slug(taken: &BTreeSet<String>, base: &str) -> AppResult<String> {
    for suffix in 1..1000 {
        let candidate = if suffix == 1 {
            base.to_string()
        } else {
            format!("{base}-{suffix}")
        };
        if !taken.contains(&candidate) {
            return Ok(candidate);
        }
    }
    Err(AppError::BadRequest(
        "too many siblings with that name".into(),
    ))
}

/// The slugs already spoken for under `parent_id`. `exclude` is the node being
/// renamed, which must not collide with itself.
pub async fn sibling_slugs(
    db: &mut SqliteConnection,
    parent_id: Option<i64>,
    exclude: Option<i64>,
) -> AppResult<BTreeSet<String>> {
    let rows = sqlx::query(
        "SELECT slug FROM nodes
         WHERE parent_id IS ?1 AND (?2 IS NULL OR id <> ?2)",
    )
    .bind(parent_id)
    .bind(exclude)
    .fetch_all(&mut *db)
    .await?;

    Ok(rows.iter().map(|row| row.get("slug")).collect())
}

/// `base`, or `base-2`, `base-3`… until no sibling holds it.
pub async fn unique_slug(
    db: &mut SqliteConnection,
    parent_id: Option<i64>,
    base: &str,
    exclude: Option<i64>,
) -> AppResult<String> {
    next_slug(&sibling_slugs(db, parent_id, exclude).await?, base)
}

fn row_to_summary(row: &sqlx::sqlite::SqliteRow, head: &str) -> NodeSummary {
    let share_token: Option<String> = row.get("share_token");
    let file = row
        .get::<Option<String>, _>("media_id")
        .map(|media_id| FileInfo {
            url: format!("/api/media/{media_id}"),
            media_id,
            mime: row
                .get::<Option<String>, _>("file_mime")
                .unwrap_or_default(),
            size: row.get::<Option<i64>, _>("file_size").unwrap_or_default(),
            encrypted: row
                .get::<Option<bool>, _>("file_encrypted")
                .unwrap_or_default(),
        });
    NodeSummary {
        id: row.get("id"),
        parent_id: row.get("parent_id"),
        kind: row.get("kind"),
        name: row.get("name"),
        slug: row.get("slug"),
        excerpt: excerpt(head),
        position: row.get("position"),
        has_board: row.get::<i64, _>("has_board") != 0,
        role: row.get("role"),
        child_count: row.get("child_count"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        shared: share_token.is_some(),
        share_token,
        sealed: row.get("sealed"),
        file,
        location: None,
    }
}

/// The spaces, in display order. Their child counts are what the switcher shows.
pub async fn spaces(
    _: Session,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<NodeSummary>>> {
    list_spaces(&state.db).await.map(Json)
}

async fn list_spaces(db: &SqlitePool) -> AppResult<Vec<NodeSummary>> {
    let rows = sqlx::query(&format!(
        "SELECT {SUMMARY_COLUMNS} FROM nodes n
         WHERE n.parent_id IS NULL AND n.kind = 'space'
         ORDER BY n.position, n.name"
    ))
    .fetch_all(db)
    .await?;

    Ok(rows.iter().map(|row| row_to_summary(row, "")).collect())
}

/// How many documents each space shows in the overview.
const OVERVIEW_RECENT: i64 = 4;

#[derive(Serialize)]
pub struct SpaceOverview {
    pub space: NodeSummary,
    pub recent: Vec<NodeSummary>,
}

/// Every space with the documents last written in it, from any depth: what
/// the overview shows, the way the desktop's overview shows each workspace
/// with its windows.
pub async fn overview(
    _: Session,
    State(state): State<AppState>,
) -> AppResult<Json<Vec<SpaceOverview>>> {
    let mut out = Vec::new();
    for space in list_spaces(&state.db).await? {
        let recent = recent_documents(&state.db, space.id, OVERVIEW_RECENT).await?;
        out.push(SpaceOverview { space, recent });
    }
    Ok(Json(out))
}

/// The `limit` most recently edited documents anywhere under `root`.
pub async fn recent_documents(
    db: &SqlitePool,
    root: i64,
    limit: i64,
) -> AppResult<Vec<NodeSummary>> {
    let rows = sqlx::query(&format!(
        "WITH RECURSIVE sub (id) AS (
             SELECT id FROM nodes WHERE id = ?1
             UNION ALL
             SELECT n.id FROM nodes n JOIN sub ON n.parent_id = sub.id
         )
         SELECT {SUMMARY_COLUMNS}, substr(n.body, 1, {HEAD_CHARS}) AS head
         FROM nodes n
         WHERE n.kind IN ('document', 'file') AND n.id IN (SELECT id FROM sub)
         ORDER BY n.updated_at DESC, n.id DESC
         LIMIT ?2"
    ))
    .bind(root)
    .bind(limit)
    .fetch_all(db)
    .await?;

    Ok(rows
        .iter()
        .map(|row| row_to_summary(row, &row.try_get::<String, _>("head").unwrap_or_default()))
        .collect())
}

pub async fn create_space(
    _: Session,
    State(state): State<AppState>,
    Json(input): Json<SpaceInput>,
) -> AppResult<Json<Node>> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("a space needs a name".into()));
    }

    let id = insert_space(&state.db, name, input.has_board.unwrap_or(true)).await?;
    state.backup.signal();
    load(&state, id).await.map(Json)
}

/// A new space, last in the switcher. Shared by the API and by `mkdir` at the
/// root of the SFTP mount, which is why it takes a pool and not the state.
pub async fn insert_space(db: &SqlitePool, name: &str, has_board: bool) -> AppResult<i64> {
    let mut tx = crate::db::begin_write(db).await?;
    let slug = unique_slug(&mut tx, None, &slugify(name), None).await?;
    let now = crate::now();
    let position: i64 = sqlx::query(
        "SELECT COALESCE(MAX(position), -1) + 1 AS next FROM nodes WHERE parent_id IS NULL",
    )
    .fetch_one(&mut *tx)
    .await?
    .get("next");

    let id: i64 = sqlx::query(
        "INSERT INTO nodes (parent_id, kind, name, slug, position, has_board, created_at, updated_at)
         VALUES (NULL, 'space', ?1, ?2, ?3, ?4, ?5, ?5)
         RETURNING id",
    )
    .bind(name)
    .bind(&slug)
    .bind(position)
    .bind(i64::from(has_board))
    .bind(now)
    .fetch_one(&mut *tx)
    .await?
    .get("id");

    tx.commit().await?;
    Ok(id)
}

pub async fn list(
    _: Session,
    State(state): State<AppState>,
    Query(params): Query<ListQuery>,
) -> AppResult<Json<Vec<NodeSummary>>> {
    let limit = params.limit.unwrap_or(500).clamp(1, 2000);
    let offset = params.offset.unwrap_or(0).max(0);
    let search = params.q.as_deref().map(str::trim).filter(|q| !q.is_empty());

    let searching = search.and_then(fts_query).is_some();
    let rows = match search.and_then(fts_query) {
        // A search reaches through the whole tree, optionally confined to one
        // space: the recursive term collects that space, or every space, and
        // everything under it — which leaves the trash out.
        Some(query) => {
            sqlx::query(&format!(
                "WITH RECURSIVE sub (id) AS (
                     SELECT id FROM nodes
                     WHERE id = ?4 OR (?4 IS NULL AND parent_id IS NULL AND kind = 'space')
                     UNION ALL
                     SELECT n.id FROM nodes n JOIN sub ON n.parent_id = sub.id
                 )
                 SELECT {SUMMARY_COLUMNS}, substr(n.body, 1, {HEAD_CHARS}) AS head
                 FROM nodes_fts
                 JOIN nodes n ON n.id = nodes_fts.rowid
                 WHERE nodes_fts MATCH ?1
                   AND n.kind <> 'space'
                   AND n.id IN (SELECT id FROM sub)
                 ORDER BY rank
                 LIMIT ?2 OFFSET ?3"
            ))
            .bind(query)
            .bind(limit)
            .bind(offset)
            .bind(params.space)
            .fetch_all(&state.db)
            .await?
        }
        // Browsing: the children of one node, folders before documents.
        None => {
            let parent = params.parent;
            sqlx::query(&format!(
                "SELECT {SUMMARY_COLUMNS}, substr(n.body, 1, {HEAD_CHARS}) AS head
                 FROM nodes n
                 WHERE n.parent_id IS ?1 AND n.role IS NOT 'trash'
                 ORDER BY CASE n.kind WHEN 'folder' THEN 0 ELSE 1 END,
                          n.position, n.created_at DESC, n.id DESC
                 LIMIT ?2 OFFSET ?3"
            ))
            .bind(parent)
            .bind(limit)
            .bind(offset)
            .fetch_all(&state.db)
            .await?
        }
    };

    let mut found: Vec<NodeSummary> = rows
        .iter()
        .map(|row| {
            let head: String = row.try_get("head").unwrap_or_default();
            row_to_summary(row, &head)
        })
        .collect();
    if searching {
        for node in &mut found {
            let path = path_of(&state.db, node.id).await?;
            let parents = &path[..path.len().saturating_sub(1)];
            node.location = Some(
                parents
                    .iter()
                    .map(|c| c.slug.as_str())
                    .collect::<Vec<_>>()
                    .join("/"),
            );
        }
    }
    Ok(Json(found))
}

/// A node in the trash reads as gone: whatever still points at it — a card,
/// a link, a tab left open — finds nothing, as it did before there was one.
pub async fn get(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<Node>> {
    if trash::in_trash(&mut *state.db.acquire().await?, id).await? {
        return Err(AppError::NotFound);
    }
    load(&state, id).await.map(Json)
}

#[derive(Deserialize)]
pub struct ResolveQuery {
    /// Slugs from a space downwards, `work/einarbeiten/01-was-ist-eebus`.
    pub path: String,
}

/// Resolve a slug path to the node it names. This is the other half of the
/// `/n/<space>/<slug>/...` addresses the client puts in the URL bar and the
/// importer writes into rewritten links: they stay readable because they are
/// resolved here rather than carrying ids around.
pub async fn resolve(
    _: Session,
    State(state): State<AppState>,
    Query(params): Query<ResolveQuery>,
) -> AppResult<Json<Node>> {
    let mut current: Option<i64> = None;
    for segment in params.path.split('/').filter(|s| !s.is_empty()) {
        // A path starts at a space, never at the trash beside them.
        let row = sqlx::query(
            "SELECT id FROM nodes
             WHERE parent_id IS ?1 AND slug = ?2 AND (?1 IS NOT NULL OR kind = 'space')",
        )
        .bind(current)
        .bind(segment)
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::NotFound)?;
        current = Some(row.get("id"));
    }
    load(&state, current.ok_or(AppError::NotFound)?)
        .await
        .map(Json)
}

pub async fn create(
    _: Session,
    State(state): State<AppState>,
    Json(input): Json<CreateInput>,
) -> AppResult<Json<Node>> {
    let kind = input.kind.as_deref().unwrap_or(DOCUMENT);
    if kind != DOCUMENT && kind != FOLDER {
        return Err(AppError::BadRequest(format!(
            "a node created here is a document or a folder, not a {kind}"
        )));
    }

    let mut tx = crate::db::begin_write(&state.db).await?;
    container(&mut tx, input.parent_id).await?;

    let now = crate::now();
    let created_at = input.created_at.unwrap_or(now);
    let slug = unique_slug(&mut tx, Some(input.parent_id), &slugify(&input.name), None).await?;

    let id: i64 = sqlx::query(
        "INSERT INTO nodes (parent_id, kind, name, slug, body, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         RETURNING id",
    )
    .bind(input.parent_id)
    .bind(kind)
    .bind(input.name.trim())
    .bind(&slug)
    .bind(&input.body)
    .bind(created_at)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?
    .get("id");

    media::link_to_node(&mut tx, id, &input.body).await?;
    tx.commit().await?;
    state.backup.signal();
    load(&state, id).await.map(Json)
}

pub async fn update(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<UpdateInput>,
) -> AppResult<Json<Node>> {
    // One transaction from the first read to the last write: the slug chosen
    // here is still free when it is written, and a file's name and its
    // download name never disagree.
    let mut tx = crate::db::begin_write(&state.db).await?;
    let row =
        sqlx::query("SELECT parent_id, kind, name, slug, updated_at FROM nodes WHERE id = ?1")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(AppError::NotFound)?;

    if input
        .base_updated_at
        .is_some_and(|base| row.get::<i64, _>("updated_at") > base)
    {
        return Err(AppError::Conflict(
            "changed somewhere else since it was opened — :e! takes that version, :w! writes over it".into(),
        ));
    }

    let parent_id: Option<i64> = row.get("parent_id");
    let kind: String = row.get("kind");
    let previous_name: String = row.get("name");
    let name = input.name.trim();

    // The slug follows the name, because a path nobody can read is no better
    // than an id. It only moves when the name does, so a link to an unrenamed
    // document keeps working.
    let slug = if name == previous_name {
        row.get::<String, _>("slug")
    } else {
        unique_slug(&mut tx, parent_id, &slugify(name), Some(id)).await?
    };

    if input.has_board.is_some() && kind != SPACE {
        return Err(AppError::BadRequest("only a space has a board".into()));
    }

    // A file has no text, only a name, and the name is also what a download
    // of it is called.
    let body = if kind == FILE {
        ""
    } else {
        input.body.as_str()
    };
    if kind == FILE {
        sqlx::query(
            "UPDATE media SET filename = ?1 WHERE id = (SELECT media_id FROM nodes WHERE id = ?2)",
        )
        .bind(name)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query(
        "UPDATE nodes
         SET name = ?1, body = ?2, slug = ?3, updated_at = ?4,
             created_at = COALESCE(?5, created_at),
             has_board = COALESCE(?6, has_board)
         WHERE id = ?7",
    )
    .bind(name)
    .bind(body)
    .bind(&slug)
    .bind(crate::now())
    .bind(input.created_at)
    .bind(input.has_board.map(i64::from))
    .bind(id)
    .execute(&mut *tx)
    .await?;

    if kind != FILE {
        media::link_to_node(&mut tx, id, body).await?;
    }
    tx.commit().await?;
    state.backup.signal();
    load(&state, id).await.map(Json)
}

/// A delete puts the node in the trash; one already in the trash is purged.
pub async fn remove(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<serde_json::Value>> {
    let purged = trash::in_trash(&mut *state.db.acquire().await?, id).await?;
    let removed = if purged {
        purge_subtree(&state.db, &state.config.uploads_dir(), id).await?
    } else {
        trash::trash_subtree(&state.db, id).await?
    };
    state.backup.signal();
    Ok(Json(
        serde_json::json!({ "ok": true, "removed": removed, "purged": purged }),
    ))
}

/// Delete a node for good, everything beneath it, and the files nothing
/// outside it embeds. What emptying the trash does, and what the mount does
/// when a save replaces one file with another. Returns how many nodes went.
pub async fn purge_subtree(
    db: &SqlitePool,
    uploads: &std::path::Path,
    id: i64,
) -> AppResult<usize> {
    let mut tx = crate::db::begin_write(db).await?;

    let role: Option<String> = sqlx::query_scalar("SELECT role FROM nodes WHERE id = ?1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AppError::NotFound)?;
    if role.as_deref() == Some(DIARY) {
        return Err(AppError::BadRequest(
            "the diary space stays — empty it instead, or rename it".into(),
        ));
    }
    if role.as_deref() == Some(trash::TRASH) {
        return Err(AppError::BadRequest(
            "the trash stays — :trash purge empties it".into(),
        ));
    }

    // Collected before the delete, which takes the node_media rows with it.
    // Everything under a folder goes with it, so the files to clean up are
    // those exclusive to the whole subtree.
    let subtree = descendants(&mut tx, id).await?;
    let orphans = media::exclusive_media(&mut tx, &subtree).await?;

    sqlx::query("DELETE FROM nodes WHERE id = ?1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    for media_id in &orphans {
        sqlx::query("DELETE FROM media WHERE id = ?1")
            .bind(media_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;

    for media_id in &orphans {
        media::remove_upload(uploads, media_id).await;
    }
    Ok(subtree.len())
}

/// Reparent a node, and optionally place it among its new siblings.
pub async fn move_node(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<MoveInput>,
) -> AppResult<Json<Node>> {
    move_nodes(&state, &[id], input.parent_id, input.position).await?;
    load(&state, id).await.map(Json)
}

/// Move several nodes into one folder: all of them, or — when one of them
/// cannot go there — none.
pub async fn move_many(
    _: Session,
    State(state): State<AppState>,
    Json(input): Json<MoveManyInput>,
) -> AppResult<Json<Moved>> {
    let mut seen = HashSet::new();
    let ids: Vec<i64> = input
        .ids
        .into_iter()
        .filter(|id| seen.insert(*id))
        .collect();
    if ids.is_empty() {
        return Err(AppError::BadRequest("there is nothing to move".into()));
    }
    if ids.len() > MOVE_AT_ONCE {
        return Err(AppError::BadRequest(format!(
            "{MOVE_AT_ONCE} nodes at most move at once"
        )));
    }
    move_nodes(&state, &ids, input.parent_id, None)
        .await
        .map(Json)
}

/// Put `ids` under `parent`, in that order. Every node is checked before any
/// moves; into a sealed folder they go sealed, or nothing goes.
pub async fn move_nodes(
    state: &AppState,
    ids: &[i64],
    parent: i64,
    position: Option<i64>,
) -> AppResult<Moved> {
    // Checked before sealing, so that nothing is sealed for a move that is
    // refused anyway, and again in the transaction that moves them.
    {
        let mut conn = state.db.acquire().await?;
        movable(&mut conn, ids, parent).await?;
    }
    for &id in ids {
        crate::routes::seal::enter(state, id, parent).await?;
    }

    let mut tx = crate::db::begin_write(&state.db).await?;
    let nodes = movable(&mut tx, ids, parent).await?;
    let mut done = Moved {
        moved: 0,
        unchanged: 0,
    };
    for (id, name, from) in nodes {
        if from == Some(parent) && position.is_none() {
            done.unchanged += 1;
            continue;
        }
        // The slug has to be free where it lands, not where it came from —
        // and it sees the ones moved in before it, so two `notes` stay apart.
        let slug = unique_slug(&mut tx, Some(parent), &slugify(&name), Some(id)).await?;
        sqlx::query(
            "UPDATE nodes
             SET parent_id = ?1, slug = ?2, position = COALESCE(?3, position), updated_at = ?4
             WHERE id = ?5",
        )
        .bind(parent)
        .bind(&slug)
        .bind(position)
        .bind(crate::now())
        .bind(id)
        .execute(&mut *tx)
        .await?;
        done.moved += 1;
    }
    tx.commit().await?;

    if done.moved > 0 {
        state.backup.signal();
    }
    Ok(done)
}

/// Refuse the move unless every one of `ids` may go into `parent`; the id,
/// name and present parent of each when they all may.
async fn movable(
    db: &mut SqliteConnection,
    ids: &[i64],
    parent: i64,
) -> AppResult<Vec<(i64, String, Option<i64>)>> {
    container(&mut *db, parent).await?;
    let mut nodes = Vec::with_capacity(ids.len());
    for &id in ids {
        if id == parent {
            return Err(AppError::BadRequest("a node cannot contain itself".into()));
        }
        let row = sqlx::query("SELECT kind, name, parent_id FROM nodes WHERE id = ?1")
            .bind(id)
            .fetch_optional(&mut *db)
            .await?
            .ok_or(AppError::NotFound)?;
        let kind: String = row.get("kind");
        let name: String = row.get("name");
        // A space is a root: under a folder it would drop out of the switcher
        // while its board and its cards carried on as if nothing had happened.
        if kind == SPACE {
            return Err(AppError::BadRequest(format!(
                "{name} is a space, and a space cannot be moved into anything"
            )));
        }
        // Moving a folder into its own subtree would cut that subtree loose
        // from every root, where nothing could reach it again.
        if descendants(&mut *db, id).await?.contains(&parent) {
            return Err(AppError::BadRequest(format!(
                "{name} cannot be moved into itself"
            )));
        }
        nodes.push((id, name, row.get("parent_id")));
    }
    Ok(nodes)
}

/// Refuse anything but a space or a folder as the place a node goes into —
/// and one in the trash, where whatever went in would go when it is purged.
pub async fn container(db: &mut SqliteConnection, id: i64) -> AppResult<()> {
    let kind: String = sqlx::query("SELECT kind FROM nodes WHERE id = ?1")
        .bind(id)
        .fetch_optional(&mut *db)
        .await?
        .ok_or(AppError::NotFound)?
        .get("kind");
    if !is_container(&kind) {
        return Err(AppError::BadRequest(format!(
            "a {kind} cannot contain other nodes"
        )));
    }
    if trash::in_trash(&mut *db, id).await? {
        return Err(AppError::BadRequest(
            "that folder is in the trash — :trash brings it back".into(),
        ));
    }
    Ok(())
}

/// A node and everything beneath it, the node itself first.
pub async fn descendants(db: &mut SqliteConnection, id: i64) -> AppResult<Vec<i64>> {
    let rows = sqlx::query(
        "WITH RECURSIVE sub (id) AS (
             SELECT id FROM nodes WHERE id = ?1
             UNION ALL
             SELECT n.id FROM nodes n JOIN sub ON n.parent_id = sub.id
         )
         SELECT id FROM sub",
    )
    .bind(id)
    .fetch_all(&mut *db)
    .await?;

    Ok(rows.iter().map(|row| row.get("id")).collect())
}

/// The ancestors of a node, the space first and the node itself last.
pub async fn path_of(db: &SqlitePool, id: i64) -> AppResult<Vec<Crumb>> {
    let rows = sqlx::query(
        "WITH RECURSIVE up (id, parent_id, kind, name, slug, depth) AS (
             SELECT id, parent_id, kind, name, slug, 0 FROM nodes WHERE id = ?1
             UNION ALL
             SELECT n.id, n.parent_id, n.kind, n.name, n.slug, up.depth + 1
             FROM nodes n JOIN up ON n.id = up.parent_id
         )
         SELECT id, kind, name, slug FROM up ORDER BY depth DESC",
    )
    .bind(id)
    .fetch_all(db)
    .await?;

    Ok(rows
        .iter()
        .map(|row| Crumb {
            id: row.get("id"),
            kind: row.get("kind"),
            name: row.get("name"),
            slug: row.get("slug"),
        })
        .collect())
}

pub async fn load(state: &AppState, id: i64) -> AppResult<Node> {
    let row = sqlx::query(&format!(
        "SELECT {SUMMARY_COLUMNS}, n.body FROM nodes n WHERE n.id = ?1"
    ))
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;

    let body: String = row.get("body");
    let summary = row_to_summary(&row, &body.chars().take(HEAD_CHARS).collect::<String>());

    Ok(Node {
        id: summary.id,
        parent_id: summary.parent_id,
        kind: summary.kind,
        name: summary.name,
        slug: summary.slug,
        body,
        excerpt: summary.excerpt,
        position: summary.position,
        has_board: summary.has_board,
        role: summary.role,
        child_count: summary.child_count,
        created_at: summary.created_at,
        updated_at: summary.updated_at,
        shared: summary.shared,
        share_token: summary.share_token,
        sealed: summary.sealed,
        file: summary.file,
        path: path_of(&state.db, id).await?,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{
        excerpt, fts_query, next_slug, purge_subtree, recent_documents, slugify, EXCERPT_CHARS,
        HEAD_CHARS,
    };
    use crate::error::AppError;

    /// A delete takes the subtree and the files only it embedded — from the
    /// table and from the disk — and leaves what something else still embeds.
    /// The diary, found by its role whatever it is called, is not deleted.
    #[tokio::test]
    async fn a_delete_takes_what_only_it_held() {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&db).await.unwrap();
        let dir = tempfile::tempdir().unwrap();

        let diary: i64 = sqlx::query_scalar("SELECT id FROM nodes WHERE role = 'diary'")
            .fetch_one(&db)
            .await
            .unwrap();
        sqlx::query("UPDATE nodes SET name = 'journal', slug = 'journal' WHERE id = ?1")
            .bind(diary)
            .execute(&db)
            .await
            .unwrap();
        assert!(matches!(
            purge_subtree(&db, dir.path(), diary).await,
            Err(AppError::BadRequest(_))
        ));

        let node = |parent: i64, kind: &'static str, slug: &'static str| {
            let db = db.clone();
            async move {
                sqlx::query_scalar::<_, i64>(
                    "INSERT INTO nodes (parent_id, kind, name, slug, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?3, 0, 0) RETURNING id",
                )
                .bind(parent)
                .bind(kind)
                .bind(slug)
                .fetch_one(&db)
                .await
                .unwrap()
            }
        };
        let folder = node(diary, "folder", "trip").await;
        let inside = node(folder, "document", "day-1").await;
        let outside = node(diary, "document", "elsewhere").await;

        let only = "11111111-1111-4111-8111-111111111111";
        let shared = "22222222-2222-4222-8222-222222222222";
        for (id, nodes) in [(only, vec![inside]), (shared, vec![inside, outside])] {
            sqlx::query("INSERT INTO media (id, filename, mime, size, created_at) VALUES (?1, 'x', 'image/png', 1, 0)")
                .bind(id)
                .execute(&db)
                .await
                .unwrap();
            std::fs::write(dir.path().join(id), b"x").unwrap();
            for node in nodes {
                sqlx::query("INSERT INTO node_media (node_id, media_id) VALUES (?1, ?2)")
                    .bind(node)
                    .bind(id)
                    .execute(&db)
                    .await
                    .unwrap();
            }
        }

        assert_eq!(purge_subtree(&db, dir.path(), folder).await.unwrap(), 2);
        let media: Vec<String> = sqlx::query_scalar("SELECT id FROM media")
            .fetch_all(&db)
            .await
            .unwrap();
        assert_eq!(media, [shared]);
        assert!(!dir.path().join(only).exists());
        assert!(dir.path().join(shared).exists());
        assert!(matches!(
            purge_subtree(&db, dir.path(), folder).await,
            Err(AppError::NotFound)
        ));
    }

    /// The overview reaches into folders, newest edit first, and stops at the
    /// limit — and never lists a folder as if it were a document.
    #[tokio::test]
    async fn recent_documents_come_from_any_depth() {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("an in-memory database");
        sqlx::migrate!("./migrations")
            .run(&db)
            .await
            .expect("migrations apply");

        let insert = |parent: i64, kind: &'static str, slug: &'static str, at: i64| {
            let db = db.clone();
            async move {
                sqlx::query_scalar::<_, i64>(
                    "INSERT INTO nodes (parent_id, kind, name, slug, body, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?3, '', ?4, ?4) RETURNING id",
                )
                .bind(parent)
                .bind(kind)
                .bind(slug)
                .bind(at)
                .fetch_one(&db)
                .await
                .expect("insert")
            }
        };
        let space: i64 = sqlx::query_scalar("SELECT id FROM nodes WHERE slug = 'uni'")
            .fetch_one(&db)
            .await
            .expect("the seeded space");
        let folder = insert(space, "folder", "deep", 1).await;
        let inner = insert(folder, "folder", "deeper", 2).await;
        insert(space, "document", "a", 10).await;
        insert(folder, "document", "b", 30).await;
        insert(inner, "document", "c", 20).await;

        let slugs: Vec<String> = recent_documents(&db, space, 2)
            .await
            .expect("query")
            .into_iter()
            .map(|n| n.slug)
            .collect();
        assert_eq!(slugs, ["b", "c"]);
    }

    /// 0010 widens a CHECK by editing the stored schema. Every connection —
    /// not just the one that ran it — has to accept a file node afterwards,
    /// the database has to stay intact, and a file an old import hung on a
    /// folder has to come out as a file node in that folder.
    #[tokio::test]
    async fn files_join_the_tree_on_a_database_that_predates_them() {
        use sqlx::{sqlite::SqliteConnectOptions, Connection, Executor};

        let dir = tempfile::tempdir().expect("tempdir");
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(dir.path().join("t.db"))
                    .create_if_missing(true)
                    .foreign_keys(true),
            )
            .await
            .expect("a database");

        let mut before = sqlx::migrate!("./migrations");
        before.migrations = before
            .migrations
            .iter()
            .filter(|m| m.version < 10)
            .cloned()
            .collect();
        before.run(&db).await.expect("migrations up to 0009");

        let folder: i64 = sqlx::query_scalar(
            "INSERT INTO nodes (parent_id, kind, name, slug, created_at, updated_at)
             VALUES ((SELECT id FROM nodes WHERE slug = 'uni'), 'folder', 'algo', 'algo', 1, 1)
             RETURNING id",
        )
        .fetch_one(&db)
        .await
        .expect("a folder");
        db.execute(
            "INSERT INTO media (id, filename, mime, size, created_at)
             VALUES ('0a1b2c3d-0000-0000-0000-000000000000', 'slides.pdf', 'application/pdf', 3, 1)",
        )
        .await
        .expect("media");
        sqlx::query("INSERT INTO node_media (node_id, media_id) VALUES (?1, '0a1b2c3d-0000-0000-0000-000000000000')")
            .bind(folder)
            .execute(&db)
            .await
            .expect("hung on the folder");

        // Opened before the migration, so it holds the schema as it was.
        let mut stale = db.acquire().await.expect("a second connection");
        stale.ping().await.expect("ping");

        sqlx::migrate!("./migrations")
            .run(&db)
            .await
            .expect("0010 applies");

        let (name, slug, owned): (String, String, i64) = sqlx::query_as(
            "SELECT n.name, n.slug, (SELECT COUNT(*) FROM node_media nm WHERE nm.node_id = n.id)
             FROM nodes n WHERE n.kind = 'file' AND n.parent_id = ?1",
        )
        .bind(folder)
        .fetch_one(&db)
        .await
        .expect("the adopted file");
        assert_eq!(
            (name.as_str(), slug.as_str(), owned),
            ("slides.pdf", "file-0a1b2c3d", 1)
        );
        let on_folder: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM node_media WHERE node_id = ?1")
                .bind(folder)
                .fetch_one(&db)
                .await
                .expect("count");
        assert_eq!(on_folder, 0);

        db.execute("INSERT INTO media (id, filename, mime, size, created_at) VALUES ('m2', 'b.zip', 'application/octet-stream', 1, 1)")
            .await
            .expect("media");
        sqlx::query(
            "INSERT INTO nodes (parent_id, kind, name, slug, created_at, updated_at, media_id)
             VALUES (?1, 'file', 'b.zip', 'b-zip', 1, 1, 'm2')",
        )
        .bind(folder)
        .execute(&mut *stale)
        .await
        .expect("the stale connection accepts a file node");

        // A file without bytes, or bytes on a document, is refused.
        assert!(sqlx::query(
            "INSERT INTO nodes (parent_id, kind, name, slug, created_at, updated_at)
             VALUES (?1, 'file', 'x', 'x', 1, 1)",
        )
        .bind(folder)
        .execute(&db)
        .await
        .is_err());

        let check: String = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_one(&db)
            .await
            .expect("check");
        assert_eq!(check, "ok");
        let dangling = sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&db)
            .await
            .expect("fk check");
        assert!(dangling.is_empty());
    }

    #[test]
    fn strips_markdown_furniture() {
        let body = "# Title\n\n![pic](/api/media/x)\n\n> a quote\n- a point";
        assert_eq!(excerpt(body), "Title a quote a point");
    }

    #[test]
    fn marks_a_body_that_was_cut_short() {
        assert!(!excerpt("short").ends_with('…'));
        assert!(excerpt(&"word ".repeat(60)).ends_with('…'));
        // A head at the read limit is truncated even when it flattens small.
        assert!(excerpt(&"![x](/api/media/y)\n".repeat(HEAD_CHARS / 18)).ends_with('…'));
    }

    #[test]
    fn excerpt_never_exceeds_its_budget() {
        let out = excerpt(&"a".repeat(HEAD_CHARS));
        assert_eq!(out.chars().count(), EXCERPT_CHARS + 1);
    }

    #[test]
    fn fts_query_discards_operators() {
        // Anything that could be read as FTS syntax must not survive as syntax.
        assert_eq!(fts_query("hello"), Some("\"hello\"*".into()));
        assert_eq!(
            fts_query("a OR b"),
            Some("\"a\"* AND \"OR\"* AND \"b\"*".into())
        );
        assert_eq!(
            fts_query("foo\"NEAR/2\"bar"),
            Some("\"foo\"* AND \"NEAR\"* AND \"2\"* AND \"bar\"*".into())
        );
        assert_eq!(fts_query("  "), None);
        assert_eq!(fts_query("***"), None);
    }

    #[test]
    fn slugs_are_url_safe() {
        assert_eq!(slugify("01 — Was ist EEBUS?"), "01-was-ist-eebus");
        assert_eq!(slugify("  Trailing and   inner  "), "trailing-and-inner");
        assert_eq!(slugify("Übergrößen"), "uebergroessen");
        assert_eq!(slugify("README.md"), "readme-md");
    }

    #[test]
    fn a_nameless_node_still_gets_a_slug() {
        assert_eq!(slugify(""), "untitled");
        assert_eq!(slugify("***"), "untitled");
        assert_eq!(slugify("   "), "untitled");
    }

    #[test]
    fn slugs_stay_short_and_never_end_in_a_dash() {
        let slug = slugify(&"word ".repeat(40));
        assert!(slug.len() <= 80, "{slug}");
        assert!(!slug.ends_with('-'), "{slug}");
    }

    #[test]
    fn a_taken_slug_gets_a_number() {
        let taken: BTreeSet<String> = ["readme", "readme-2"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(next_slug(&taken, "readme").unwrap(), "readme-3");
        assert_eq!(next_slug(&taken, "notes").unwrap(), "notes");
        assert_eq!(next_slug(&BTreeSet::new(), "readme").unwrap(), "readme");
    }
}
