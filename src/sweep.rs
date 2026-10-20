//! Housekeeping for `uploads/`: what a crash, a killed upload or a forgotten
//! embed leaves behind. Nothing else ever deletes these, so without this they
//! would only grow — on the disk and in every backup of it.

use std::{collections::HashSet, path::Path, time::Duration};

use sqlx::SqlitePool;

use crate::routes::media;

/// A file this much younger may still be on its way in: an upload streaming
/// to `.part`, an import whose rows are about to be written.
const IN_FLIGHT: Duration = Duration::from_secs(60 * 60);

/// How long an upload that nothing embeds is kept. Long enough to paste an
/// image, delete the line by mistake, and put it back days later.
const UNREFERENCED_DAYS: i64 = 7;

const EVERY: Duration = Duration::from_secs(24 * 60 * 60);

pub fn spawn(db: SqlitePool, uploads: std::path::PathBuf) {
    crate::tasks::supervise("the uploads sweep", move || {
        let (db, uploads) = (db.clone(), uploads.clone());
        sweep_forever(db, uploads)
    });
}

async fn sweep_forever(db: SqlitePool, uploads: std::path::PathBuf) {
    {
        let mut ticker = tokio::time::interval(EVERY);
        loop {
            ticker.tick().await;
            match run(&db, &uploads).await {
                Ok(swept) if swept.any() => tracing::info!(
                    "swept uploads: {} partial, {} without a row, {} that nothing embeds",
                    swept.partial,
                    swept.stray,
                    swept.unreferenced
                ),
                Ok(_) => {}
                Err(e) => tracing::warn!("could not sweep uploads: {e:#}"),
            }
        }
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct Swept {
    pub partial: usize,
    pub stray: usize,
    pub unreferenced: usize,
}

impl Swept {
    fn any(&self) -> bool {
        self.partial + self.stray + self.unreferenced > 0
    }
}

pub async fn run(db: &SqlitePool, uploads: &Path) -> anyhow::Result<Swept> {
    let mut swept = Swept::default();

    sqlx::query("DELETE FROM sessions WHERE expires_at < ?1")
        .bind(crate::now())
        .execute(db)
        .await?;

    // Rows first, so the files they leave are strays by the time the
    // directory is walked, and go in the same pass.
    let cutoff = crate::now() - UNREFERENCED_DAYS * 86_400;
    let unreferenced: Vec<String> = sqlx::query_scalar(
        "SELECT m.id FROM media m
         WHERE m.created_at < ?1
           AND NOT EXISTS (SELECT 1 FROM node_media nm WHERE nm.media_id = m.id)
           AND NOT EXISTS (SELECT 1 FROM nodes n WHERE n.media_id = m.id)
           -- Belt and braces: a body or a card that names the file keeps it,
           -- whatever the link table says.
           AND NOT EXISTS (SELECT 1 FROM nodes n WHERE instr(n.body, m.id) > 0)
           AND NOT EXISTS (SELECT 1 FROM cards c WHERE instr(c.body, m.id) > 0)",
    )
    .bind(cutoff)
    .fetch_all(db)
    .await?;
    for id in &unreferenced {
        sqlx::query("DELETE FROM media WHERE id = ?1")
            .bind(id)
            .execute(db)
            .await?;
    }
    swept.unreferenced = unreferenced.len();

    let known: HashSet<String> = sqlx::query_scalar("SELECT id FROM media")
        .fetch_all(db)
        .await?
        .into_iter()
        .collect();

    let mut entries = tokio::fs::read_dir(uploads).await?;
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name().to_string_lossy().into_owned();
        let meta = entry.metadata().await?;
        if !meta.is_file() {
            continue;
        }
        let age = meta.modified()?.elapsed().unwrap_or_default();
        if age < IN_FLIGHT {
            continue;
        }
        let kind = if name.ends_with(".part") || name.starts_with(".import-") {
            &mut swept.partial
        } else if media::canonical_id(&name).as_deref() == Some(name.as_str())
            && !known.contains(&name)
        {
            &mut swept.stray
        } else {
            // Not ours to judge: anything that is not a uuid or a leftover of
            // one was put there by someone else.
            continue;
        };
        match tokio::fs::remove_file(entry.path()).await {
            Ok(()) => *kind += 1,
            Err(e) => tracing::warn!("could not remove {name} from uploads: {e}"),
        }
    }

    Ok(swept)
}

#[cfg(test)]
mod tests {
    use super::{run, Swept};

    #[tokio::test]
    async fn only_what_nothing_knows_about_goes() {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&db).await.unwrap();
        let dir = tempfile::tempdir().unwrap();

        let old = crate::now() - 30 * 86_400;
        let kept = "11111111-1111-4111-8111-111111111111";
        let embedded = "22222222-2222-4222-8222-222222222222";
        let forgotten = "33333333-3333-4333-8333-333333333333";
        let stray = "44444444-4444-4444-8444-444444444444";
        for id in [kept, embedded, forgotten] {
            sqlx::query("INSERT INTO media (id, filename, mime, size, created_at) VALUES (?1, 'x', 'image/png', 1, ?2)")
                .bind(id)
                .bind(old)
                .execute(&db)
                .await
                .unwrap();
        }
        let space: i64 = sqlx::query_scalar("SELECT id FROM nodes WHERE role = 'diary'")
            .fetch_one(&db)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO nodes (parent_id, kind, name, slug, body, created_at, updated_at)
             VALUES (?1, 'document', 'a', 'a', ?2, 0, 0)",
        )
        .bind(space)
        .bind(format!("![](/api/media/{embedded})"))
        .execute(&db)
        .await
        .unwrap();
        // `kept` is linked the way a saved document links what it embeds.
        sqlx::query(
            "INSERT INTO node_media (node_id, media_id) SELECT id, ?1 FROM nodes WHERE slug = 'a'",
        )
        .bind(kept)
        .execute(&db)
        .await
        .unwrap();

        let hour_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 60 * 60);
        for name in [kept, embedded, forgotten, stray, "x.part", "notes.txt"] {
            let path = dir.path().join(name);
            std::fs::write(&path, b"x").unwrap();
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(hour_ago)
                .unwrap();
        }
        let fresh = "55555555-5555-4555-8555-555555555555.part";
        std::fs::write(dir.path().join(fresh), b"x").unwrap();

        let swept = run(&db, dir.path()).await.unwrap();
        assert_eq!(
            swept,
            Swept {
                partial: 1,
                stray: 2,
                unreferenced: 1
            }
        );

        let left: std::collections::BTreeSet<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        let expected: std::collections::BTreeSet<String> = [kept, embedded, "notes.txt", fresh]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(left, expected);
    }
}
