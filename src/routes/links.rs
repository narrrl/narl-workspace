//! Links between documents. A document links to another by the path the
//! address bar shows, `[the plan](/n/work/plan)`, or by its title,
//! `[[Plan]]` or `[[Plan|the plan]]`.
//!
//! What a document links to is kept in `links`, rebuilt whenever its text is
//! written. Read the other way round, that is what links to a document, and
//! the texts a rename or a move would break: those are rewritten, in the same
//! transaction, so a link keeps working however its target moves around.

use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::LazyLock,
};

use axum::{
    extract::{Path, State},
    Json,
};
use regex::Regex;
use serde::Serialize;
use sqlx::{Row, SqliteConnection, SqlitePool};

use crate::{
    auth::Session,
    error::{AppError, AppResult},
    routes::{nodes, revisions, trash},
    state::AppState,
};

/// A markdown link to a path in the workspace, inline or by reference:
/// `](/n/work/plan)` or `[plan]: /n/work/plan`. A path anywhere else in the
/// prose is text about one, not a link.
static PATH_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)(?:\]\(\s*<?|^[ \t]{0,3}\[[^\]\n]+\]:[ \t]*<?)/n/([a-z0-9-]+(?:/[a-z0-9-]+)*)")
        .expect("static regex is valid")
});

/// `[[title]]`, or `[[title|what the link reads]]`.
static WIKILINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[\[([^\[\]|\n]+)(\|[^\[\]\n]*)?\]\]").expect("static regex is valid")
});

/// How a link names what it links to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Kind {
    /// By its path, `work/plan`.
    Path,
    /// By its title, `Plan`.
    Wiki,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Path => "path",
            Kind::Wiki => "wiki",
        }
    }

    fn parse(kind: &str) -> Self {
        if kind == "wiki" {
            Kind::Wiki
        } else {
            Kind::Path
        }
    }
}

/// The path links in `body`, each as the span of the path after `/n/`.
fn paths(body: &str) -> impl Iterator<Item = regex::Match<'_>> {
    PATH_LINK.captures_iter(body).filter_map(|caps| {
        let path = caps.get(1)?;
        // The path has to end where the link does: `/n/work/Plan` or
        // `/n/work/plan.md` is not a link to `work`.
        let rest = &body[path.end()..];
        let rest = rest.strip_prefix('/').unwrap_or(rest);
        let ends = rest
            .chars()
            .next()
            .is_none_or(|c| c.is_whitespace() || matches!(c, ')' | '#' | '?' | '>' | '"' | '\''));
        ends.then_some(path)
    })
}

/// What `body` links to, as it is written.
fn written(body: &str) -> BTreeSet<(Kind, String)> {
    let mut found: BTreeSet<(Kind, String)> = paths(body)
        .map(|path| (Kind::Path, path.as_str().to_string()))
        .collect();
    for caps in WIKILINK.captures_iter(body) {
        let title = caps[1].trim();
        if !title.is_empty() {
            found.insert((Kind::Wiki, title.to_string()));
        }
    }
    found
}

/// `body` with the links named in `to` pointed elsewhere: each target written
/// there becomes the one it maps to. A label stays as it was.
fn rewrite(body: &str, to: &HashMap<(Kind, String), String>) -> String {
    let mut out = String::with_capacity(body.len());
    let mut last = 0;
    for path in paths(body) {
        if let Some(new) = to.get(&(Kind::Path, path.as_str().to_string())) {
            out.push_str(&body[last..path.start()]);
            out.push_str(new);
            last = path.end();
        }
    }
    out.push_str(&body[last..]);

    WIKILINK
        .replace_all(&out, |caps: &regex::Captures| {
            match to.get(&(Kind::Wiki, caps[1].trim().to_string())) {
                Some(new) => format!("[[{new}{}]]", caps.get(2).map_or("", |m| m.as_str())),
                None => caps[0].to_string(),
            }
        })
        .into_owned()
}

