//! The diary as a diary rather than a list: the days that have an entry, the
//! entry before and after the one being read, what was written on this day in
//! earlier years — and templates, which every space has, but which the diary
//! is the first to want.
//!
//! A day is a local day in `WORKSPACE_TIMEZONE`, the same one the evening
//! reminder asks about, so the calendar and the mail never disagree.

use axum::{
    extract::{Path, Query, State},
    Json,
};
use chrono::{Datelike, Months, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqliteConnection, SqlitePool};

use crate::{
    auth::Session,
    error::{AppError, AppResult},
    notify::{day_bounds, local_day, local_instant, weekday_name},
    routes::{
        links, media,
        nodes::{excerpt, path_of, slugify, unique_slug, HEAD_CHARS},
    },
    state::AppState,
};

/// The folder at the root of a space that holds its templates. Found by name,
/// as a person would look for it: `role` is for the one diary and the trash.
pub const TEMPLATES: &str = "templates";

/// The template a new diary entry starts from, when the diary has one.
pub const DAY: &str = "day";

/// The diary's entries: every document in the diary space except the
/// templates in its `templates` folder, which are about no day at all.
/// `WITH RECURSIVE {ENTRIES} SELECT … WHERE kind = 'document' AND id IN
/// (SELECT id FROM entries)`.
pub(crate) const ENTRIES: &str = "entries (id) AS (
         SELECT id FROM nodes WHERE role = 'diary'
         UNION ALL
         SELECT n.id FROM nodes n JOIN entries e ON n.parent_id = e.id
         WHERE NOT (n.kind = 'folder' AND n.slug = 'templates'
                    AND n.parent_id = (SELECT id FROM nodes WHERE role = 'diary'))
     )";

#[derive(Serialize)]
pub struct Template {
    pub id: i64,
    pub name: String,
    pub slug: String,
}

/// `{{date}}`, `{{weekday}}`, `{{time}}` and `{{title}}` in a template, for a
/// document dated `at`. The title goes in last, so a title that happens to
/// contain a placeholder is left as it was typed.
pub(crate) fn fill(text: &str, tz: Tz, at: i64, title: &str) -> String {
    let local = Utc
        .timestamp_opt(at, 0)
        .single()
        .unwrap_or_else(Utc::now)
        .with_timezone(&tz);
    text.replace("{{date}}", &local.format("%Y-%m-%d").to_string())
        .replace("{{weekday}}", weekday_name(local.weekday()))
        .replace("{{time}}", &local.format("%H:%M").to_string())
        .replace("{{title}}", title)
}

/// The templates of the space `space_id`, by name.
pub(crate) async fn templates_of(
    conn: &mut SqliteConnection,
    space_id: i64,
) -> AppResult<Vec<Template>> {
    let rows = sqlx::query(
        "SELECT t.id, t.name, t.slug FROM nodes t
         JOIN nodes f ON f.id = t.parent_id
         WHERE f.parent_id = ?1 AND f.kind = 'folder' AND f.slug = ?2 AND t.kind = 'document'
         ORDER BY t.slug",
    )
    .bind(space_id)
    .bind(TEMPLATES)
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .iter()
        .map(|row| Template {
            id: row.get("id"),
            name: row.get("name"),
            slug: row.get("slug"),
        })
        .collect())
}

/// The body of the template `id`, filled in for a document called `title`
/// dated `at`.
pub(crate) async fn from_template(
    conn: &mut SqliteConnection,
    tz: Tz,
    id: i64,
    at: i64,
    title: &str,
) -> AppResult<String> {
    let body: Option<String> =
        sqlx::query_scalar("SELECT body FROM nodes WHERE id = ?1 AND kind = 'document'")
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?;
    let body = body.ok_or_else(|| AppError::BadRequest("a template is a document".into()))?;
    Ok(fill(&body, tz, at, title))
}

/// The diary's `templates/day`, filled in for `at`, when there is one.
pub(crate) async fn day_template(
    conn: &mut SqliteConnection,
    tz: Tz,
    diary: i64,
    at: i64,
) -> AppResult<Option<String>> {
    let day = templates_of(conn, diary)
        .await?
        .into_iter()
        .find(|template| template.slug == DAY);
    match day {
        Some(template) => Ok(Some(from_template(conn, tz, template.id, at, "").await?)),
        None => Ok(None),
    }
}

