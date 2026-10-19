//! The readable side of the mirror: every file node of the workspace under
//! `files/` on the device, in its folder and by its name.
//!
//! ```text
//! narl-workspace/
//!   data/                  the restorable copy (see `sync`)
//!   files/<space>/<folder>/<name>
//! ```
//!
//! `data/uploads/<uuid>` is what a restore needs, but nobody can find a lecture
//! PDF in a folder of uuids. This is the same bytes laid out the way the tree
//! shows them, so Proton Drive's own apps can open them. It is a view, not the
//! backup: it follows renames, moves and deletes whatever
//! `WORKSPACE_BACKUP_PRUNE` says, and a failure in it is logged rather than
//! failing the run, because the copy that matters is already in `data/`.
//!
//! A rename or a move becomes a rename or a move on Drive — never a second
//! upload of bytes that are already there.

use std::collections::{BTreeSet, HashMap, HashSet};

use anyhow::{Context, Result};
use proton_drive_rs::ProtonDriveClient;
use proton_sdk::ids::NodeUid;
use sqlx::{Row, SqlitePool};

use super::sync::{parse_uid, Summary};
use crate::config::Config;

/// The longest name given to Drive. Proton allows a little more; the margin
/// is for the ` (123)` a clash appends.
const MAX_NAME: usize = 200;

/// One node, as much of it as the layout needs.
pub(crate) struct TreeNode {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub kind: String,
    pub name: String,
    pub slug: String,
    pub media_id: Option<String>,
    pub mime: String,
}

/// Where one file node belongs in the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Placed {
    pub node_id: i64,
    pub media_id: String,
    pub mime: String,
    /// The folder under `files/`, `uni/algo`; never empty, a space is the least.
    pub dir: String,
    pub name: String,
}

impl Placed {
    fn path(&self) -> String {
        format!("{}/{}", self.dir, self.name)
    }
}

/// A name Drive will take: no separators, no control characters, nothing
/// that means "here" or "up", not too long. What cannot be saved becomes the
/// slug, which always can.
fn clean(name: &str, slug: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c == '/' || c == '\\' || c.is_control() { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        return slug.to_string();
    }
    cleaned.chars().take(MAX_NAME).collect()
}

/// `slides.pdf` → `slides (42).pdf`, for the second of two files that share
/// a name. Ids never change, so the loser keeps the same name every run.
fn disambiguate(name: &str, id: i64) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem} ({id}).{ext}"),
        _ => format!("{name} ({id})"),
    }
}

/// Lay the file nodes out as paths. Names are unique per folder only up to
/// case on most of the systems this ends up synced to, and two folders may
/// share a name where they never share a slug — so the first of a name, by
/// id, keeps it and the later ones are told apart.
pub(crate) fn layout(nodes: &[TreeNode]) -> Vec<Placed> {
    let mut sorted: Vec<&TreeNode> = nodes.iter().collect();
    sorted.sort_by_key(|n| n.id);

    let mut taken: HashMap<Option<i64>, HashSet<String>> = HashMap::new();
    let mut segment: HashMap<i64, String> = HashMap::new();
    let parent_of: HashMap<i64, Option<i64>> = nodes.iter().map(|n| (n.id, n.parent_id)).collect();

    for node in sorted.iter().filter(|n| n.kind == "space" || n.kind == "folder") {
        let name = clean(&node.name, &node.slug);
        let names = taken.entry(node.parent_id).or_default();
        let name = if names.insert(name.to_lowercase()) {
            name
        } else {
            let other = format!("{name} ({})", node.slug);
            names.insert(other.to_lowercase());
            other
        };
        segment.insert(node.id, name);
    }

    // A folder's path, from the segments of its ancestors. Bounded by the node
    // count, so a parent chain that somehow loops cannot spin here.
    let dir_of = |mut id: i64| -> Option<String> {
        let mut parts = Vec::new();
        for _ in 0..=nodes.len() {
            parts.push(segment.get(&id)?.clone());
            match parent_of.get(&id)? {
                Some(parent) => id = *parent,
                None => {
                    parts.reverse();
                    return Some(parts.join("/"));
                }
            }
        }
        None
    };

    let mut placed = Vec::new();
    for node in sorted.iter().filter(|n| n.kind == "file") {
        let (Some(media_id), Some(parent)) = (&node.media_id, node.parent_id) else {
            continue;
        };
        let Some(dir) = dir_of(parent) else {
            continue;
        };
        let name = clean(&node.name, &node.slug);
        let names = taken.entry(Some(parent)).or_default();
        let name = if names.insert(name.to_lowercase()) {
            name
        } else {
            let other = disambiguate(&name, node.id);
            names.insert(other.to_lowercase());
            other
        };
        placed.push(Placed {
            node_id: node.id,
            media_id: media_id.clone(),
            mime: node.mime.clone(),
            dir,
            name,
        });
    }
    placed
}