/// The node a slug path names, from a space down, as an `/n/` address
/// names it. Never one in the trash, which is no space.
pub async fn resolve_path(db: &mut SqliteConnection, path: &str) -> AppResult<Option<i64>> {
    let mut current: Option<i64> = None;
    for segment in path.split('/').filter(|s| !s.is_empty()) {
        let found: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM nodes
             WHERE parent_id IS ?1 AND slug = ?2 AND (?1 IS NOT NULL OR kind = 'space')",
        )
        .bind(current)
        .bind(segment)
        .fetch_optional(&mut *db)
        .await?;
        let Some(id) = found else {
            return Ok(None);
        };
        current = Some(id);
    }
    Ok(current)
}

/// The space a node is in, or the node itself when it is one. `None` in the
/// trash.
async fn space_of(db: &mut SqliteConnection, id: i64) -> AppResult<Option<i64>> {
    Ok(sqlx::query_scalar(
        "WITH RECURSIVE up (id, parent_id) AS (
             SELECT id, parent_id FROM nodes WHERE id = ?1
             UNION ALL
             SELECT n.id, n.parent_id FROM nodes n JOIN up ON n.id = up.parent_id
         )
         SELECT n.id FROM up JOIN nodes n ON n.id = up.id
         WHERE up.parent_id IS NULL AND n.kind = 'space'",
    )
    .bind(id)
    .fetch_optional(&mut *db)
    .await?)
}

/// The document or file a `[[title]]` names: one called that, whatever the
/// case, or one whose slug it is. In the space it is written in first, then
/// in any; one called that before one only slugged so; the oldest of what is
/// still left. Never one in the trash.
async fn resolve_title(
    db: &mut SqliteConnection,
    title: &str,
    space: Option<i64>,
) -> AppResult<Option<i64>> {
    // `[[???]]` slugs to `untitled`, which is not what it names.
    let slug = title
        .chars()
        .any(char::is_alphanumeric)
        .then(|| nodes::slugify(title));
    let rows = sqlx::query(
        "WITH RECURSIVE up (origin, id, parent_id) AS (
             SELECT id, id, parent_id FROM nodes
             WHERE kind IN ('document', 'file') AND (name = ?1 COLLATE NOCASE OR slug = ?2)
             UNION ALL
             SELECT up.origin, n.id, n.parent_id FROM nodes n JOIN up ON n.id = up.parent_id
         )
         SELECT up.origin, up.id AS space, (o.name = ?1 COLLATE NOCASE) AS named
         FROM up JOIN nodes s ON s.id = up.id JOIN nodes o ON o.id = up.origin
         WHERE up.parent_id IS NULL AND s.kind = 'space'",
    )
    .bind(title)
    .bind(slug)
    .fetch_all(&mut *db)
    .await?;

    Ok(rows
        .iter()
        .map(|row| {
            let origin: i64 = row.get("origin");
            let here = space.is_some_and(|space| row.get::<i64, _>("space") == space);
            (here, row.get::<bool, _>("named"), Reverse(origin))
        })
        .max()
        .map(|(_, _, Reverse(origin))| origin))
}

async fn resolve(
    db: &mut SqliteConnection,
    kind: Kind,
    target: &str,
    space: Option<i64>,
) -> AppResult<Option<i64>> {
    match kind {
        Kind::Path => resolve_path(db, target).await,
        Kind::Wiki => resolve_title(db, target, space).await,
    }
}

/// A node's path from its space, `work/notes/plan`, as `/n/` writes it.
async fn path_text(db: &mut SqliteConnection, id: i64) -> AppResult<String> {
    let slugs: Vec<String> = sqlx::query_scalar(
        "WITH RECURSIVE up (id, parent_id, slug, depth) AS (
             SELECT id, parent_id, slug, 0 FROM nodes WHERE id = ?1
             UNION ALL
             SELECT n.id, n.parent_id, n.slug, up.depth + 1
             FROM nodes n JOIN up ON n.id = up.parent_id
         )
         SELECT slug FROM up ORDER BY depth DESC",
    )
    .bind(id)
    .fetch_all(&mut *db)
    .await?;
    Ok(slugs.join("/"))
}