/// `GET /api/nodes/{id}/templates`: the templates of the space `id` is in.
pub async fn templates(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<Vec<Template>>> {
    let path = path_of(&state.db, id).await?;
    let space = path.first().ok_or(AppError::NotFound)?;
    let mut conn = state.db.acquire().await?;
    templates_of(&mut conn, space.id).await.map(Json)
}

#[derive(Deserialize)]
pub struct DaysQuery {
    /// `2026-10`.
    pub month: String,
}

/// An entry in the diary, as the calendar and the agenda show it.
#[derive(Serialize, Debug)]
pub struct Entry {
    pub id: i64,
    pub name: String,
    /// The local day it is about: `2026-10-20`.
    pub day: String,
    pub created_at: i64,
    /// `diary/untitled`, to open it by.
    pub path: String,
    pub excerpt: String,
}

async fn entries_between(db: &SqlitePool, tz: Tz, start: i64, end: i64) -> AppResult<Vec<Entry>> {
    let rows = sqlx::query(&format!(
        "WITH RECURSIVE {ENTRIES}
         SELECT id, name, created_at, substr(body, 1, {HEAD_CHARS}) AS head FROM nodes
         WHERE kind = 'document' AND id IN (SELECT id FROM entries)
           AND created_at >= ?1 AND created_at < ?2
         ORDER BY created_at, id"
    ))
    .bind(start)
    .bind(end)
    .fetch_all(db)
    .await?;
    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        let id: i64 = row.get("id");
        let created_at: i64 = row.get("created_at");
        entries.push(Entry {
            id,
            name: row.get("name"),
            day: local_day(tz, created_at).format("%Y-%m-%d").to_string(),
            created_at,
            path: slug_path(db, id).await?,
            excerpt: excerpt(row.get("head")),
        });
    }
    Ok(entries)
}

async fn slug_path(db: &SqlitePool, id: i64) -> AppResult<String> {
    Ok(path_of(db, id)
        .await?
        .iter()
        .map(|crumb| crumb.slug.as_str())
        .collect::<Vec<_>>()
        .join("/"))
}

/// The entries of one month, from the first of it to the first of the next,
/// both in the configured timezone.
async fn month_entries(db: &SqlitePool, tz: Tz, month: &str) -> AppResult<Vec<Entry>> {
    let first = NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d")
        .map_err(|_| AppError::BadRequest(format!("not a month: {month}, but 2026-10 is")))?;
    let next = first
        .checked_add_months(Months::new(1))
        .ok_or_else(|| AppError::BadRequest(format!("not a month: {month}")))?;
    entries_between(db, tz, day_bounds(tz, first).0, day_bounds(tz, next).0).await
}

/// `GET /api/diary/days?month=2026-10`: the entries of a month, for the
/// calendar to mark its days with.
pub async fn days(
    _: Session,
    State(state): State<AppState>,
    Query(query): Query<DaysQuery>,
) -> AppResult<Json<Vec<Entry>>> {
    month_entries(&state.db, state.config.mail.timezone, &query.month)
        .await
        .map(Json)
}

#[derive(Serialize, Debug, Default)]
pub struct Around {
    pub prev: Option<Entry>,
    pub next: Option<Entry>,
}

/// The entries either side of `id` by date: `[d` and `]d`.
async fn around(db: &SqlitePool, tz: Tz, id: i64) -> AppResult<Around> {
    let at: Option<i64> = sqlx::query_scalar(&format!(
        "WITH RECURSIVE {ENTRIES}
         SELECT created_at FROM nodes
         WHERE id = ?1 AND kind = 'document' AND id IN (SELECT id FROM entries)"
    ))
    .bind(id)
    .fetch_optional(db)
    .await?;
    let at = at.ok_or_else(|| AppError::BadRequest("this is not a diary entry".into()))?;

    let step = |before: bool| {
        let (compare, order) = if before { ("<", "DESC") } else { (">", "ASC") };
        format!(
            "WITH RECURSIVE {ENTRIES}
             SELECT id FROM nodes
             WHERE kind = 'document' AND id IN (SELECT id FROM entries)
               AND (created_at {compare} ?1 OR (created_at = ?1 AND id {compare} ?2))
             ORDER BY created_at {order}, id {order}
             LIMIT 1"
        )
    };
    let mut found = Around::default();
    for before in [true, false] {
        let next: Option<i64> = sqlx::query_scalar(&step(before))
            .bind(at)
            .bind(id)
            .fetch_optional(db)
            .await?;
        let Some(next) = next else { continue };
        let created_at: i64 = sqlx::query_scalar("SELECT created_at FROM nodes WHERE id = ?1")
            .bind(next)
            .fetch_one(db)
            .await?;
        let entry = entries_between(db, tz, created_at, created_at + 1)
            .await?
            .into_iter()
            .find(|entry| entry.id == next);
        if before {
            found.prev = entry;
        } else {
            found.next = entry;
        }
    }
    Ok(found)
}

