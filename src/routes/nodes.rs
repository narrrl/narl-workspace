use std::collections::{BTreeMap, BTreeSet, HashSet};

use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqliteConnection, SqlitePool};

use crate::{
    auth::Session,
    error::{AppError, AppResult},
    routes::{boards, diary, links, media, revisions, trash},
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

/// Whether a node of kind `parent` may hold one of kind `child`: a space or a
/// folder anything but a space, and a document its own files — the picture
/// pasted into it, the PDF it is about — so they move, seal, go to the trash
/// and come back with it.
pub fn holds(parent: &str, child: &str) -> bool {
    match parent {
        SPACE | FOLDER => child != SPACE,
        DOCUMENT => child == FILE,
        _ => false,
    }
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
    /// Search results only: a file found by what it says rather than by its
    /// name, with the words around the match as its excerpt — each word that
    /// matched between U+0002 and U+0003, which no text file has a use for,
    /// for the client to light.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub in_text: bool,
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
    /// For a document, whether the files it is given are sealed: the folder
    /// it is in decides.
    pub sealed: bool,
    pub file: Option<FileInfo>,
    /// A document's own files, oldest first: what was pasted into it, and
    /// what was put beside it.
    pub files: Vec<NodeSummary>,
    /// This node's ancestors, the space first and the node itself last — the
    /// breadcrumb, and everything needed to build its /n/... path.
    pub path: Vec<Crumb>,
    /// How many documents link here.
    pub backlinks: i64,
    /// The cards about it: the ones whose notes it is, and the ones whose
    /// summary links here.
    pub cards: Vec<boards::AgendaCard>,
    /// Where each `[[title]]` in the body leads, as an /n/... path. A title
    /// that leads nowhere is not in it.
    pub wikilinks: BTreeMap<String, String>,
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
    /// A document to start from, its placeholders filled in for this one. A
    /// new entry in the diary starts from `templates/day` without being asked.
    #[serde(default)]
    pub template_id: Option<i64>,
}

#[derive(Deserialize)]
pub struct UpdateInput {
    /// Left out, the name stays as it is: a write changes only what it sends,
    /// so a rename cannot empty a document and a save cannot rename one.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
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
    /// The text sent is a revision brought back: what it replaces is kept
    /// as a revision of its own, however recently the last one was.
    #[serde(default)]
    pub restore: bool,
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
pub(crate) const HEAD_CHARS: usize = 1024;
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
pub(crate) fn excerpt(head: &str) -> String {
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
pub(crate) fn fts_query(raw: &str) -> Option<String> {
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
/// The longest name a node may have, in characters. A file is called by its
/// name on the SFTP mount, where most filesystems stop at 255 bytes, and a
/// name that long helps nobody in a list or a crumb either.
pub const NAME_MAX: usize = 200;

pub fn check_name(name: &str) -> AppResult<()> {
    if name.chars().count() > NAME_MAX {
        return Err(AppError::BadRequest(format!(
            "a name is at most {NAME_MAX} characters"
        )));
    }
    Ok(())
}

/// Spaces are a short, numbered list switched by name: two that read alike
/// are only confusing, and `:space work` could not say which it meant. Taken
/// means the same name in any case, or the same slug.
async fn space_name_free(
    db: &mut SqliteConnection,
    name: &str,
    exclude: Option<i64>,
) -> AppResult<()> {
    let taken: Option<String> = sqlx::query_scalar(
        "SELECT name FROM nodes
         WHERE parent_id IS NULL AND kind = 'space' AND id IS NOT ?3
           AND (lower(name) = lower(?1) OR slug = ?2)",
    )
    .bind(name)
    .bind(slugify(name))
    .bind(exclude)
    .fetch_optional(&mut *db)
    .await?;
    match taken {
        Some(taken) => Err(AppError::BadRequest(format!(
            "there is a space called {taken} already"
        ))),
        None => Ok(()),
    }
}

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
        in_text: false,
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
    check_name(name)?;

    let id = insert_space(&state.db, name, input.has_board.unwrap_or(true)).await?;
    state.backup.signal();
    load(&state, id).await.map(Json)
}

/// A new space, last in the switcher. Shared by the API and by `mkdir` at the
/// root of the SFTP mount, which is why it takes a pool and not the state.
pub async fn insert_space(db: &SqlitePool, name: &str, has_board: bool) -> AppResult<i64> {
    let mut tx = crate::db::begin_write(db).await?;
    space_name_free(&mut tx, name, None).await?;
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

    let query = search.and_then(fts_query);
    // Something asked for that is all punctuation finds nothing; it is not a
    // request to browse the spaces.
    if search.is_some() && query.is_none() {
        return Ok(Json(Vec::new()));
    }
    let searching = query.is_some();
    let rows = match query {
        // A search reaches through the whole tree, optionally confined to one
        // space: the recursive term collects that space, or every space, and
        // everything under it — which leaves the trash out. It asks both
        // indexes, names and documents in one and the text of files in the
        // other, and a node found in both is listed once, at its better rank.
        // The two ranks are BM25 over different tables, so their order is
        // approximate, which is good enough for a short list.
        Some(query) => {
            sqlx::query(&format!(
                "WITH RECURSIVE sub (id) AS (
                     SELECT id FROM nodes
                     WHERE id = ?4 OR (?4 IS NULL AND parent_id IS NULL AND kind = 'space')
                     UNION ALL
                     SELECT n.id FROM nodes n JOIN sub ON n.parent_id = sub.id
                 ),
                 hits (id, rank, snippet) AS (
                     SELECT rowid, rank, NULL FROM nodes_fts WHERE nodes_fts MATCH ?1
                     UNION ALL
                     SELECT rowid, rank, snippet(files_fts, 0, char(2), char(3), '…', 16)
                     FROM files_fts WHERE files_fts MATCH ?1
                 ),
                 best AS (
                     SELECT id, MIN(rank) AS rank, MAX(snippet) AS snippet
                     FROM hits GROUP BY id
                 )
                 SELECT {SUMMARY_COLUMNS},
                        COALESCE(best.snippet, substr(n.body, 1, {HEAD_CHARS})) AS head,
                        best.snippet IS NOT NULL AS in_text
                 FROM best
                 JOIN nodes n ON n.id = best.id
                 WHERE n.kind <> 'space'
                   AND n.id IN (SELECT id FROM sub)
                 ORDER BY best.rank
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
            NodeSummary {
                in_text: row.try_get("in_text").unwrap_or(false),
                ..row_to_summary(row, &head)
            }
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
    // A path starts at a space, never at the trash beside them.
    let found = links::resolve_path(&mut *state.db.acquire().await?, &params.path).await?;
    load(&state, found.ok_or(AppError::NotFound)?)
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

    check_name(input.name.trim())?;
    let mut tx = crate::db::begin_write(&state.db).await?;
    container(&mut tx, input.parent_id).await?;

    let now = crate::now();
    let created_at = input.created_at.unwrap_or(now);
    let slug = unique_slug(&mut tx, Some(input.parent_id), &slugify(&input.name), None).await?;

    let tz = state.config.mail.timezone;
    let name = input.name.trim();
    let body = match input.template_id {
        Some(_) if kind != DOCUMENT => {
            return Err(AppError::BadRequest(
                "only a document starts from a template".into(),
            ));
        }
        Some(_) if !input.body.is_empty() => {
            return Err(AppError::BadRequest(
                "a document starts from a template or from a body, not both".into(),
            ));
        }
        Some(template) => diary::from_template(&mut tx, tz, template, created_at, name).await?,
        None if kind == DOCUMENT
            && input.body.is_empty()
            && diary::is_diary(&mut tx, input.parent_id).await? =>
        {
            diary::day_template(&mut tx, tz, input.parent_id, created_at)
                .await?
                .unwrap_or_default()
        }
        None => input.body,
    };

    let id: i64 = sqlx::query(
        "INSERT INTO nodes (parent_id, kind, name, slug, body, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         RETURNING id",
    )
    .bind(input.parent_id)
    .bind(kind)
    .bind(name)
    .bind(&slug)
    .bind(&body)
    .bind(created_at)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?
    .get("id");

    media::link_to_node(&mut tx, id, &body).await?;
    links::index(&mut tx, id, &body).await?;
    // A `[[link]]` written before the document it names now finds it.
    links::follow(&mut tx, id).await?;
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
    let row = sqlx::query(
        "SELECT parent_id, kind, name, slug, body, updated_at FROM nodes WHERE id = ?1",
    )
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
    let previous_body: String = row.get("body");
    let name = input
        .name
        .as_deref()
        .map_or(previous_name.as_str(), str::trim);
    check_name(name)?;
    if kind == SPACE && name != previous_name {
        space_name_free(&mut tx, name, Some(id)).await?;
    }

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
        input.body.as_deref().unwrap_or(&previous_body)
    };
    let reason = if input.restore {
        revisions::Reason::Restore
    } else if input.base_updated_at.is_none() {
        revisions::Reason::Force
    } else {
        revisions::Reason::Edit
    };
    revisions::keep(&mut tx, id, body, reason).await?;
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
        links::index(&mut tx, id, body).await?;
    }
    // A new name is a new path, for this and everything under it, and a new
    // title: the links written with the old ones are rewritten to find it.
    if name != previous_name {
        links::follow(&mut tx, id).await?;
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
    let mut moved = Vec::with_capacity(nodes.len());
    for (id, name, from) in nodes {
        if from == Some(parent) && position.is_none() {
            done.unchanged += 1;
            continue;
        }
        moved.push(id);
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
    // Once all of them are where they go, so a link between two of them is
    // rewritten to where both now are.
    for id in moved {
        links::follow(&mut tx, id).await?;
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
        place_for(&mut *db, parent, &kind).await?;
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
    place_for(db, id, FOLDER).await
}

/// Refuse `id` as the place a node of `kind` goes into: one that cannot hold
/// it, by [`holds`], or one in the trash.
pub async fn place_for(db: &mut SqliteConnection, id: i64, kind: &str) -> AppResult<()> {
    let parent: String = sqlx::query("SELECT kind FROM nodes WHERE id = ?1")
        .bind(id)
        .fetch_optional(&mut *db)
        .await?
        .ok_or(AppError::NotFound)?
        .get("kind");
    if !holds(&parent, kind) {
        return Err(AppError::BadRequest(if parent == DOCUMENT {
            "a document holds only its own files".into()
        } else {
            format!("a {parent} cannot contain other nodes")
        }));
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
    let (backlinks, wikilinks, cards) = {
        let mut conn = state.db.acquire().await?;
        (
            links::linked_from(&mut conn, id).await?,
            links::wikilinks(&mut conn, id, &body).await?,
            boards::cards_of(&mut conn, id).await?,
        )
    };
    let (files, sealed) = if summary.kind == DOCUMENT {
        let files = sqlx::query(&format!(
            "SELECT {SUMMARY_COLUMNS} FROM nodes n
             WHERE n.parent_id = ?1 AND n.kind = 'file' ORDER BY n.created_at, n.id"
        ))
        .bind(id)
        .fetch_all(&state.db)
        .await?;
        (
            files.iter().map(|row| row_to_summary(row, "")).collect(),
            crate::routes::seal::sealed(state, id).await?,
        )
    } else {
        (Vec::new(), summary.sealed)
    };

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
        sealed,
        file: summary.file,
        files,
        path: path_of(&state.db, id).await?,
        backlinks,
        cards,
        wikilinks,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{
        excerpt, fts_query, load, move_nodes, next_slug, place_for, purge_subtree,
        recent_documents, slugify, DOCUMENT, EXCERPT_CHARS, FILE, FOLDER, HEAD_CHARS,
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

    /// 0029: a file a document embeds, and that was in the tree nowhere,
    /// becomes that document's file — a living document's before one in the
    /// trash — under the same media id. A file already in the tree stays
    /// where it is, and one nothing embeds is left to the sweep.
    #[tokio::test]
    async fn embedded_files_become_the_files_of_their_document() {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("an in-memory database");
        let mut before = sqlx::migrate!("./migrations");
        before.migrations = before
            .migrations
            .iter()
            .filter(|m| m.version < 29)
            .cloned()
            .collect();
        before.run(&db).await.expect("migrations up to 0028");

        let node = |parent: &'static str, kind: &'static str, slug: &'static str| {
            let db = db.clone();
            async move {
                sqlx::query_scalar::<_, i64>(
                    "INSERT INTO nodes (parent_id, kind, name, slug, body, created_at, updated_at)
                     VALUES ((SELECT id FROM nodes WHERE slug = ?1), ?2, ?3, ?3, '', 1, 1)
                     RETURNING id",
                )
                .bind(parent)
                .bind(kind)
                .bind(slug)
                .fetch_one(&db)
                .await
                .expect("insert")
            }
        };
        let gone = node(".trash", "document", "gone").await;
        let plan = node("uni", "document", "plan").await;
        for (id, name) in [("a0", "both.png"), ("b0", "gone.png"), ("c0", "loose.png")] {
            sqlx::query(
                "INSERT INTO media (id, filename, mime, size, created_at)
                 VALUES (?1 || '000000-0000-0000-0000-000000000000', ?2, 'image/png', 1, 1)",
            )
            .bind(id)
            .bind(name)
            .execute(&db)
            .await
            .expect("media");
        }
        for (doc, media) in [(gone, "a0"), (plan, "a0"), (gone, "b0")] {
            sqlx::query(
                "INSERT INTO node_media (node_id, media_id)
                 VALUES (?1, ?2 || '000000-0000-0000-0000-000000000000')",
            )
            .bind(doc)
            .bind(media)
            .execute(&db)
            .await
            .expect("embedded");
        }

        sqlx::migrate!("./migrations")
            .run(&db)
            .await
            .expect("0029 applies");

        let files: Vec<(String, i64, String)> = sqlx::query_as(
            "SELECT n.name, n.parent_id, n.slug FROM nodes n WHERE n.kind = 'file' ORDER BY n.name",
        )
        .fetch_all(&db)
        .await
        .expect("files");
        assert_eq!(
            files,
            [
                ("both.png".into(), plan, "file-a0000000".into()),
                ("gone.png".into(), gone, "file-b0000000".into()),
            ]
        );
        let owned: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM node_media nm JOIN nodes f ON f.id = nm.node_id
             WHERE f.kind = 'file' AND nm.media_id = f.media_id",
        )
        .fetch_one(&db)
        .await
        .expect("count");
        assert_eq!(owned, 2, "each file owns its bytes");
    }

    /// A document holds its own files and nothing else; a folder holds both.
    #[tokio::test]
    async fn a_document_holds_files_only() {
        let state = crate::routes::tests::state_in(".".into()).await;
        let mut conn = state.db.acquire().await.expect("a connection");
        let space: i64 = sqlx::query_scalar("SELECT id FROM nodes WHERE slug = 'uni'")
            .fetch_one(&mut *conn)
            .await
            .expect("the seeded space");
        let doc: i64 = sqlx::query_scalar(
            "INSERT INTO nodes (parent_id, kind, name, slug, body, created_at, updated_at)
             VALUES (?1, 'document', 'Plan', 'plan', '', 1, 1) RETURNING id",
        )
        .bind(space)
        .fetch_one(&mut *conn)
        .await
        .expect("a document");
        place_for(&mut conn, doc, FILE)
            .await
            .expect("a file in a document");
        place_for(&mut conn, space, FILE)
            .await
            .expect("a file in a space");
        for kind in [DOCUMENT, FOLDER] {
            assert!(place_for(&mut conn, doc, kind).await.is_err(), "{kind}");
        }
        drop(conn);

        crate::routes::media::insert_file(
            &state.db,
            doc,
            "f0000000-0000-0000-0000-000000000000",
            "a.png",
            "image/png",
            1,
            false,
        )
        .await
        .expect("a file of the document");
        let loaded = load(&state, doc).await.expect("load");
        assert_eq!(loaded.files.len(), 1);
        assert_eq!(loaded.files[0].name, "a.png");
        assert_eq!(loaded.child_count, 1);
        assert!(move_nodes(&state, &[doc], doc, None).await.is_err());
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

    /// A search of nothing but punctuation finds nothing, rather than
    /// answering with the spaces as if it had been asked to browse.
    #[tokio::test]
    async fn punctuation_finds_nothing() {
        use axum::extract::{Query, State};

        let state = crate::routes::tests::state_in(".".into()).await;
        let search = |q: &'static str| {
            let state = state.clone();
            async move {
                let query = serde_json::from_value(serde_json::json!({ "q": q })).unwrap();
                super::list(
                    crate::auth::Session { id: "test".into() },
                    State(state),
                    Query(query),
                )
                .await
                .unwrap()
                .0
            }
        };
        assert!(search("!!!").await.is_empty());
        assert!(search(" -- ").await.is_empty());
    }

    /// Two spaces may not read alike, in any case or by slug, and a name has
    /// a ceiling — for a new space, a rename, and a new document alike.
    #[tokio::test]
    async fn names_are_checked() {
        use axum::extract::{Path, State};

        let state = crate::routes::tests::state_in(".".into()).await;
        let session = || crate::auth::Session { id: "test".into() };
        let space = |name: &str| {
            let state = state.clone();
            let input = serde_json::from_value(serde_json::json!({ "name": name })).unwrap();
            async move { super::create_space(session(), State(state), axum::Json(input)).await }
        };
        assert!(matches!(space("Work").await, Err(AppError::BadRequest(_))));
        assert!(matches!(
            space(" work ").await,
            Err(AppError::BadRequest(_))
        ));
        let home = space("home").await.unwrap().0;

        let rename = |name: String| {
            let state = state.clone();
            let input = serde_json::from_value(serde_json::json!({ "name": name })).unwrap();
            async move { super::update(session(), State(state), Path(home.id), axum::Json(input)).await }
        };
        assert!(matches!(
            rename("uni".into()).await,
            Err(AppError::BadRequest(_))
        ));
        assert_eq!(rename("Home".into()).await.unwrap().0.name, "Home");
        assert!(matches!(
            rename("x".repeat(201)).await,
            Err(AppError::BadRequest(_))
        ));

        let long = serde_json::from_value(
            serde_json::json!({ "parent_id": home.id, "name": "y".repeat(201) }),
        )
        .unwrap();
        assert!(matches!(
            super::create(session(), State(state.clone()), axum::Json(long)).await,
            Err(AppError::BadRequest(_))
        ));
    }
}