/// What a `[[link]]` to a node says: its name, or its slug when the name
/// could not stand between the brackets.
async fn title_of(db: &mut SqliteConnection, id: i64) -> AppResult<String> {
    let row = sqlx::query("SELECT name, slug FROM nodes WHERE id = ?1")
        .bind(id)
        .fetch_one(&mut *db)
        .await?;
    let name: String = row.get("name");
    let name = name.trim();
    Ok(if name.is_empty() || name.contains(['[', ']', '|', '\n']) {
        row.get("slug")
    } else {
        name.to_string()
    })
}

/// Record what `from` links to. Called by every write of a document's text,
/// after it, in the same transaction.
pub async fn index(db: &mut SqliteConnection, from: i64, body: &str) -> AppResult<()> {
    let space = space_of(db, from).await?;
    let before: HashMap<(Kind, String), i64> =
        sqlx::query("SELECT kind, target, to_id FROM links WHERE from_id = ?1")
            .bind(from)
            .fetch_all(&mut *db)
            .await?
            .iter()
            .map(|row| {
                (
                    (Kind::parse(row.get("kind")), row.get("target")),
                    row.get("to_id"),
                )
            })
            .collect();

    let mut rows = Vec::new();
    for (kind, target) in written(body) {
        let to = match resolve(db, kind, &target, space).await? {
            Some(to) => Some(to),
            // What it linked to is in the trash. The row stays, so a restore
            // that puts it somewhere else still finds the link to rewrite.
            None => match before.get(&(kind, target.clone())) {
                Some(&to) if trash::in_trash(db, to).await? => Some(to),
                _ => None,
            },
        };
        if let Some(to) = to.filter(|&to| to != from) {
            rows.push((kind, target, to));
        }
    }

    sqlx::query("DELETE FROM links WHERE from_id = ?1")
        .bind(from)
        .execute(&mut *db)
        .await?;
    for (kind, target, to) in rows {
        sqlx::query("INSERT INTO links (from_id, kind, target, to_id) VALUES (?1, ?2, ?3, ?4)")
            .bind(from)
            .bind(kind.as_str())
            .bind(&target)
            .bind(to)
            .execute(&mut *db)
            .await?;
    }
    Ok(())
}

/// After `id` was created, renamed or moved, in the same transaction. Every
/// link to it, or to anything under it, that no longer finds what it linked
/// to is rewritten to find it again; a link that found nothing until now and
/// names it now is recorded. Returns how many documents were rewritten.
pub async fn follow(db: &mut SqliteConnection, id: i64) -> AppResult<usize> {
    // Nothing in the trash is followed there: its links stay as they were
    // written, for whatever the restore brings it back as.
    if trash::in_trash(db, id).await? {
        return Ok(0);
    }
    let rows = sqlx::query(
        "WITH RECURSIVE sub (id) AS (
             SELECT ?1
             UNION ALL
             SELECT n.id FROM nodes n JOIN sub ON n.parent_id = sub.id
         )
         SELECT from_id, kind, target, to_id FROM links
         WHERE to_id IN (SELECT id FROM sub)",
    )
    .bind(id)
    .fetch_all(&mut *db)
    .await?;

    let mut linking: BTreeMap<i64, Vec<(Kind, String, i64)>> = BTreeMap::new();
    for row in &rows {
        linking.entry(row.get("from_id")).or_default().push((
            Kind::parse(row.get("kind")),
            row.get("target"),
            row.get("to_id"),
        ));
    }

    let mut rewritten = 0;
    for (from, links) in linking {
        let space = space_of(db, from).await?;
        let mut to = HashMap::new();
        for (kind, target, node) in links {
            if resolve(db, kind, &target, space).await? == Some(node) {
                continue;
            }
            let now = match kind {
                Kind::Path => path_text(db, node).await?,
                Kind::Wiki => title_of(db, node).await?,
            };
            to.insert((kind, target), now);
        }
        if to.is_empty() {
            continue;
        }

        let body: String = sqlx::query_scalar("SELECT body FROM nodes WHERE id = ?1")
            .bind(from)
            .fetch_one(&mut *db)
            .await?;
        let body_now = rewrite(&body, &to);
        if body_now == body {
            continue;
        }
        revisions::keep(db, from, &body_now, revisions::Reason::Edit).await?;
        // `updated_at` moves, so an editor open on the old text is told it
        // changed rather than saving the broken links back over it.
        sqlx::query("UPDATE nodes SET body = ?1, updated_at = ?2 WHERE id = ?3")
            .bind(&body_now)
            .bind(crate::now())
            .bind(from)
            .execute(&mut *db)
            .await?;
        index(db, from, &body_now).await?;
        rewritten += 1;
    }

    claim(db, id).await?;
    Ok(rewritten)
}