/// `GET /api/diary/around/{id}`.
pub async fn around_entry(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<Around>> {
    around(&state.db, state.config.mail.timezone, id)
        .await
        .map(Json)
}

/// What was written on this day in earlier years, the latest year first.
/// 29 February comes round only every fourth year; in the others it is
/// remembered on the 28th, so a leap day is not skipped for three years.
pub(crate) async fn on_this_day(db: &SqlitePool, tz: Tz, now: i64) -> AppResult<Vec<Entry>> {
    let first: Option<i64> = sqlx::query_scalar(&format!(
        "WITH RECURSIVE {ENTRIES}
         SELECT MIN(created_at) FROM nodes
         WHERE kind = 'document' AND id IN (SELECT id FROM entries)"
    ))
    .fetch_one(db)
    .await?;
    let Some(first) = first else {
        return Ok(Vec::new());
    };

    let today = local_day(tz, now);
    let leap = NaiveDate::from_ymd_opt(today.year(), 2, 29).is_some();
    let late_leap_day = !leap && today.month() == 2 && today.day() == 28;
    let mut found = Vec::new();
    for year in (local_day(tz, first).year()..today.year()).rev() {
        let mut days = vec![NaiveDate::from_ymd_opt(year, today.month(), today.day())];
        if late_leap_day {
            days.push(NaiveDate::from_ymd_opt(year, 2, 29));
        }
        for day in days.into_iter().flatten() {
            let (start, end) = day_bounds(tz, day);
            found.extend(entries_between(db, tz, start, end).await?);
        }
    }
    Ok(found)
}

/// Whether `id` is the diary space itself, where a new document is an entry.
pub(crate) async fn is_diary(conn: &mut SqliteConnection, id: i64) -> AppResult<bool> {
    let role: Option<Option<String>> = sqlx::query_scalar("SELECT role FROM nodes WHERE id = ?1")
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
    Ok(role.flatten().as_deref() == Some(crate::routes::nodes::DIARY))
}

/// The entry about the local day `day`: the first written, wherever in the
/// diary it was filed. With none, a new one at the root of the diary, from
/// `templates/day` when there is one, dated `at` — or, for another day than
/// the one `at` falls on, noon of it. `T`, the calendar and a capture all ask
/// this, so they find and start the same entry, by the server's clock.
pub(crate) async fn entry_of(
    conn: &mut SqliteConnection,
    tz: Tz,
    day: NaiveDate,
    at: i64,
) -> AppResult<Day> {
    let diary: i64 = sqlx::query_scalar("SELECT id FROM nodes WHERE role = ?1")
        .bind(crate::routes::nodes::DIARY)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or_else(|| AppError::BadRequest("there is no diary to write in".into()))?;
    let (start, end) = day_bounds(tz, day);
    let found: Option<(i64, String)> = sqlx::query_as(&format!(
        "WITH RECURSIVE {ENTRIES}
         SELECT id, body FROM nodes
         WHERE kind = 'document' AND id IN (SELECT id FROM entries)
           AND created_at >= ?1 AND created_at < ?2
         ORDER BY created_at, id
         LIMIT 1"
    ))
    .bind(start)
    .bind(end)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some((id, body)) = found {
        return Ok(Day {
            id,
            body,
            created: false,
        });
    }

    let at = if local_day(tz, at) == day {
        at
    } else {
        local_instant(tz, day, 12, 0)
    };
    let body = day_template(conn, tz, diary, at).await?.unwrap_or_default();
    let slug = unique_slug(&mut *conn, Some(diary), &slugify(""), None).await?;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO nodes (parent_id, kind, name, slug, body, created_at, updated_at)
         VALUES (?1, 'document', '', ?2, ?3, ?4, ?5)
         RETURNING id",
    )
    .bind(diary)
    .bind(&slug)
    .bind(&body)
    .bind(at)
    .bind(crate::now())
    .fetch_one(&mut *conn)
    .await?;
    media::link_to_node(conn, id, &body).await?;
    links::index(conn, id, &body).await?;
    Ok(Day {
        id,
        body,
        created: true,
    })
}