/// Every folder `path` sits in, itself included: `a/b` → `a`, `a/b`.
fn prefixes(path: &str) -> impl Iterator<Item = &str> {
    path.match_indices('/')
        .map(move |(at, _)| &path[..at])
        .chain(std::iter::once(path))
}

struct Tracked {
    path: String,
    media_id: String,
    uid: NodeUid,
}

/// Bring `files/` in line with the tree.
pub async fn run(
    db: &SqlitePool,
    config: &Config,
    client: &ProtonDriveClient,
    root: &NodeUid,
    summary: &mut Summary,
) -> Result<()> {
    // A sealed file has no place in a view meant to be opened by name from
    // anywhere: it is left out, so it leaves the view when it is sealed.
    let tree: Vec<TreeNode> = sqlx::query(
        "SELECT n.id, n.parent_id, n.kind, n.name, n.slug,
                CASE WHEN m.encrypted = 0 THEN n.media_id END AS media_id,
                COALESCE(m.mime, '') AS mime
         FROM nodes n LEFT JOIN media m ON m.id = n.media_id",
    )
    .fetch_all(db)
    .await?
    .iter()
    .map(|row| TreeNode {
        id: row.get("id"),
        parent_id: row.get("parent_id"),
        kind: row.get("kind"),
        name: row.get("name"),
        slug: row.get("slug"),
        media_id: row.get("media_id"),
        mime: row.get("mime"),
    })
    .collect();
    let wanted = layout(&tree);

    let mut tracked: HashMap<i64, Tracked> = HashMap::new();
    for row in sqlx::query("SELECT node_id, path, media_id, node_uid FROM backup_view")
        .fetch_all(db)
        .await?
    {
        let node_id: i64 = row.get("node_id");
        match parse_uid(&row.get::<String, _>("node_uid")) {
            Some(uid) => {
                tracked.insert(
                    node_id,
                    Tracked { path: row.get("path"), media_id: row.get("media_id"), uid },
                );
            }
            None => forget(db, node_id).await?,
        }
    }

    let mut dirs: HashMap<String, NodeUid> = HashMap::new();
    for row in sqlx::query("SELECT path, node_uid FROM backup_view_dirs").fetch_all(db).await? {
        if let Some(uid) = parse_uid(&row.get::<String, _>("node_uid")) {
            dirs.insert(row.get("path"), uid);
        }
    }

    let sealed: HashSet<i64> = sqlx::query_scalar(
        "SELECT n.id FROM nodes n JOIN media m ON m.id = n.media_id WHERE m.encrypted <> 0",
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .collect();

    let mut view = View { db, config, client, root, dirs, summary };

    // What is gone, or is no longer the same bytes, goes first: it frees the
    // names the moves below may want.
    let by_id: HashMap<i64, &Placed> = wanted.iter().map(|p| (p.node_id, p)).collect();
    let stale: Vec<i64> = tracked
        .iter()
        .filter(|(id, t)| by_id.get(id).is_none_or(|p| p.media_id != t.media_id))
        .map(|(id, _)| *id)
        .collect();
    for id in stale {
        if let Some(gone) = tracked.remove(&id) {
            // What was sealed since leaves no copy in Drive's trash either.
            if sealed.contains(&id) {
                if let Err(e) = super::sync::purge(client, &gone.uid).await {
                    tracing::warn!(file = %gone.path, error = %format!("{e:#}"), "could not purge a sealed file from files/");
                    continue;
                }
                view.summary.pruned += 1;
            } else {
                view.trash(&gone.uid).await;
            }
            forget(db, id).await?;
        }
    }

    for placed in &wanted {
        let outcome = match tracked.get(&placed.node_id) {
            Some(t) if t.path == placed.path() => {
                view.summary.skipped += 1;
                Ok(())
            }
            Some(t) => view.relocate(t, placed).await,
            None => view.upload(placed).await,
        };
        if let Err(e) = outcome {
            tracing::warn!(file = %placed.path(), error = %format!("{e:#}"), "could not mirror a file into files/");
            // A folder remembered by a uid that has since gone would fail every
            // run; forgetting it lets the next one find or make it again.
            view.forget_dirs(&placed.dir).await?;
        }
    }

    // Folders nothing is in any more. Only the outermost is trashed, which
    // takes the ones inside along.
    let needed: BTreeSet<&str> = wanted.iter().flat_map(|p| prefixes(&p.dir)).collect();
    let mut unneeded: Vec<String> = view
        .dirs
        .keys()
        .filter(|path| !needed.contains(path.as_str()))
        .cloned()
        .collect();
    unneeded.sort();
    let mut trashed: Vec<String> = Vec::new();
    for path in unneeded {
        let inside_trashed = trashed.iter().any(|t| path.starts_with(&format!("{t}/")));
        if !inside_trashed {
            if let Some(uid) = view.dirs.get(&path).cloned() {
                view.trash(&uid).await;
            }
            trashed.push(path.clone());
        }
        view.dirs.remove(&path);
        sqlx::query("DELETE FROM backup_view_dirs WHERE path = ?")
            .bind(&path)
            .execute(db)
            .await?;
    }

    Ok(())
}

async fn forget(db: &SqlitePool, node_id: i64) -> Result<()> {
    sqlx::query("DELETE FROM backup_view WHERE node_id = ?")
        .bind(node_id)
        .execute(db)
        .await?;
    Ok(())
}

struct View<'a> {
    db: &'a SqlitePool,
    config: &'a Config,
    client: &'a ProtonDriveClient,
    root: &'a NodeUid,
    dirs: HashMap<String, NodeUid>,
    summary: &'a mut Summary,
}

