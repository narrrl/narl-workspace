use std::{
    collections::HashMap,
    io::{Seek, SeekFrom, Write},
};

use axum::{
    body::Body,
    extract::State,
    http::header,
    response::{IntoResponse, Response},
};
use sqlx::Row;
use tokio_util::io::ReaderStream;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

use crate::{
    auth::Session,
    error::{AppError, AppResult},
    state::AppState,
};

/// One node, flattened out of the database ahead of the blocking zip work.
struct ExportNode {
    id: i64,
    parent_id: Option<i64>,
    kind: String,
    name: String,
    slug: String,
    body: String,
    media_id: Option<String>,
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
        let Some(ancestor) = nodes.get(&id) else { break };
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
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
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
    let name = if safe.is_empty() { node.slug.clone() } else { safe };
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

  <space>/   one directory per space, holding the same tree of folders,
             documents and files that the workspace shows; one markdown file
             per document, and every file under its own name
  media/     the files documents embed that are not in the tree themselves

Embedded media, and links from one document to another, are relative — so the
documents render and cross-link correctly in any markdown reader as long as the
tree stays intact. Nothing here needs the workspace application to read it.
";

/// Everything, as one zip. A workspace you cannot get out of is not one you
/// can trust, so this is deliberately a plain archive of plain files.
pub async fn export(_: Session, State(state): State<AppState>) -> AppResult<Response> {
    let nodes: HashMap<i64, ExportNode> = sqlx::query(
        "SELECT id, parent_id, kind, name, slug, body, media_id FROM nodes ORDER BY created_at, id",
    )
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
    let sealed: std::collections::HashSet<String> = rows
        .iter()
        .filter(|row| row.get::<bool, _>("encrypted"))
        .map(|row| row.get("id"))
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

    // Zip writing is synchronous and reads every upload off disk, so it happens
    // on the blocking pool, into a temporary file rather than into memory: a
    // diary with photographs in it does not fit comfortably in a Vec.
    let file = tokio::task::spawn_blocking(move || -> anyhow::Result<std::fs::File> {
        let mut zip = ZipWriter::new(tempfile::tempfile()?);
        let text = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        // Media is overwhelmingly already-compressed; deflating it again costs
        // a lot of CPU to save nothing.
        let blob = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

        zip.start_file("README.txt", text)?;
        zip.write_all(README.as_bytes())?;

        // A file node's bytes go where the node is; the rest go to media/.
        // Ordered by id, so a clash of names resolves the same way every time.
        let mut files: Vec<&ExportNode> = nodes.values().filter(|n| n.media_id.is_some()).collect();
        files.sort_by_key(|n| n.id);
        let mut taken = std::collections::HashSet::new();
        let in_tree: HashMap<String, String> = files
            .into_iter()
            .filter_map(|n| Some((n.media_id.clone()?, file_path(&nodes, n, &mut taken))))
            .collect();

        let names: Vec<(String, String)> = media
            .iter()
            .filter(|(id, _)| !in_tree.contains_key(id))
            .map(|(id, filename)| (id.clone(), media_filename(id, filename)))
            .collect();

        // Every document by the address the application knows it as, so a link
        // between two of them can be found and pointed at the file instead.
        let addresses: HashMap<String, String> = nodes
            .values()
            .filter(|n| n.kind == "document")
            .map(|n| {
                let path = document_path(&nodes, n);
                (format!("/n/{}", path.trim_end_matches(".md")), path)
            })
            .collect();

        for node in nodes.values().filter(|n| n.kind == "document") {
            let path = document_path(&nodes, node);
            // A document three directories deep needs three steps back up to
            // reach the media folder next to the spaces.
            let up = "../".repeat(path.matches('/').count());

            let mut body = node.body.clone();
            for (id, name) in &names {
                body = body.replace(&format!("/api/media/{id}"), &format!("{up}media/{name}"));
            }
            for (id, target) in &in_tree {
                body = body.replace(&format!("/api/media/{id}"), &relative_link(&path, target));
            }
            for (address, target) in &addresses {
                // Matching the `)` or `#` that ends a markdown link is what
                // keeps `/n/work/a` from matching inside `/n/work/ab`.
                body = body.replace(&format!("]({address})"), &format!("]({})", relative_link(&path, target)));
                body = body.replace(&format!("]({address}#"), &format!("]({}#", relative_link(&path, target)));
            }
            let document = if node.name.trim().is_empty() {
                body
            } else {
                format!("# {}\n\n{body}", node.name.trim())
            };

            zip.start_file(path, text)?;
            zip.write_all(document.as_bytes())?;
        }

        let entries = names
            .iter()
            .map(|(id, name)| (id, format!("media/{name}")))
            .chain(in_tree.iter().map(|(id, path)| (id, path.clone())));
        for (id, entry) in entries {
            // A row whose file has gone missing must not fail the whole backup.
            let path = uploads.join(id);
            let source: Box<dyn std::io::Read> = match (&key, sealed.contains(id)) {
                (Some(key), true) => match crate::vault::Reader::open(key, id, &path) {
                    Ok(reader) => Box::new(reader),
                    Err(_) => continue,
                },
                _ => match std::fs::File::open(&path) {
                    Ok(file) => Box::new(file),
                    Err(_) => continue,
                },
            };
            zip.start_file(entry, blob)?;
            std::io::copy(&mut { source }, &mut zip)?;
        }

        let mut file = zip.finish()?;
        file.seek(SeekFrom::Start(0))?;
        Ok(file)
    })
    .await??;

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
        assert_eq!(file_path(&nodes, &nodes[&3], &mut taken), "uni/algo/Slides_3.pdf");
        assert_eq!(file_path(&nodes, &nodes[&4], &mut taken), "uni/algo/4-Slides_3.pdf");
    }

    #[test]
    fn a_document_keeps_its_place_in_the_tree() {
        let nodes = tree(&[
            (1, None, "space", "work"),
            (2, Some(1), "folder", "einarbeiten"),
            (3, Some(2), "document", "01-was-ist-eebus"),
            (4, Some(1), "document", "notes"),
        ]);
        assert_eq!(document_path(&nodes, &nodes[&3]), "work/einarbeiten/01-was-ist-eebus.md");
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
        assert_eq!(media_filename("abc", "my photo (1).png"), "abc-my_photo__1_.png");
        assert_eq!(media_filename("abc", "photo.jpg"), "abc-photo.jpg");
        assert_eq!(media_filename("abc", "..."), "abc");
    }

    #[test]
    fn links_between_documents_become_file_paths() {
        assert_eq!(relative_link("uni/proj/readme.md", "uni/proj/sub/deep.md"), "sub/deep.md");
        assert_eq!(relative_link("uni/proj/sub/deep.md", "uni/proj/readme.md"), "../readme.md");
        assert_eq!(relative_link("work/a.md", "uni/b.md"), "../uni/b.md");
        assert_eq!(relative_link("work/a.md", "work/b.md"), "b.md");
    }
}