/// A day's entry, and whether asking for it is what started it.
#[derive(Serialize, Debug)]
pub struct Day {
    pub id: i64,
    #[serde(skip)]
    pub body: String,
    pub created: bool,
}

#[derive(Deserialize)]
pub struct DayInput {
    /// `2026-10-22`; today, in `WORKSPACE_TIMEZONE`, without it.
    #[serde(default)]
    pub day: Option<String>,
}

/// `POST /api/diary/day`: the entry of a day, started when there is none.
pub async fn day(
    _: Session,
    State(state): State<AppState>,
    Json(input): Json<DayInput>,
) -> AppResult<Json<Day>> {
    let tz = state.config.mail.timezone;
    let now = crate::now();
    let day = match input.day.as_deref() {
        Some(day) => NaiveDate::parse_from_str(day, "%Y-%m-%d")
            .map_err(|_| AppError::BadRequest(format!("not a day: {day}, but 2026-10-22 is")))?,
        None => local_day(tz, now),
    };
    let mut tx = crate::db::begin_write(&state.db).await?;
    let entry = entry_of(&mut tx, tz, day, now).await?;
    tx.commit().await?;
    if entry.created {
        state.backup.signal();
    }
    Ok(Json(entry))
}

#[cfg(test)]
mod tests {
    use axum::{extract::State, Json};
    use chrono::TimeZone;
    use chrono_tz::Europe::Berlin;
    use sqlx::SqlitePool;