impl View<'_> {
    /// The folder at `path` under `files/`, made one segment at a time so each
    /// one is remembered and can be trashed once it is empty.
    async fn dir(&mut self, path: &str) -> Result<NodeUid> {
        let mut parent = self.root.clone();
        for prefix in prefixes(path) {
            if let Some(uid) = self.dirs.get(prefix) {
                parent = uid.clone();
                continue;
            }
            let segment = prefix.rsplit('/').next().unwrap_or(prefix);
            // Finds the folder when it is already there, so a forgotten one is
            // picked up again rather than made twice.
            let uid = self
                .client
                .create_folder_path(&parent, segment)
                .await
                .with_context(|| format!("could not make files/{prefix}"))?;
            sqlx::query(
                "INSERT INTO backup_view_dirs (path, node_uid) VALUES (?, ?)
                 ON CONFLICT (path) DO UPDATE SET node_uid = excluded.node_uid",
            )
            .bind(prefix)
            .bind(uid.to_string())
            .execute(self.db)
            .await?;
            self.dirs.insert(prefix.to_string(), uid.clone());
            parent = uid;
        }
        Ok(parent)
    }

    async fn forget_dirs(&mut self, path: &str) -> Result<()> {
        for prefix in prefixes(path) {
            self.dirs.remove(prefix);
            sqlx::query("DELETE FROM backup_view_dirs WHERE path = ?")
                .bind(prefix)
                .execute(self.db)
                .await?;
        }
        Ok(())
    }

    /// Move and rename a file already on Drive. If Drive refuses — the name
    /// taken, the node gone — it is trashed and uploaded afresh instead.
    async fn relocate(&mut self, tracked: &Tracked, placed: &Placed) -> Result<()> {
        let (old_dir, old_name) = tracked.path.rsplit_once('/').unwrap_or(("", &tracked.path));
        let moved = async {
            if old_dir != placed.dir {
                let target = self.dir(&placed.dir).await?;
                self.client.move_node(&tracked.uid, &target).await?;
            }
            if old_name != placed.name {
                self.client
                    .rename_node(&tracked.uid, &placed.name, Some(&placed.mime))
                    .await?;
            }
            anyhow::Ok(())
        }
        .await;

        match moved {
            Ok(()) => {
                self.record(placed, &tracked.uid).await?;
                tracing::info!(from = %tracked.path, to = %placed.path(), "moved in files/");
                Ok(())
            }
            Err(e) => {
                tracing::warn!(file = %tracked.path, error = %format!("{e:#}"), "could not move in files/; uploading again");
                self.trash(&tracked.uid).await;
                forget(self.db, placed.node_id).await?;
                self.upload(placed).await
            }
        }
    }

    async fn upload(&mut self, placed: &Placed) -> Result<()> {
        let path = self.config.uploads_dir().join(&placed.media_id);
        let Ok(metadata) = tokio::fs::metadata(&path).await else {
            tracing::warn!(media = %placed.media_id, "file node has no bytes on disk; skipping");
            return Ok(());
        };
        let size = metadata.len();
        let parent = self.dir(&placed.dir).await?;

        let fresh = self
            .client
            .upload_file_from(
                &parent,
                &placed.name,
                &placed.mime,
                std::fs::File::open(&path)?,
                size as i64,
                Vec::new(),
                Some(crate::now()),
                false,
            )
            .await;

        let uid = match fresh {
            Ok(uid) => uid,
            // A file of that name already there — left by a run whose
            // bookkeeping a restore rolled back — is taken over as a new
            // revision rather than fought over.
            Err(e) => match self.client.get_node_by_path(&parent, &placed.name).await {
                Ok(Some(existing)) => {
                    let uid = existing.uid.clone();
                    self.client
                        .upload_new_revision_from(&uid, std::fs::File::open(&path)?, size as i64, Vec::new(), Some(crate::now()))
                        .await
                        .with_context(|| format!("could not upload files/{}", placed.path()))?;
                    uid
                }
                _ => return Err(e).with_context(|| format!("could not upload files/{}", placed.path())),
            },
        };

        self.record(placed, &uid).await?;
        self.summary.uploaded += 1;
        self.summary.bytes += size;
        tracing::info!(file = %placed.path(), bytes = size, "mirrored into files/");
        Ok(())
    }

    async fn record(&self, placed: &Placed, uid: &NodeUid) -> Result<()> {
        sqlx::query(
            "INSERT INTO backup_view (node_id, path, media_id, node_uid) VALUES (?, ?, ?, ?)
             ON CONFLICT (node_id) DO UPDATE SET
                 path = excluded.path, media_id = excluded.media_id, node_uid = excluded.node_uid",
        )
        .bind(placed.node_id)
        .bind(placed.path())
        .bind(&placed.media_id)
        .bind(uid.to_string())
        .execute(self.db)
        .await?;
        Ok(())
    }

    /// Into Drive's trash, where it can still be fished out for a while. A
    /// failure is only logged: the worst case is a stale copy in the view.
    async fn trash(&mut self, uid: &NodeUid) {
        match self.client.trash_nodes(std::slice::from_ref(uid)).await {
            Ok(results) => {
                for (uid, result) in results {
                    match result {
                        Ok(()) => self.summary.pruned += 1,
                        Err(e) => tracing::warn!(node = %uid, error = %e, "could not trash in files/"),
                    }
                }
            }
            Err(e) => tracing::warn!(node = %uid, error = %e, "could not trash in files/"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{clean, layout, prefixes, Placed, TreeNode};

    fn node(id: i64, parent_id: Option<i64>, kind: &str, name: &str) -> TreeNode {
        TreeNode {
            id,
            parent_id,
            kind: kind.into(),
            name: name.into(),
            slug: format!("slug-{id}"),
            media_id: (kind == "file").then(|| format!("m{id}")),
            mime: "application/pdf".into(),
        }
    }

    fn paths(placed: &[Placed]) -> Vec<String> {
        placed.iter().map(Placed::path).collect()
    }

    #[test]
    fn files_land_in_their_folder_under_their_name() {
        let tree = [
            node(1, None, "space", "uni"),
            node(2, Some(1), "folder", "Algorithms"),
            node(3, Some(2), "file", "week 1.pdf"),
            node(4, Some(2), "document", "notes"),
            node(5, Some(1), "file", "syllabus.pdf"),
        ];
        assert_eq!(paths(&layout(&tree)), ["uni/Algorithms/week 1.pdf", "uni/syllabus.pdf"]);
    }

    #[test]
    fn a_clash_is_told_apart_by_id_and_the_first_keeps_its_name() {
        let tree = [
            node(1, None, "space", "uni"),
            node(2, Some(1), "folder", "algo"),
            node(3, Some(1), "folder", "Algo"),
            node(4, Some(2), "file", "slides.pdf"),
            node(5, Some(2), "file", "Slides.pdf"),
            node(6, Some(3), "file", "slides.pdf"),
            node(7, Some(1), "file", "algo"),
        ];
        assert_eq!(
            paths(&layout(&tree)),
            [
                "uni/algo/slides.pdf",
                "uni/algo/Slides (5).pdf",
                "uni/Algo (slug-3)/slides.pdf",
                "uni/algo (7)",
            ]
        );
    }

    #[test]
    fn names_drive_cannot_take_are_made_safe() {
        assert_eq!(clean("a/b\\c", "s"), "a_b_c");
        assert_eq!(clean("  ..  ", "s"), "s");
        assert_eq!(clean("", "s"), "s");
        assert_eq!(clean(&"x".repeat(300), "s").chars().count(), 200);
    }

    #[test]
    fn a_file_whose_folder_is_missing_is_left_out() {
        let tree = [node(1, None, "space", "uni"), node(3, Some(9), "file", "lost.pdf")];
        assert!(layout(&tree).is_empty());
    }

    #[test]
    fn prefixes_walk_down_the_path() {
        assert_eq!(prefixes("a/b/c").collect::<Vec<_>>(), ["a", "a/b", "a/b/c"]);
        assert_eq!(prefixes("a").collect::<Vec<_>>(), ["a"]);
    }
}
