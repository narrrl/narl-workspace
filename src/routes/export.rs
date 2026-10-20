use std::{
    collections::{HashMap, HashSet},
    io::{Seek, SeekFrom, Write},
    sync::LazyLock,
};

use axum::{
    body::Body,
    extract::State,
    http::header,
    response::{IntoResponse, Response},
};
use regex::Regex;
use sqlx::Row;
use tokio_util::io::ReaderStream;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

use crate::{
    auth::Session,
    error::{AppError, AppResult},
    routes::{
        manifest::{self, CardMeta, ListMeta, Manifest, NodeMeta, SpaceMeta},
        media::MEDIA_REF,
        nodes::{is_container, DOCUMENT, LIVE, SPACE},
    },
    state::AppState,
};

/// A markdown link to an address inside the application, `](/n/work/notes)`,
/// up to the `)` or `#` that ends it — which is what keeps `/n/work/a` from
/// matching inside `/n/work/ab`.
static NODE_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\]\((/n/[^)#\s]+)([)#])").expect("static regex is valid"));

/// One node, flattened out of the database ahead of the blocking zip work.
struct ExportNode {
    id: i64,
    parent_id: Option<i64>,
    kind: String,
    name: String,
    slug: String,
    body: String,
    media_id: Option<String>,
    created_at: i64,
    updated_at: i64,
    has_board: bool,
    role: Option<String>,
}

/// A container's directory in the archive.
fn container_path(nodes: &HashMap<i64, ExportNode>, node: &ExportNode) -> String {
    tree_path(nodes, node.parent_id, node.slug.clone())
}

/// Where a document lands in the archive: the slugs of its ancestors as
/// directories, its own slug as the file. Slugs are already unique among
/// siblings, so the path is unique without adding the id to it.
fn document_path(nodes: &HashMap<i64, ExportNode>, node: &ExportNode) -> String {
    tree_path(nodes, node.parent_id, format!("{}.md", node.slug))
}

/// `leaf` under the directories of a node's ancestors.
fn tree_path(nodes: &HashMap<i64, ExportNode>, parent: Option<i64>, leaf: String) -> String {
    let mut segments = vec![leaf];
    let mut parent = parent;
    // Bounded by the number of nodes, so a parent chain that somehow loops
    // cannot spin here.
    for _ in 0..nodes.len() {
        let Some(id) = parent else { break };
        let Some(ancestor) = nodes.get(&id) else {
            break;
        };
        segments.push(ancestor.slug.clone());
        parent = ancestor.parent_id;
    }
    segments.reverse();
    segments.join("/")
}

/// A link from one archived document to another, relative to the first. The
/// application addresses documents as `/n/<space>/<slug>/...`, which means
/// nothing outside the application, so the archive turns those links back into
/// file paths — the shape they had before they were ever imported.
fn relative_link(from: &str, to: &str) -> String {
    let from: Vec<&str> = from.split('/').collect();
    let to: Vec<&str> = to.split('/').collect();
    // The last segment of `from` is the file itself, not a directory.
    let shared = from[..from.len() - 1]
        .iter()
        .zip(to[..to.len() - 1].iter())
        .take_while(|(a, b)| a == b)
        .count();

    let up = "../".repeat(from.len() - 1 - shared);
    format!("{up}{}", to[shared..].join("/"))
}

/// `YYYY-MM-DD` in UTC, from a Unix timestamp, without pulling in a date crate.
fn day_string(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    // Civil-from-days, Howard Hinnant's algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Only characters that are safe in a path *and* need no escaping inside a
/// markdown link survive, because the entries link to these names directly.
fn safe_name(original: &str) -> String {
    let safe: String = original
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    safe.trim_matches(['.', '_']).to_string()
}

/// Uploads are stored under a uuid; the archive gives them their name back.
fn media_filename(id: &str, original: &str) -> String {
    let safe = safe_name(original);
    if safe.is_empty() {
        id.to_string()
    } else {
        format!("{id}-{safe}")
    }
}

/// Where a file node lands: in its folder, under its own name. Two files may
/// share a name where they can never share a slug, so the loser of a clash is
/// told apart by its node id.
fn file_path(
    nodes: &HashMap<i64, ExportNode>,
    node: &ExportNode,
    taken: &mut std::collections::HashSet<String>,
) -> String {
    let safe = safe_name(&node.name);
    let name = if safe.is_empty() {
        node.slug.clone()
    } else {
        safe
    };
    let path = tree_path(nodes, node.parent_id, name.clone());
    if taken.insert(path.clone()) {
        return path;
    }
    let path = tree_path(nodes, node.parent_id, format!("{}-{name}", node.id));
    taken.insert(path.clone());
    path
}

const README: &str = "\
This is a complete copy of a ~/workspace.

  <space>/          one directory per space, holding the same tree of folders,
                    documents and files that the workspace shows; one markdown
                    file per document, and every file under its own name
  media/            the files documents embed that are not in the tree themselves
  .workspace.json   what plain files cannot hold: dates, names as they were
                    typed, the boards. Importing the archive puts it all back.

Embedded media, and links from one document to another, are relative — so the
documents render and cross-link correctly in any markdown reader as long as the
tree stays intact. Nothing here needs the workspace application to read it.
";

/// Everything, as one zip. A workspace you cannot get out of is not one you
/// can trust, so this is deliberately a plain archive of plain files.
pub async fn export(_: Session, State(state): State<AppState>) -> AppResult<Response> {
    let nodes: HashMap<i64, ExportNode> = sqlx::query(&format!(
        "WITH RECURSIVE {LIVE}
         SELECT id, parent_id, kind, name, slug, body, media_id, created_at, updated_at,
                has_board, role
         FROM nodes WHERE id IN (SELECT id FROM live) ORDER BY created_at, id"
    ))
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(|row| {
        let node = ExportNode {
            id: row.get("id"),
            parent_id: row.get("parent_id"),
            kind: row.get("kind"),
            name: row.get("name"),
            slug: row.get("slug"),
            body: row.get("body"),
            media_id: row.get("media_id"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
            has_board: row.get("has_board"),
            role: row.get("role"),
        };
        (node.id, node)
    })
    .collect();

    let rows = sqlx::query("SELECT id, filename, encrypted FROM media")
        .fetch_all(&state.db)
        .await?;
    let media: Vec<(String, String)> = rows
        .iter()
        .map(|row| (row.get("id"), row.get("filename")))
        .collect();
    let sealed: HashSet<String> = rows
        .iter()
        .filter(|row| row.get::<bool, _>("encrypted"))
        .map(|row| row.get("id"))
        .collect();

    let lists: Vec<(i64, i64, String)> =
        sqlx::query("SELECT id, space_id, name FROM board_lists ORDER BY position, id")
            .fetch_all(&state.db)
            .await?
            .iter()
            .map(|row| (row.get("id"), row.get("space_id"), row.get("name")))
            .collect();
    let cards: Vec<(i64, CardMeta, Option<i64>)> = sqlx::query(
        "SELECT list_id, title, body, due_at, done_at, created_at, updated_at, node_id
         FROM cards ORDER BY position, id",
    )
    .fetch_all(&state.db)
    .await?
    .iter()
    .map(|row| {
        let card = CardMeta {
            title: row.get("title"),
            body: row.get("body"),
            due_at: row.get("due_at"),
            done_at: row.get("done_at"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
            document: None,
        };
        (row.get("list_id"), card, row.get("node_id"))
    })
    .collect();

    // The archive is meant to be read without this application, so sealed
    // files go in opened — which needs the vault open. Leaving them out
    // instead would make an export that silently is not complete.
    let key = if sealed.is_empty() {
        None
    } else {
        Some(state.vault.key().ok_or(AppError::Locked)?)
    };

    let uploads = state.config.uploads_dir();
    let scratch = state.config.data_dir.clone();

    // Zip writing is synchronous and reads every upload off disk, so it happens
    // on the blocking pool, into a temporary file rather than into memory: a
    // diary with photographs in it does not fit comfortably in a Vec. The file
    // is made in the data directory, which is sized for the workspace, rather
    // than in a /tmp that may be a small tmpfs.
    let file = tokio::task::spawn_blocking(move || -> AppResult<std::fs::File> {
        let mut zip = ZipWriter::new(tempfile::tempfile_in(&scratch)?);
        let text = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        // Media is overwhelmingly already-compressed; deflating it again costs
        // a lot of CPU to save nothing.
        let blob = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

        zip.start_file("README.txt", text)?;
        zip.write_all(README.as_bytes())?;

        // Every path the archive will hold, claimed in one set: directories
        // first, then documents, then files — the only ones that can give
        // way, by taking their id — so no two entries ever share a name.
        let mut ordered: Vec<&ExportNode> = nodes.values().collect();
        ordered.sort_by_key(|n| n.id);
        let mut taken: HashSet<String> =
            HashSet::from(["README.txt".into(), manifest::FILE_NAME.into()]);
        let containers: Vec<(&ExportNode, String)> = ordered
            .iter()
            .filter(|n| is_container(&n.kind))
            .map(|n| (*n, container_path(&nodes, n)))
            .collect();
        taken.extend(containers.iter().map(|(_, path)| path.clone()));
        let documents: Vec<(&ExportNode, String)> = ordered
            .iter()
            .filter(|n| n.kind == DOCUMENT)
            .map(|n| (*n, document_path(&nodes, n)))
            .collect();
        taken.extend(documents.iter().map(|(_, path)| path.clone()));
        let in_tree: HashMap<String, (&ExportNode, String)> = ordered
            .iter()
            .filter_map(|n| Some((n.media_id.clone()?, (*n, file_path(&nodes, n, &mut taken)))))
            .collect();

        let names: HashMap<String, String> = media
            .iter()
            .filter(|(id, _)| !in_tree.contains_key(id))
            .map(|(id, filename)| (id.clone(), media_filename(id, filename)))
            .collect();

        // Every document and folder by the address the application knows it
        // as, so a link to one can be found and pointed at the file instead.
        let addresses: HashMap<String, String> = documents
            .iter()
            .map(|(_, path)| (format!("/n/{}", path.trim_end_matches(".md")), path.clone()))
            .chain(
                containers
                    .iter()
                    .map(|(_, path)| (format!("/n/{path}"), path.clone())),
            )
            .collect();

        let mut meta = Manifest {
            format: manifest::FORMAT,
            ..Manifest::default()
        };
        let mut note = |path: &str, node: &ExportNode| {
            meta.nodes.insert(
                path.to_string(),
                NodeMeta {
                    kind: node.kind.clone(),
                    name: node.name.clone(),
                    created_at: node.created_at,
                    updated_at: node.updated_at,
                },
            );
        };

        for (node, path) in &containers {
            note(path, node);
            // An empty folder is still part of the tree.
            zip.add_directory(format!("{path}/"), text)?;
        }

        for (node, path) in &documents {
            note(path, node);
            // A document three directories deep needs three steps back up to
            // reach the media folder next to the spaces.
            let up = "../".repeat(path.matches('/').count());

            // One pass for each kind of link, looking every match up, rather
            // than one pass per file and per document the workspace holds.
            let body = MEDIA_REF.replace_all(&node.body, |caps: &regex::Captures| {
                let id = caps[1].to_lowercase();
                if let Some(name) = names.get(&id) {
                    format!("{up}{}/{name}", manifest::MEDIA_DIR)
                } else if let Some((_, target)) = in_tree.get(&id) {
                    relative_link(path, target)
                } else {
                    caps[0].to_string()
                }
            });
            let body = NODE_LINK.replace_all(&body, |caps: &regex::Captures| {
                match addresses.get(&caps[1]) {
                    Some(target) => format!("]({}{}", relative_link(path, target), &caps[2]),
                    None => caps[0].to_string(),
                }
            });
            let document = if node.name.trim().is_empty() {
                body.into_owned()
            } else {
                format!("# {}\n\n{body}", node.name.trim())
            };

            zip.start_file(path.as_str(), text)?;
            zip.write_all(document.as_bytes())?;
        }

        for (node, path) in in_tree.values() {
            note(path, node);
        }

        let mut entries: Vec<(&String, String)> = names
            .iter()
            .map(|(id, name)| (id, format!("{}/{name}", manifest::MEDIA_DIR)))
            .chain(in_tree.iter().map(|(id, (_, path))| (id, path.clone())))
            .collect();
        entries.sort();
        for (id, entry) in entries {
            let path = uploads.join(id);
            let source: Box<dyn std::io::Read> = match (&key, sealed.contains(id)) {
                // A sealed file that will not open is not a file to leave out
                // quietly: the archive would claim to be complete and not be.
                (Some(key), true) => Box::new(key.reader(id, &path).map_err(|e| {
                    AppError::BadRequest(format!(
                        "{entry} is sealed and would not open ({e}) — nothing was exported"
                    ))
                })?),
                // A row whose file has gone missing must not fail the whole
                // export; there is nothing to put in its place either way.
                _ => match std::fs::File::open(&path) {
                    Ok(file) => Box::new(file),
                    Err(e) => {
                        tracing::warn!("export: {entry} has no file on disk ({e}), left out");
                        continue;
                    }
                },
            };
            if let Some(original) = media.iter().find(|(m, _)| m == id).map(|(_, f)| f) {
                if !in_tree.contains_key(id) {
                    meta.media.insert(entry.clone(), original.clone());
                }
            }
            zip.start_file(entry, blob)?;
            std::io::copy(&mut { source }, &mut zip)?;
        }

        // The boards, with each card's notes named by where they sit in here.
        let paths: HashMap<i64, &String> = documents.iter().map(|(n, path)| (n.id, path)).collect();
        for (node, path) in &containers {
            if node.kind != SPACE || node.parent_id.is_some() {
                continue;
            }
            let lists = lists
                .iter()
                .filter(|(_, space, _)| *space == node.id)
                .map(|(id, _, name)| ListMeta {
                    name: name.clone(),
                    cards: cards
                        .iter()
                        .filter(|(list, _, _)| list == id)
                        .map(|(_, card, document)| CardMeta {
                            document: document.and_then(|d| paths.get(&d).map(|p| (*p).clone())),
                            ..card.clone()
                        })
                        .collect(),
                })
                .collect();
            meta.spaces.insert(
                path.clone(),
                SpaceMeta {
                    has_board: node.has_board,
                    role: node.role.clone(),
                    lists,
                },
            );
        }

        zip.start_file(manifest::FILE_NAME, text)?;
        serde_json::to_writer_pretty(&mut zip, &meta).map_err(anyhow::Error::from)?;

        let mut file = zip.finish()?;
        file.seek(SeekFrom::Start(0))?;
        Ok(file)
    })
    .await
    .map_err(anyhow::Error::from)??;

    let size = file.metadata()?.len();
    let name = format!("workspace-{}.zip", day_string(crate::now()));

    Ok((
        [
            (header::CONTENT_TYPE, "application/zip".to_string()),
            (header::CONTENT_LENGTH, size.to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{name}\""),
            ),
            (header::CACHE_CONTROL, "no-store".to_string()),
        ],
        Body::from_stream(ReaderStream::new(tokio::fs::File::from_std(file))),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{day_string, document_path, file_path, media_filename, relative_link, ExportNode};

    #[test]
    fn renders_days_without_a_date_crate() {
        assert_eq!(day_string(0), "1970-01-01");
        assert_eq!(day_string(1_757_030_400), "2025-09-05");
        assert_eq!(day_string(951_782_400), "2000-02-29"); // a leap day the 100-year rule skips
        assert_eq!(day_string(-1), "1969-12-31");
    }

    fn tree(rows: &[(i64, Option<i64>, &str, &str)]) -> HashMap<i64, ExportNode> {
        rows.iter()
            .map(|(id, parent_id, kind, slug)| {
                (
                    *id,
                    ExportNode {
                        id: *id,
                        parent_id: *parent_id,
                        kind: (*kind).into(),
                        name: (*slug).into(),
                        slug: (*slug).into(),
                        body: String::new(),
                        media_id: (*kind == "file").then(|| format!("m{id}")),
                        created_at: 0,
                        updated_at: 0,
                        has_board: false,
                        role: None,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn a_file_keeps_its_name_and_a_clash_gets_its_id() {
        let mut nodes = tree(&[
            (1, None, "space", "uni"),
            (2, Some(1), "folder", "algo"),
            (3, Some(2), "file", "slides-pdf"),
            (4, Some(2), "file", "slides-pdf-2"),
        ]);
        nodes.get_mut(&3).unwrap().name = "Slides 3.pdf".into();
        nodes.get_mut(&4).unwrap().name = "Slides 3.pdf".into();

        let mut taken = std::collections::HashSet::new();
        assert_eq!(
            file_path(&nodes, &nodes[&3], &mut taken),
            "uni/algo/Slides_3.pdf"
        );
        assert_eq!(
            file_path(&nodes, &nodes[&4], &mut taken),
            "uni/algo/4-Slides_3.pdf"
        );
    }

    #[test]
    fn a_document_keeps_its_place_in_the_tree() {
        let nodes = tree(&[
            (1, None, "space", "work"),
            (2, Some(1), "folder", "einarbeiten"),
            (3, Some(2), "document", "01-was-ist-eebus"),
            (4, Some(1), "document", "notes"),
        ]);
        assert_eq!(
            document_path(&nodes, &nodes[&3]),
            "work/einarbeiten/01-was-ist-eebus.md"
        );
        assert_eq!(document_path(&nodes, &nodes[&4]), "work/notes.md");
    }

    #[test]
    fn a_broken_parent_chain_still_terminates() {
        // A parent that is not in the map, and a cycle: neither may hang the
        // export or walk off the end of the archive.
        let orphan = tree(&[(3, Some(99), "document", "loose")]);
        assert_eq!(document_path(&orphan, &orphan[&3]), "loose.md");

        let cycle = tree(&[(1, Some(2), "folder", "a"), (2, Some(1), "document", "b")]);
        assert!(document_path(&cycle, &cycle[&2]).ends_with("b.md"));
    }

    #[test]
    fn media_names_cannot_escape_their_folder() {
        assert_eq!(media_filename("abc", "../../etc/passwd"), "abc-etc_passwd");
        // Nothing that would need escaping in the markdown link that points at it.
        assert_eq!(
            media_filename("abc", "my photo (1).png"),
            "abc-my_photo__1_.png"
        );
        assert_eq!(media_filename("abc", "photo.jpg"), "abc-photo.jpg");
        assert_eq!(media_filename("abc", "..."), "abc");
    }

    #[test]
    fn links_between_documents_become_file_paths() {
        assert_eq!(
            relative_link("uni/proj/readme.md", "uni/proj/sub/deep.md"),
            "sub/deep.md"
        );
        assert_eq!(
            relative_link("uni/proj/sub/deep.md", "uni/proj/readme.md"),
            "../readme.md"
        );
        assert_eq!(relative_link("work/a.md", "uni/b.md"), "../uni/b.md");
        assert_eq!(relative_link("work/a.md", "work/b.md"), "b.md");
    }
}