/// Index again the documents that may have been waiting for `id`: whose
/// `[[title]]` or path named nothing until it was called that, or put there.
/// A rough cut by text, as the index then decides what each link finds.
async fn claim(db: &mut SqliteConnection, id: i64) -> AppResult<()> {
    let row = sqlx::query("SELECT name, slug FROM nodes WHERE id = ?1")
        .bind(id)
        .fetch_one(&mut *db)
        .await?;
    // `lower` in SQLite folds ASCII only, so the name is folded the same way.
    let name = row.get::<String, _>("name").trim().to_ascii_lowercase();
    let slug: String = row.get("slug");
    let path = path_text(db, id).await?;
    let waiting: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id, body FROM nodes
         WHERE kind = 'document' AND id <> ?1
           AND (instr(body, '/n/' || ?4) > 0
                OR (instr(body, '[[') > 0
                    AND ((?2 <> '' AND instr(lower(body), ?2) > 0) OR instr(body, ?3) > 0)))",
    )
    .bind(id)
    .bind(&name)
    .bind(&slug)
    .bind(&path)
    .fetch_all(&mut *db)
    .await?;
    for (from, body) in waiting {
        index(db, from, &body).await?;
    }
    Ok(())
}

/// The documents that link to `id`, the last written first; none in the
/// trash.
async fn linking(db: &mut SqliteConnection, id: i64) -> AppResult<Vec<i64>> {
    let from: Vec<i64> = sqlx::query_scalar(
        "SELECT DISTINCT l.from_id FROM links l JOIN nodes n ON n.id = l.from_id
         WHERE l.to_id = ?1 ORDER BY n.updated_at DESC, n.id",
    )
    .bind(id)
    .fetch_all(&mut *db)
    .await?;
    let mut live = Vec::with_capacity(from.len());
    for from in from {
        if !trash::in_trash(db, from).await? {
            live.push(from);
        }
    }
    Ok(live)
}

/// How many documents link to `id`, for the line under its title.
pub async fn linked_from(db: &mut SqliteConnection, id: i64) -> AppResult<i64> {
    Ok(linking(db, id).await?.len() as i64)
}

/// Where the `[[titles]]` in `body` lead, as `/n/` paths, for the reader to
/// draw them as links. A title that leads nowhere is left out.
pub async fn wikilinks(
    db: &mut SqliteConnection,
    from: i64,
    body: &str,
) -> AppResult<BTreeMap<String, String>> {
    let space = space_of(db, from).await?;
    wikilinks_in(db, space, body).await
}

/// Where the `[[titles]]` in `body` lead, read from the space `space`: a
/// card's description, which is in a space without being a node in it.
pub async fn wikilinks_in(
    db: &mut SqliteConnection,
    space: Option<i64>,
    body: &str,
) -> AppResult<BTreeMap<String, String>> {
    let mut found = BTreeMap::new();
    let titles: Vec<String> = written(body)
        .into_iter()
        .filter(|(kind, _)| *kind == Kind::Wiki)
        .map(|(_, title)| title)
        .collect();
    if titles.is_empty() {
        return Ok(found);
    }
    for title in titles {
        if let Some(to) = resolve_title(db, &title, space).await? {
            found.insert(title, path_text(db, to).await?);
        }
    }
    Ok(found)
}