    use super::{around, entry_of, fill, month_entries, on_this_day};
    use crate::{auth::Session, routes::nodes::CreateInput, state::AppState};

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        Berlin
            .with_ymd_and_hms(y, mo, d, h, mi, 0)
            .earliest()
            .unwrap()
            .timestamp()
    }

    async fn diary(db: &SqlitePool) -> i64 {
        sqlx::query_scalar("SELECT id FROM nodes WHERE role = 'diary'")
            .fetch_one(db)
            .await
            .unwrap()
    }

    async fn add(db: &SqlitePool, parent: i64, kind: &str, name: &str, body: &str, at: i64) -> i64 {
        let slug = crate::routes::nodes::slugify(name);
        let slug = if slug == "untitled" {
            format!("e{at}")
        } else {
            slug
        };
        sqlx::query_scalar(
            "INSERT INTO nodes (parent_id, kind, name, slug, body, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6) RETURNING id",
        )
        .bind(parent)
        .bind(kind)
        .bind(name)
        .bind(slug)
        .bind(body)
        .bind(at)
        .fetch_one(db)
        .await
        .unwrap()
    }

    async fn create(state: &AppState, input: CreateInput) -> String {
        let Json(node) = crate::routes::nodes::create(
            Session { id: "test".into() },
            State(state.clone()),
            Json(input),
        )
        .await
        .unwrap();
        node.body
    }

    fn input(parent_id: i64, name: &str, created_at: i64, template_id: Option<i64>) -> CreateInput {
        CreateInput {
            parent_id,
            kind: None,
            name: name.into(),
            body: String::new(),
            created_at: Some(created_at),
            template_id,
        }
    }

    #[test]
    fn placeholders_keep_to_the_wall_clock_across_daylight_saving() {
        let text = "{{weekday}} {{date}} {{time}} {{title}}";
        // Spring: 01:59 is followed by 03:00.
        assert_eq!(
            fill(text, Berlin, at(2026, 3, 29, 1, 30), "a"),
            "Sunday 2026-03-29 01:30 a"
        );
        assert_eq!(
            fill(text, Berlin, at(2026, 3, 29, 1, 30) + 3600, "a"),
            "Sunday 2026-03-29 03:30 a"
        );
        // Autumn: 02:30 happens twice, an hour apart.
        let first = at(2026, 10, 25, 2, 30);
        assert_eq!(fill("{{time}}", Berlin, first, ""), "02:30");
        assert_eq!(fill("{{time}}", Berlin, first + 3600, ""), "02:30");
        // Before midnight in UTC, after it here.
        assert_eq!(
            fill(text, Berlin, at(2026, 10, 21, 0, 30), "{{date}}"),
            "Wednesday 2026-10-21 00:30 {{date}}"
        );
    }

    #[tokio::test]
    async fn a_new_document_starts_from_its_template_and_an_entry_from_the_day() {
        let state = crate::routes::tests::state_in(".".into()).await;
        let db = &state.db;
        let diary = diary(db).await;
        let work: i64 = sqlx::query_scalar("SELECT id FROM nodes WHERE slug = 'work'")
            .fetch_one(db)
            .await
            .unwrap();
        let morning = at(2026, 10, 20, 8, 15);

        // No day template yet: an entry starts empty, as it always has.
        assert_eq!(create(&state, input(diary, "", morning, None)).await, "");

        let folder = add(db, work, "folder", "templates", "", 0).await;
        let meeting = add(
            db,
            folder,
            "document",
            "meeting",
            "# {{title}}\n{{weekday}} {{date}}\n",
            0,
        )
        .await;
        let body = create(&state, input(work, "Budget", morning, Some(meeting))).await;
        assert_eq!(body, "# Budget\nTuesday 2026-10-20\n");

        let folder = add(db, diary, "folder", "templates", "", morning).await;
        add(db, folder, "document", "day", "## {{weekday}}\n", morning).await;
        let body = create(&state, input(diary, "", at(2026, 10, 21, 9, 0), None)).await;
        assert_eq!(body, "## Wednesday\n");
        // The template is not an entry of the day it was written on.
        let days = month_entries(db, Berlin, "2026-10").await.unwrap();
        assert_eq!(days.len(), 2, "{days:?}");

        // Capture that starts a day starts it from the template too.
        let captured =
            crate::routes::capture::tests::diary_at(db, "rain", at(2026, 10, 22, 7, 0)).await;
        let body: String = sqlx::query_scalar("SELECT body FROM nodes WHERE id = ?1")
            .bind(captured)
            .fetch_one(db)
            .await
            .unwrap();
        assert_eq!(body, "## Thursday\n\n- 07:00 rain\n");
    }

    #[tokio::test]
    async fn a_day_has_one_entry_found_by_the_servers_clock() {
        let state = crate::routes::tests::state_in(".".into()).await;
        let db = &state.db;
        let diary = diary(db).await;
        // 23:30 in UTC on the 21st is already the 22nd here.
        let now = at(2026, 10, 22, 1, 30);
        let day = |d| chrono::NaiveDate::from_ymd_opt(2026, 10, d).unwrap();

        // An entry filed in a folder is still the day's.
        let folder = add(db, diary, "folder", "october", "", 0).await;
        let filed = add(db, folder, "document", "", "", at(2026, 10, 22, 0, 10)).await;
        let templates = add(db, diary, "folder", "templates", "", 0).await;
        add(db, templates, "document", "day", "{{weekday}}\n", 0).await;
        let mut conn = db.acquire().await.unwrap();
        let found = entry_of(&mut conn, Berlin, day(22), now).await.unwrap();
        assert_eq!((found.id, found.created), (filed, false));

        // A day without one gets one at the root, at noon, from the template.
        let started = entry_of(&mut conn, Berlin, day(20), now).await.unwrap();
        assert!(started.created);
        assert_eq!(started.body, "Tuesday\n");
        let (parent, created_at): (i64, i64) =
            sqlx::query_as("SELECT parent_id, created_at FROM nodes WHERE id = ?1")
                .bind(started.id)
                .fetch_one(&mut *conn)
                .await
                .unwrap();
        assert_eq!((parent, created_at), (diary, at(2026, 10, 20, 12, 0)));
        // Asked again, it is found rather than started twice.
        let again = entry_of(&mut conn, Berlin, day(20), now).await.unwrap();
        assert_eq!((again.id, again.created), (started.id, false));

        // Today's is dated now, and a capture writes into the same one.
        let today = entry_of(&mut conn, Berlin, day(21), at(2026, 10, 21, 9, 0))
            .await
            .unwrap();
        drop(conn);
        let captured =
            crate::routes::capture::tests::diary_at(db, "rain", at(2026, 10, 21, 18, 0)).await;
        assert_eq!(captured, today.id);
    }

    #[tokio::test]
    async fn a_month_holds_the_days_of_the_timezone() {
        let state = crate::routes::tests::state_in(".".into()).await;
        let db = &state.db;
        let diary = diary(db).await;
        // An hour apart in UTC, on 31 October both, but a month apart here.
        let october = add(db, diary, "document", "", "", at(2026, 10, 31, 23, 30)).await;
        let november = add(db, diary, "document", "", "", at(2026, 11, 1, 0, 30)).await;

        let days = month_entries(db, Berlin, "2026-10").await.unwrap();
        assert_eq!(days.iter().map(|e| e.id).collect::<Vec<_>>(), [october]);
        assert_eq!(days[0].day, "2026-10-31");
        let days = month_entries(db, Berlin, "2026-11").await.unwrap();
        assert_eq!(days.iter().map(|e| e.id).collect::<Vec<_>>(), [november]);
        assert_eq!(days[0].day, "2026-11-01");
        assert!(days[0].path.starts_with("diary/"), "{}", days[0].path);
        // December rolls over into the next year.
        assert!(month_entries(db, Berlin, "2026-12")
            .await
            .unwrap()
            .is_empty());
        for month in ["2026-13", "october", ""] {
            assert!(month_entries(db, Berlin, month).await.is_err(), "{month}");
        }
    }

    #[tokio::test]
    async fn the_entries_either_side_go_by_date() {
        let state = crate::routes::tests::state_in(".".into()).await;
        let db = &state.db;
        let diary = diary(db).await;
        let folder = add(db, diary, "folder", "2025", "", 0).await;
        let older = add(db, folder, "document", "", "", at(2025, 12, 31, 12, 0)).await;
        let middle = add(db, diary, "document", "", "", at(2026, 1, 2, 12, 0)).await;
        // Written later, about an earlier day.
        let backdated = add(db, diary, "document", "", "", at(2026, 1, 1, 12, 0)).await;

        let found = around(db, Berlin, backdated).await.unwrap();
        assert_eq!(found.prev.map(|e| e.id), Some(older));
        assert_eq!(found.next.map(|e| e.id), Some(middle));
        let found = around(db, Berlin, older).await.unwrap();
        assert!(found.prev.is_none());
        assert!(around(db, Berlin, middle).await.unwrap().next.is_none());
        assert!(around(db, Berlin, folder).await.is_err());
    }

    #[tokio::test]
    async fn on_this_day_remembers_the_leap_day_in_the_years_without_one() {
        let state = crate::routes::tests::state_in(".".into()).await;
        let db = &state.db;
        let diary = diary(db).await;
        let leap = add(db, diary, "document", "", "", at(2024, 2, 29, 20, 0)).await;
        let plain = add(db, diary, "document", "", "", at(2025, 2, 28, 20, 0)).await;
        add(db, diary, "document", "", "", at(2025, 3, 1, 20, 0)).await;
        let today = add(db, diary, "document", "", "", at(2027, 2, 28, 8, 0)).await;

        let ids = |found: Vec<super::Entry>| found.into_iter().map(|e| e.id).collect::<Vec<_>>();
        // 2027 has no 29th, so its 28th remembers both, the latest year first;
        // its own entry is today's, not a memory.
        assert_eq!(
            ids(on_this_day(db, Berlin, at(2027, 2, 28, 9, 0))
                .await
                .unwrap()),
            [plain, leap]
        );
        // A leap year has a 29th of its own to remember the leap day on.
        assert_eq!(
            ids(on_this_day(db, Berlin, at(2028, 2, 28, 9, 0))
                .await
                .unwrap()),
            [today, plain]
        );
        assert_eq!(
            ids(on_this_day(db, Berlin, at(2028, 2, 29, 9, 0))
                .await
                .unwrap()),
            [leap]
        );
    }
}