#[derive(Serialize)]
pub struct Backlink {
    pub id: i64,
    pub name: String,
    pub slug: String,
    /// Where it sits, `work/notes/plan`.
    pub path: String,
    /// The line the link is on.
    pub context: String,
}

/// How much of the line a link is on a backlink shows.
const CONTEXT_CHARS: usize = 200;

/// `GET /api/nodes/{id}/backlinks` — the documents that link here.
pub async fn backlinks(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<Vec<Backlink>>> {
    let mut conn = state.db.acquire().await?;
    if trash::in_trash(&mut conn, id).await? {
        return Err(AppError::NotFound);
    }
    let mut found = Vec::new();
    for from in linking(&mut conn, id).await? {
        let row = sqlx::query("SELECT name, slug, body FROM nodes WHERE id = ?1")
            .bind(from)
            .fetch_one(&mut *conn)
            .await?;
        let spellings: Vec<String> =
            sqlx::query("SELECT kind, target FROM links WHERE from_id = ?1 AND to_id = ?2")
                .bind(from)
                .bind(id)
                .fetch_all(&mut *conn)
                .await?
                .iter()
                .map(|link| match Kind::parse(link.get("kind")) {
                    Kind::Path => format!("/n/{}", link.get::<String, _>("target")),
                    Kind::Wiki => format!("[[{}", link.get::<String, _>("target")),
                })
                .collect();
        let body: String = row.get("body");
        let context = body
            .lines()
            .find(|line| {
                spellings
                    .iter()
                    .any(|spelling| line.contains(spelling.as_str()))
            })
            .unwrap_or_default()
            .trim();
        let mut context: String = context.chars().take(CONTEXT_CHARS).collect();
        if context.chars().count() == CONTEXT_CHARS {
            context.push('…');
        }
        found.push(Backlink {
            id: from,
            name: row.get("name"),
            slug: row.get("slug"),
            path: path_text(&mut conn, from).await?,
            context,
        });
    }
    Ok(Json(found))
}

/// Index every document, when there is no link recorded at all: the ones
/// written before links were kept. Returns how many documents link anywhere.
pub async fn backfill(db: &SqlitePool) -> AppResult<usize> {
    let any: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM links)")
        .fetch_one(db)
        .await?;
    if any {
        return Ok(0);
    }
    let documents = sqlx::query(&format!(
        "WITH RECURSIVE {} SELECT id, body FROM nodes
         WHERE kind = 'document' AND id IN (SELECT id FROM live)
           AND (instr(body, '/n/') > 0 OR instr(body, '[[') > 0)",
        nodes::LIVE
    ))
    .fetch_all(db)
    .await?;

    let mut tx = crate::db::begin_write(db).await?;
    for row in &documents {
        index(&mut tx, row.get("id"), row.get("body")).await?;
    }
    let linking: i64 = sqlx::query_scalar("SELECT COUNT(DISTINCT from_id) FROM links")
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(linking as usize)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use axum::extract::{Path, State};

    use super::{rewrite, written, Kind};
    use crate::{routes::nodes, state::AppState};

    #[test]
    fn what_counts_as_a_link() {
        let body = "[plan](/n/work/plan) and [notes](</n/work/a/notes#goals>)\n\
                    [ref]: /n/uni/thesis\n\
                    not a link: /n/work/prose, [cased](/n/work/Plan), [file](/n/work/a.md)\n\
                    [[Plan]], [[ Other plan |the other]], [[]], [[a|b|c]] links to a";
        let written = written(body);
        let found: Vec<(Kind, &str)> = written
            .iter()
            .map(|(kind, target)| (*kind, target.as_str()))
            .collect();
        assert_eq!(
            found,
            [
                (Kind::Path, "uni/thesis"),
                (Kind::Path, "work/a/notes"),
                (Kind::Path, "work/plan"),
                (Kind::Wiki, "Other plan"),
                (Kind::Wiki, "Plan"),
                (Kind::Wiki, "a"),
            ]
        );
    }

    #[test]
    fn a_rewrite_moves_only_the_link_it_names() {
        let to = HashMap::from([
            ((Kind::Path, "work/a".to_string()), "work/b".to_string()),
            ((Kind::Wiki, "Plan".to_string()), "Roadmap".to_string()),
        ]);
        let body = "[a](/n/work/a) [deeper](/n/work/a/x) [longer](/n/work/ab) [[Plan]] [[Plan|the plan]] [[Plans]]";
        assert_eq!(
            rewrite(body, &to),
            "[a](/n/work/b) [deeper](/n/work/a/x) [longer](/n/work/ab) [[Roadmap]] [[Roadmap|the plan]] [[Plans]]"
        );
    }

    async fn space(state: &AppState, slug: &str) -> i64 {
        sqlx::query_scalar("SELECT id FROM nodes WHERE slug = ?1 AND parent_id IS NULL")
            .bind(slug)
            .fetch_one(&state.db)
            .await
            .unwrap()
    }

    async fn create(state: &AppState, parent: i64, kind: &str, name: &str, body: &str) -> i64 {
        let input = serde_json::from_value(serde_json::json!({
            "parent_id": parent, "kind": kind, "name": name, "body": body,
        }))
        .unwrap();
        nodes::create(session(), State(state.clone()), axum::Json(input))
            .await
            .unwrap()
            .0
            .id
    }

    async fn rename(state: &AppState, id: i64, name: &str) {
        let body: String = sqlx::query_scalar("SELECT body FROM nodes WHERE id = ?1")
            .bind(id)
            .fetch_one(&state.db)
            .await
            .unwrap();
        let input =
            serde_json::from_value(serde_json::json!({ "name": name, "body": body })).unwrap();
        let _renamed = nodes::update(session(), State(state.clone()), Path(id), axum::Json(input))
            .await
            .unwrap();
    }

    async fn body(state: &AppState, id: i64) -> String {
        sqlx::query_scalar("SELECT body FROM nodes WHERE id = ?1")
            .bind(id)
            .fetch_one(&state.db)
            .await
            .unwrap()
    }

    async fn targets(state: &AppState, from: i64) -> Vec<(String, String, i64)> {
        sqlx::query_as("SELECT kind, target, to_id FROM links WHERE from_id = ?1 ORDER BY kind")
            .bind(from)
            .fetch_all(&state.db)
            .await
            .unwrap()
    }

    fn session() -> crate::auth::Session {
        crate::auth::Session { id: "test".into() }
    }

    /// A rename of the target rewrites the path and the title that link to
    /// it; a rename of a folder above it rewrites the path.
    #[tokio::test]
    async fn a_rename_rewrites_what_links_to_it() {
        let state = crate::routes::tests::state_in(".".into()).await;
        let work = space(&state, "work").await;
        let folder = create(&state, work, "folder", "Notes", "").await;
        let plan = create(&state, folder, "document", "Plan", "the plan").await;
        let linker = create(
            &state,
            work,
            "document",
            "Linker",
            "see [it](/n/work/notes/plan#goals) and [[Plan]]",
        )
        .await;
        assert_eq!(
            targets(&state, linker).await,
            [
                ("path".to_string(), "work/notes/plan".to_string(), plan),
                ("wiki".to_string(), "Plan".to_string(), plan),
            ]
        );

        rename(&state, plan, "Roadmap").await;
        assert_eq!(
            body(&state, linker).await,
            "see [it](/n/work/notes/roadmap#goals) and [[Roadmap]]"
        );
        rename(&state, folder, "Ideas").await;
        assert_eq!(
            body(&state, linker).await,
            "see [it](/n/work/ideas/roadmap#goals) and [[Roadmap]]"
        );
        assert_eq!(targets(&state, linker).await.len(), 2);

        // Each rewrite kept what it replaced, as any other write of the text.
        let kept: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM revisions WHERE node_id = ?1")
            .bind(linker)
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(kept, 1, "the second rewrite came within ten minutes");
    }

    /// A move into another folder rewrites the path; the title still finds it.
    #[tokio::test]
    async fn a_move_rewrites_the_path() {
        let state = crate::routes::tests::state_in(".".into()).await;
        let work = space(&state, "work").await;
        let elsewhere = create(&state, work, "folder", "Elsewhere", "").await;
        let plan = create(&state, work, "document", "Plan", "").await;
        let linker = create(
            &state,
            work,
            "document",
            "Linker",
            "[p](/n/work/plan) [[plan]]",
        )
        .await;

        nodes::move_nodes(&state, &[plan], elsewhere, None)
            .await
            .unwrap();
        assert_eq!(
            body(&state, linker).await,
            "[p](/n/work/elsewhere/plan) [[plan]]"
        );
    }

    /// A title is looked for in the linking document's space first.
    #[tokio::test]
    async fn a_title_resolves_in_its_own_space_first() {
        let state = crate::routes::tests::state_in(".".into()).await;
        let work = space(&state, "work").await;
        let uni = space(&state, "uni").await;
        let at_work = create(&state, work, "document", "Plan", "").await;
        let at_uni = create(&state, uni, "document", "Plan", "").await;
        let from_uni = create(&state, uni, "document", "a", "[[Plan]]").await;
        let from_work = create(&state, work, "document", "b", "[[plan]]").await;
        let only_uni = create(&state, work, "document", "c", "[[Thesis]]").await;
        let thesis = create(&state, uni, "document", "Thesis", "").await;

        assert_eq!(targets(&state, from_uni).await[0].2, at_uni);
        assert_eq!(targets(&state, from_work).await[0].2, at_work);
        // Written before the thesis was, and claimed by it when it came.
        assert_eq!(targets(&state, only_uni).await[0].2, thesis);
    }

    /// The trash keeps the link; a restore elsewhere rewrites it, a purge
    /// leaves the text as it was written.
    #[tokio::test]
    async fn the_trash_keeps_a_link_and_a_purge_drops_it() {
        let state = crate::routes::tests::state_in(".".into()).await;
        let work = space(&state, "work").await;
        let folder = create(&state, work, "folder", "Notes", "").await;
        let plan = create(&state, folder, "document", "Plan", "").await;
        let linker = create(
            &state,
            work,
            "document",
            "Linker",
            "[p](/n/work/notes/plan)",
        )
        .await;

        crate::routes::trash::trash_subtree(&state.db, plan)
            .await
            .unwrap();
        // Written again while the plan is in the trash: the link stays known.
        rename(&state, linker, "Linker").await;
        assert_eq!(targets(&state, linker).await.len(), 1);

        nodes::purge_subtree(&state.db, std::path::Path::new("."), folder)
            .await
            .unwrap();
        // The folder is gone for good, so the plan goes back to the space.
        crate::routes::trash::restore_node(&state, plan)
            .await
            .unwrap();
        assert_eq!(body(&state, linker).await, "[p](/n/work/plan)");

        crate::routes::trash::trash_subtree(&state.db, plan)
            .await
            .unwrap();
        crate::routes::trash::purge_older(&state.db, std::path::Path::new("."), i64::MAX)
            .await
            .unwrap();
        assert!(targets(&state, linker).await.is_empty());
        assert_eq!(body(&state, linker).await, "[p](/n/work/plan)");
    }

    /// The documents written before links were kept are indexed once, and
    /// only while nothing is.
    #[tokio::test]
    async fn the_backfill_indexes_what_came_before() {
        let state = crate::routes::tests::state_in(".".into()).await;
        let work = space(&state, "work").await;
        let plan = create(&state, work, "document", "Plan", "").await;
        let linker = create(&state, work, "document", "Linker", "[[Plan]]").await;
        sqlx::query("DELETE FROM links")
            .execute(&state.db)
            .await
            .unwrap();

        assert_eq!(super::backfill(&state.db).await.unwrap(), 1);
        assert_eq!(targets(&state, linker).await[0].2, plan);
        assert_eq!(super::backfill(&state.db).await.unwrap(), 0);
    }
}
