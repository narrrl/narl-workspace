//! Kanban boards, one per space.
//!
//! The question a board answers here is "what am I working on in *this* part of
//! my life", so there is exactly one board per space and no board table: a
//! space with `has_board` set has a board, and the lists hang off the space
//! node directly. That keeps the deployment one binary and one SQLite file, and
//! the Proton mirror keeps covering everything without a second service.
//!
//! The columns carry meaning by position, not by name: the first list is the
//! inbox new cards land in, the last list is "done", and a card is finished
//! exactly when it sits there. Moving a card into the last list stamps
//! `done_at`, moving it out clears it, so a tick and a column can never
//! disagree about whether something is finished.
//!
//! Cards may point at a document. The link is deliberately weak — deleting the
//! document sets `node_id` to NULL rather than deleting the card, because the
//! work is still open once its notes are gone.

use axum::{
    extract::{Path, Query, State},
    Json,
};
use std::collections::{BTreeMap, HashMap};

use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqliteConnection, SqlitePool};

use crate::{
    auth::Session,
    error::{AppError, AppResult},
    routes::{
        links,
        manifest::ListMeta,
        nodes::{self, DOCUMENT, SPACE},
        repeat::Repeat,
        trash,
    },
    state::AppState,
};

/// What a board starts with. Three columns is the smallest arrangement that
/// still says something: not started, in hand, finished.
const DEFAULT_LISTS: [&str; 3] = ["backlog", "doing", "done"];

#[derive(Serialize)]
pub struct Board {
    pub space_id: i64,
    pub space_name: String,
    pub lists: Vec<BoardList>,
}

#[derive(Serialize)]
pub struct BoardList {
    pub id: i64,
    pub name: String,
    pub position: i64,
    pub cards: Vec<Card>,
}

#[derive(Serialize)]
pub struct Card {
    pub id: i64,
    pub list_id: i64,
    pub title: String,
    pub body: String,
    pub node_id: Option<i64>,
    /// The linked document's name, so a card can say what it points at without
    /// the client reading every document on the board.
    pub node_name: Option<String>,
    pub due_at: Option<i64>,
    pub done_at: Option<i64>,
    /// The rule it comes back by once finished, as `Repeat::name` writes it.
    pub repeat: Option<String>,
    pub checklist: Vec<Task>,
    pub labels: Vec<String>,
    pub position: i64,
    pub created_at: i64,
    pub updated_at: i64,
    /// Where each `[[title]]` in the description leads, as a document's do.
    pub wikilinks: BTreeMap<String, String>,
}

/// A step on a card's checklist.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Task {
    pub text: String,
    #[serde(default)]
    pub done: bool,
}

/// The most a card holds of each, so a runaway client cannot grow one row
/// without end.
const MAX_TASKS: usize = 200;
const MAX_LABELS: usize = 12;
const MAX_LABEL_LEN: usize = 32;

/// A checklist as it is kept: each step trimmed, the empty ones dropped.
fn tidy_checklist(tasks: Vec<Task>) -> AppResult<Vec<Task>> {
    let tasks: Vec<Task> = tasks
        .into_iter()
        .filter_map(|task| {
            let text = task.text.trim();
            (!text.is_empty()).then(|| Task {
                text: text.to_string(),
                done: task.done,
            })
        })
        .collect();
    if tasks.len() > MAX_TASKS {
        return Err(AppError::BadRequest(format!(
            "a checklist holds at most {MAX_TASKS} steps"
        )));
    }
    Ok(tasks)
}

/// Labels as they are kept: lower case, trimmed, each once, in the order given.
fn tidy_labels(labels: Vec<String>) -> AppResult<Vec<String>> {
    let mut kept: Vec<String> = Vec::new();
    for label in labels {
        let label = label.trim().to_lowercase();
        if label.is_empty() || kept.contains(&label) {
            continue;
        }
        if label.chars().count() > MAX_LABEL_LEN {
            return Err(AppError::BadRequest(format!(
                "a label is at most {MAX_LABEL_LEN} characters"
            )));
        }
        kept.push(label);
    }
    if kept.len() > MAX_LABELS {
        return Err(AppError::BadRequest(format!(
            "a card carries at most {MAX_LABELS} labels"
        )));
    }
    Ok(kept)
}

/// A JSON column read back, or empty when it does not parse: a card with a
/// mangled checklist still shows, which beats the whole board failing.
fn json_column<T: serde::de::DeserializeOwned + Default>(
    row: &sqlx::sqlite::SqliteRow,
    name: &str,
) -> T {
    serde_json::from_str(row.get::<&str, _>(name)).unwrap_or_default()
}

fn to_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("a checklist or label list serialises")
}

#[derive(Deserialize)]
pub struct ListInput {
    pub name: String,
}

#[derive(Deserialize)]
pub struct CardInput {
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// The document the card is about, if it has one.
    #[serde(default)]
    pub node_id: Option<i64>,
    #[serde(default)]
    pub due_at: Option<i64>,
}

/// Every field is optional: the client sends what changed. `null` and "absent"
/// are different for the two that can be cleared, so they are doubly wrapped —
/// `"due_at": null` drops the date, leaving it out keeps it.
#[derive(Deserialize)]
pub struct CardPatch {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub node_id: Option<Option<i64>>,
    #[serde(default, deserialize_with = "double_option")]
    pub due_at: Option<Option<i64>>,
    #[serde(default, deserialize_with = "double_option")]
    pub repeat: Option<Option<String>>,
    /// The whole checklist, as it should now read.
    #[serde(default)]
    pub checklist: Option<Vec<Task>>,
    #[serde(default)]
    pub labels: Option<Vec<String>>,
    pub done: Option<bool>,
}

/// What a move or an edit answers with: the card, and the one it left in the
/// inbox when it was a repeating card that has just been finished.
#[derive(Serialize)]
pub struct Moved {
    #[serde(flatten)]
    pub card: Card,
    pub again: Option<Card>,
}

fn double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    Option::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
pub struct MoveInput {
    pub list_id: i64,
    /// Where among the cards of that list it lands. Past the end means last.
    #[serde(default)]
    pub position: Option<i64>,
}

/// `ids` with `card` taken out and put back at `to`. Pulled out of the move
/// handler because off-by-one in a reorder is silent: the card lands next to
/// where it was asked to go and nobody notices until the board reads wrong.
fn reordered(ids: &[i64], card: i64, to: usize) -> Vec<i64> {
    let mut rest: Vec<i64> = ids.iter().copied().filter(|id| *id != card).collect();
    rest.insert(to.min(rest.len()), card);
    rest
}

/// How far ahead the agenda looks for due dates.
const AGENDA_DAYS: i64 = 7;

/// What is open across every board: the answer to "what should I be doing",
/// asked without having to remember which space to look in.
#[derive(Serialize)]
pub struct Agenda {
    pub cards: Vec<AgendaCard>,
    /// Whether the diary has an entry for today, in the configured timezone —
    /// `None` when there is no diary space to ask about.
    pub diary_today: Option<bool>,
    /// Diary entries from this day in earlier years, the latest year first.
    pub on_this_day: Vec<crate::routes::diary::Entry>,
    /// Today in the configured timezone, which is the day the cards are due
    /// on and the mails are sent by — not the browser's, which may be
    /// somewhere else, and not 24 hours long when the clocks change.
    pub today: AgendaDay,
}

#[derive(Serialize)]
pub struct AgendaDay {
    /// `2026-10-21`.
    pub date: String,
    /// When today starts and ends, in Unix seconds.
    pub start: i64,
    pub end: i64,
    /// The end of the seventh day after today: what is due before it is this week.
    pub week_end: i64,
}

#[derive(Serialize)]
pub struct AgendaCard {
    #[serde(flatten)]
    pub card: Card,
    pub space_id: i64,
    pub space_slug: String,
    pub list_name: String,
    /// `inbox` for the first list, `doing` for any list between it and the
    /// last, `done` once finished.
    pub role: &'static str,
}

/// What a card is read with, from `cards c`, as `row_to_card` reads it.
macro_rules! card_columns {
    () => {
        "c.id, c.list_id, c.title, c.body, c.node_id, c.due_at, c.done_at,
         c.repeat, c.checklist, c.labels, c.position, c.created_at, c.updated_at,
         (SELECT n.name FROM nodes n WHERE n.id = c.node_id) AS node_name"
    };
}

/// Cards with where each stands, as `row_to_placed` reads them: on a board
/// that is shown, of a space that is not in the trash. The query goes on with
/// its own `WHERE`.
const PLACED: &str = concat!(
    "WITH bounds AS (
         SELECT space_id, MIN(position) AS first, MAX(position) AS last
         FROM board_lists GROUP BY space_id
     )
     SELECT ",
    card_columns!(),
    ",
            s.id AS space_id, s.slug AS space_slug, l.name AS list_name,
            l.position = b.first AS is_first
     FROM cards c
     JOIN board_lists l ON l.id = c.list_id
     JOIN bounds b ON b.space_id = l.space_id
     JOIN nodes s ON s.id = l.space_id AND s.has_board <> 0 AND s.parent_id IS NULL"
);

fn row_to_placed(row: &sqlx::sqlite::SqliteRow) -> AgendaCard {
    let card = row_to_card(row);
    AgendaCard {
        role: match (card.done_at, row.get::<bool, _>("is_first")) {
            (Some(_), _) => "done",
            (None, true) => "inbox",
            (None, false) => "doing",
        },
        card,
        space_id: row.get("space_id"),
        space_slug: row.get("space_slug"),
        list_name: row.get("list_name"),
    }
}

/// Every unfinished card that is due within a week (or overdue), or that sits
/// in a list between a board's inbox and its done list — work in hand. The
/// last list of each board is done by definition, so nothing there is asked.
pub async fn agenda(_: Session, State(state): State<AppState>) -> AppResult<Json<Agenda>> {
    let cards = agenda_cards(&state.db, crate::now() + (AGENDA_DAYS + 1) * 86400).await?;

    let has_diary: Option<i64> = sqlx::query_scalar("SELECT id FROM nodes WHERE role = 'diary'")
        .fetch_optional(&state.db)
        .await?;
    let tz = state.config.mail.timezone;
    let diary_today = match has_diary {
        Some(_) => {
            let today = crate::notify::local_day(tz, crate::now());
            Some(crate::notify::diary_written(&state.db, tz, today).await?)
        }
        None => None,
    };
    let on_this_day = crate::routes::diary::on_this_day(&state.db, tz, crate::now()).await?;

    let day = crate::notify::local_day(tz, crate::now());
    let (start, end) = crate::notify::day_bounds(tz, day);
    let (_, week_end) = crate::notify::day_bounds(tz, day + chrono::Duration::days(AGENDA_DAYS));
    Ok(Json(Agenda {
        cards,
        diary_today,
        on_this_day,
        today: AgendaDay {
            date: day.format("%Y-%m-%d").to_string(),
            start,
            end,
            week_end,
        },
    }))
}

/// The agenda's cards, due before `horizon` or in hand. Apart from the handler
/// so the selection can be tested against a real schema.
async fn agenda_cards(db: &SqlitePool, horizon: i64) -> AppResult<Vec<AgendaCard>> {
    let rows = sqlx::query(&format!(
        "{PLACED}
         WHERE c.done_at IS NULL
           AND (l.position < b.last OR b.first = b.last)
           AND ((c.due_at IS NOT NULL AND c.due_at < ?1) OR (l.position > b.first))
         ORDER BY c.due_at IS NULL, c.due_at, s.position, l.position, c.position"
    ))
    .bind(horizon)
    .fetch_all(db)
    .await?;
    Ok(rows.iter().map(row_to_placed).collect())
}

/// How many finished cards a document lists. A repeating card leaves one
/// behind every time it is done, and all of them would bury the open one.
const FINISHED_SHOWN: usize = 10;

/// The cards about a node: the ones whose notes it is, and the ones whose
/// summary links to it. The open ones first, the soonest due first; then the
/// last few finished, the latest first.
pub(crate) async fn cards_of(db: &mut SqliteConnection, node: i64) -> AppResult<Vec<AgendaCard>> {
    let rows = sqlx::query(&format!(
        "{PLACED}
         WHERE c.node_id = ?1 OR c.id IN (SELECT card_id FROM card_links WHERE to_id = ?1)
         ORDER BY c.done_at IS NOT NULL, c.due_at IS NULL, c.due_at, c.done_at DESC,
                  s.position, l.position, c.position"
    ))
    .bind(node)
    .fetch_all(&mut *db)
    .await?;
    let (open, done): (Vec<AgendaCard>, Vec<AgendaCard>) = rows
        .iter()
        .map(row_to_placed)
        .partition(|card| card.card.done_at.is_none());
    Ok(open
        .into_iter()
        .chain(done.into_iter().take(FINISHED_SHOWN))
        .collect())
}

/// `GET /api/nodes/{id}/cards` — the cards about a document, to show beside
/// it and to work on without going to the board.
pub async fn node_cards(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<Vec<AgendaCard>>> {
    let mut conn = state.db.acquire().await?;
    if trash::in_trash(&mut conn, id).await? {
        return Err(AppError::NotFound);
    }
    cards_of(&mut conn, id).await.map(Json)
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: String,
    /// Confine the search to one space's board.
    #[serde(default)]
    pub space: Option<i64>,
}

/// How many cards a search lists.
const SEARCH_LIMIT: i64 = 50;

/// `GET /api/cards?q=…` — the cards whose title, summary or a step of whose
/// checklist matches, the open ones first and the best match first in each.
pub async fn search(
    _: Session,
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> AppResult<Json<Vec<AgendaCard>>> {
    let Some(query) = nodes::fts_query(&params.q) else {
        return Ok(Json(Vec::new()));
    };
    let rows = sqlx::query(&format!(
        "{PLACED}
         JOIN cards_fts ON cards_fts.rowid = c.id
         WHERE cards_fts MATCH ?1 AND (?2 IS NULL OR s.id = ?2)
         ORDER BY c.done_at IS NOT NULL, cards_fts.rank
         LIMIT ?3"
    ))
    .bind(query)
    .bind(params.space)
    .bind(SEARCH_LIMIT)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.iter().map(row_to_placed).collect()))
}

/// `done_at` after a card moves from list `from` to list `to` on a board whose
/// last list is `last`. Shuffling inside one list changes nothing; arriving in
/// the last list finishes the card (keeping an earlier stamp), leaving it
/// reopens it.
fn done_after_move(from: i64, to: i64, last: i64, done_at: Option<i64>, now: i64) -> Option<i64> {
    if from == to {
        done_at
    } else if to == last {
        done_at.or(Some(now))
    } else {
        None
    }
}

/// The id and position of the last list of a space's board — its "done".
async fn last_list(
    db: impl sqlx::SqliteExecutor<'_>,
    space_id: i64,
) -> AppResult<Option<(i64, i64)>> {
    Ok(sqlx::query(
        "SELECT id, position FROM board_lists WHERE space_id = ?1
         ORDER BY position DESC, id DESC LIMIT 1",
    )
    .bind(space_id)
    .fetch_optional(db)
    .await?
    .map(|row| (row.get("id"), row.get("position"))))
}

fn row_to_card(row: &sqlx::sqlite::SqliteRow) -> Card {
    Card {
        id: row.get("id"),
        list_id: row.get("list_id"),
        title: row.get("title"),
        body: row.get("body"),
        node_id: row.get("node_id"),
        node_name: row.get("node_name"),
        due_at: row.get("due_at"),
        done_at: row.get("done_at"),
        repeat: row.get("repeat"),
        checklist: json_column(row, "checklist"),
        labels: json_column(row, "labels"),
        position: row.get("position"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        wikilinks: BTreeMap::new(),
    }
}

/// The space behind an id, refusing anything that is not a space with a board.
async fn board_space(db: &SqlitePool, space_id: i64) -> AppResult<String> {
    let row = sqlx::query("SELECT kind, name, has_board FROM nodes WHERE id = ?1")
        .bind(space_id)
        .fetch_optional(db)
        .await?
        .ok_or(AppError::NotFound)?;

    if row.get::<String, _>("kind") != SPACE {
        return Err(AppError::BadRequest("only a space has a board".into()));
    }
    if row.get::<i64, _>("has_board") == 0 {
        // The diary is the case this exists for: a journal has no backlog.
        return Err(AppError::BadRequest(
            "this space has no board — :set board turns one on".into(),
        ));
    }
    Ok(row.get("name"))
}

/// The space a list belongs to, and the same board check along the way.
async fn list_space(db: &SqlitePool, list_id: i64) -> AppResult<i64> {
    let space_id: i64 = sqlx::query("SELECT space_id FROM board_lists WHERE id = ?1")
        .bind(list_id)
        .fetch_optional(db)
        .await?
        .ok_or(AppError::NotFound)?
        .get("space_id");
    board_space(db, space_id).await?;
    Ok(space_id)
}

/// Seed a board that has no lists yet. A space created before this feature
/// existed — or one whose board was switched on later — gets its columns the
/// first time the board is looked at, which is the only moment it matters.
async fn ensure_lists(db: &SqlitePool, space_id: i64) -> AppResult<()> {
    // Count and seed under one write lock: two tabs opening a fresh board at
    // once would otherwise both find it empty and seed it twice.
    let mut tx = crate::db::begin_write(db).await?;
    let existing: i64 = sqlx::query("SELECT COUNT(*) AS n FROM board_lists WHERE space_id = ?1")
        .bind(space_id)
        .fetch_one(&mut *tx)
        .await?
        .get("n");
    if existing > 0 {
        return Ok(());
    }

    let now = crate::now();
    for (position, name) in DEFAULT_LISTS.iter().enumerate() {
        sqlx::query(
            "INSERT INTO board_lists (space_id, name, position, created_at) VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(space_id)
        .bind(name)
        .bind(position as i64)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Make every card on a board agree with where it stands: finished in the
/// last list, open anywhere else. Moving a card keeps this by itself; this is
/// for the changes that move the lists instead — one added after a lone list,
/// the last one deleted. A board of one list has no "done" to agree with and
/// is left as it is.
async fn settle(db: &mut SqliteConnection, space_id: i64) -> AppResult<()> {
    let lists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM board_lists WHERE space_id = ?1")
        .bind(space_id)
        .fetch_one(&mut *db)
        .await?;
    let Some((last, _)) = last_list(&mut *db, space_id).await? else {
        return Ok(());
    };
    if lists < 2 {
        return Ok(());
    }
    sqlx::query(
        "UPDATE cards SET done_at = NULL
         WHERE done_at IS NOT NULL AND list_id <> ?2
           AND list_id IN (SELECT id FROM board_lists WHERE space_id = ?1)",
    )
    .bind(space_id)
    .bind(last)
    .execute(&mut *db)
    .await?;
    sqlx::query("UPDATE cards SET done_at = ?2 WHERE done_at IS NULL AND list_id = ?1")
        .bind(last)
        .bind(crate::now())
        .execute(&mut *db)
        .await?;
    Ok(())
}

/// Put the lists and cards of an exported board back on a space. An empty
/// board takes the lists as they were; one that has lists already keeps them,
/// gains the ones it lacks in front of its last — the way `:list` adds one —
/// and takes each card into the list of the same name. Returns the cards.
pub(crate) async fn restore(
    db: &mut SqliteConnection,
    space_id: i64,
    lists: &[ListMeta],
    documents: &HashMap<String, i64>,
) -> AppResult<usize> {
    if lists.is_empty() {
        return Ok(0);
    }
    let existing: Vec<(i64, String)> =
        sqlx::query("SELECT id, name FROM board_lists WHERE space_id = ?1 ORDER BY position, id")
            .bind(space_id)
            .fetch_all(&mut *db)
            .await?
            .iter()
            .map(|row| (row.get("id"), row.get("name")))
            .collect();

    let now = crate::now();
    let mut ids = Vec::with_capacity(lists.len());
    for (index, list) in lists.iter().enumerate() {
        if let Some((id, _)) = existing.iter().find(|(_, name)| *name == list.name) {
            ids.push(*id);
            continue;
        }
        let position = match last_list(&mut *db, space_id).await? {
            // In front of "done", which has to stay last.
            Some((last, position)) if !existing.is_empty() => {
                sqlx::query("UPDATE board_lists SET position = position + 1 WHERE id = ?1")
                    .bind(last)
                    .execute(&mut *db)
                    .await?;
                position
            }
            _ => index as i64,
        };
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO board_lists (space_id, name, position, created_at)
             VALUES (?1, ?2, ?3, ?4) RETURNING id",
        )
        .bind(space_id)
        .bind(&list.name)
        .bind(position)
        .bind(now)
        .fetch_one(&mut *db)
        .await?;
        ids.push(id);
    }

    let mut count = 0;
    for (list, id) in lists.iter().zip(ids) {
        let mut position: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(position), -1) + 1 FROM cards WHERE list_id = ?1",
        )
        .bind(id)
        .fetch_one(&mut *db)
        .await?;
        for card in &list.cards {
            let card_id: i64 = sqlx::query_scalar(
                "INSERT INTO cards (list_id, node_id, title, body, due_at, done_at, repeat,
                                    checklist, labels, position, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 RETURNING id",
            )
            .bind(id)
            .bind(card.document.as_ref().and_then(|path| documents.get(path)))
            .bind(&card.title)
            .bind(&card.body)
            .bind(card.due_at)
            .bind(card.done_at)
            // A rule this build cannot read is dropped rather than kept as
            // text nothing would ever act on.
            .bind(
                card.repeat
                    .as_deref()
                    .and_then(Repeat::parse)
                    .map(Repeat::name),
            )
            .bind(to_json(&tidy_checklist(card.checklist.clone())?))
            .bind(to_json(&tidy_labels(card.labels.clone())?))
            .bind(position)
            .bind(card.created_at)
            .bind(card.updated_at)
            .fetch_one(&mut *db)
            .await?;
            links::index_card(db, card_id, &card.body).await?;
            position += 1;
            count += 1;
        }
    }
    settle(db, space_id).await?;
    Ok(count)
}

/// A card links to a document — its notes — and to nothing else: a folder
/// cannot be opened to write in, and a file has no text to hold them.
async fn linkable(db: &SqlitePool, node_id: Option<i64>) -> AppResult<()> {
    let Some(node_id) = node_id else {
        return Ok(());
    };
    let kind: Option<String> = sqlx::query_scalar("SELECT kind FROM nodes WHERE id = ?1")
        .bind(node_id)
        .fetch_optional(db)
        .await?;
    match kind.as_deref() {
        Some(DOCUMENT) => Ok(()),
        Some(kind) => Err(AppError::BadRequest(format!(
            "a card links to a document, not a {kind}"
        ))),
        None => Err(AppError::NotFound),
    }
}

pub async fn board(
    _: Session,
    State(state): State<AppState>,
    Path(space_id): Path<i64>,
) -> AppResult<Json<Board>> {
    let space_name = board_space(&state.db, space_id).await?;
    ensure_lists(&state.db, space_id).await?;
    load(&state, space_id, space_name).await.map(Json)
}

async fn load(state: &AppState, space_id: i64, space_name: String) -> AppResult<Board> {
    let list_rows = sqlx::query(
        "SELECT id, name, position FROM board_lists WHERE space_id = ?1 ORDER BY position, id",
    )
    .bind(space_id)
    .fetch_all(&state.db)
    .await?;

    // One query for every card on the board rather than one per list: a board
    // is small, and the round trips are what would be felt.
    let card_rows = sqlx::query(concat!(
        "SELECT ",
        card_columns!(),
        "
         FROM cards c
         JOIN board_lists l ON l.id = c.list_id
         WHERE l.space_id = ?1
         ORDER BY c.position, c.id"
    ))
    .bind(space_id)
    .fetch_all(&state.db)
    .await?;

    let mut cards: Vec<Card> = card_rows.iter().map(row_to_card).collect();
    let mut conn = state.db.acquire().await?;
    for card in cards.iter_mut().filter(|card| card.body.contains("[[")) {
        card.wikilinks = links::wikilinks_in(&mut conn, Some(space_id), &card.body).await?;
    }
    let lists = list_rows
        .iter()
        .map(|row| {
            let id: i64 = row.get("id");
            BoardList {
                id,
                name: row.get("name"),
                position: row.get("position"),
                cards: cards.extract_if(.., |card| card.list_id == id).collect(),
            }
        })
        .collect();

    Ok(Board {
        space_id,
        space_name,
        lists,
    })
}

pub async fn create_list(
    _: Session,
    State(state): State<AppState>,
    Path(space_id): Path<i64>,
    Json(input): Json<ListInput>,
) -> AppResult<Json<Board>> {
    let space_name = board_space(&state.db, space_id).await?;
    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("a list needs a name".into()));
    }

    // A new column goes in front of the last one, because the last one is
    // "done" and has to stay last for that to keep meaning anything. A board
    // down to a single list just grows a second one after it.
    let mut tx = crate::db::begin_write(&state.db).await?;
    let count: i64 = sqlx::query("SELECT COUNT(*) AS n FROM board_lists WHERE space_id = ?1")
        .bind(space_id)
        .fetch_one(&mut *tx)
        .await?
        .get("n");

    let position = match last_list(&mut *tx, space_id).await? {
        Some((last, position)) if count >= 2 => {
            sqlx::query("UPDATE board_lists SET position = position + 1 WHERE id = ?1")
                .bind(last)
                .execute(&mut *tx)
                .await?;
            position
        }
        Some((_, position)) => position + 1,
        None => 0,
    };

    sqlx::query(
        "INSERT INTO board_lists (space_id, name, position, created_at) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(space_id)
    .bind(name)
    .bind(position)
    .bind(crate::now())
    .execute(&mut *tx)
    .await?;
    settle(&mut tx, space_id).await?;
    tx.commit().await?;

    state.backup.signal();
    load(&state, space_id, space_name).await.map(Json)
}

pub async fn rename_list(
    _: Session,
    State(state): State<AppState>,
    Path(list_id): Path<i64>,
    Json(input): Json<ListInput>,
) -> AppResult<Json<Board>> {
    let space_id = list_space(&state.db, list_id).await?;
    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("a list needs a name".into()));
    }

    sqlx::query("UPDATE board_lists SET name = ?1 WHERE id = ?2")
        .bind(name)
        .bind(list_id)
        .execute(&state.db)
        .await?;

    state.backup.signal();
    let space_name = board_space(&state.db, space_id).await?;
    load(&state, space_id, space_name).await.map(Json)
}

/// Delete a list and the cards on it. A board must keep one list, or there is
/// nowhere left to put a card.
pub async fn remove_list(
    _: Session,
    State(state): State<AppState>,
    Path(list_id): Path<i64>,
) -> AppResult<Json<Board>> {
    let space_id = list_space(&state.db, list_id).await?;

    let mut tx = crate::db::begin_write(&state.db).await?;
    let remaining: i64 = sqlx::query("SELECT COUNT(*) AS n FROM board_lists WHERE space_id = ?1")
        .bind(space_id)
        .fetch_one(&mut *tx)
        .await?
        .get("n");
    if remaining <= 1 {
        return Err(AppError::BadRequest(
            "a board needs at least one list".into(),
        ));
    }

    sqlx::query("DELETE FROM board_lists WHERE id = ?1")
        .bind(list_id)
        .execute(&mut *tx)
        .await?;
    // Deleting the last list makes the one before it "done".
    settle(&mut tx, space_id).await?;
    tx.commit().await?;

    state.backup.signal();
    let space_name = board_space(&state.db, space_id).await?;
    load(&state, space_id, space_name).await.map(Json)
}

pub async fn create_card(
    _: Session,
    State(state): State<AppState>,
    Path(list_id): Path<i64>,
    Json(input): Json<CardInput>,
) -> AppResult<Json<Card>> {
    let space_id = list_space(&state.db, list_id).await?;
    let id = insert_card(&state.db, space_id, list_id, &input).await?;
    state.backup.signal();
    card(&state.db, id).await.map(Json)
}

/// A new card at the bottom of a list, which must be on `space_id`'s board.
pub(crate) async fn insert_card(
    db: &SqlitePool,
    space_id: i64,
    list_id: i64,
    input: &CardInput,
) -> AppResult<i64> {
    let title = input.title.trim();
    if title.is_empty() {
        return Err(AppError::BadRequest("a card needs a title".into()));
    }
    linkable(db, input.node_id).await?;

    let mut tx = crate::db::begin_write(db).await?;
    let position: i64 =
        sqlx::query("SELECT COALESCE(MAX(position), -1) + 1 AS next FROM cards WHERE list_id = ?1")
            .bind(list_id)
            .fetch_one(&mut *tx)
            .await?
            .get("next");

    let now = crate::now();
    let id: i64 = sqlx::query(
        "INSERT INTO cards (list_id, node_id, title, body, due_at, position, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
         RETURNING id",
    )
    .bind(list_id)
    .bind(input.node_id)
    .bind(title)
    .bind(&input.body)
    .bind(input.due_at)
    .bind(position)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?
    .get("id");
    links::index_card(&mut tx, id, &input.body).await?;
    // A card made straight into the done list is finished as it lands.
    settle(&mut tx, space_id).await?;
    tx.commit().await?;
    Ok(id)
}

/// Where a card goes that is written down from somewhere else.
pub struct Inbox {
    pub space_id: i64,
    pub space_slug: String,
    pub list_id: i64,
    pub list_name: String,
}

/// The inbox — the first list — of the board of the space with that slug, or
/// of the first space in the switcher that has a board: the rule `:card`
/// follows away from the board, for a capture that has no screen to ask on.
pub(crate) async fn inbox_of(db: &SqlitePool, space: Option<&str>) -> AppResult<Inbox> {
    let found: Option<(i64, String)> = sqlx::query_as(
        "SELECT id, slug FROM nodes
         WHERE parent_id IS NULL AND kind = 'space' AND has_board = 1
           AND (?1 IS NULL OR slug = ?1)
         ORDER BY position, name
         LIMIT 1",
    )
    .bind(space)
    .fetch_optional(db)
    .await?;
    let (space_id, space_slug) = found.ok_or_else(|| {
        AppError::BadRequest(match space {
            Some(slug) => format!("no space {slug} with a board"),
            None => "no space has a board — :set board turns one on".into(),
        })
    })?;
    ensure_lists(db, space_id).await?;
    let (list_id, list_name): (i64, String) = sqlx::query_as(
        "SELECT id, name FROM board_lists WHERE space_id = ?1 ORDER BY position, id LIMIT 1",
    )
    .bind(space_id)
    .fetch_one(db)
    .await?;
    Ok(Inbox {
        space_id,
        space_slug,
        list_id,
        list_name,
    })
}

pub async fn update_card(
    _: Session,
    State(state): State<AppState>,
    Path(card_id): Path<i64>,
    Json(patch): Json<CardPatch>,
) -> AppResult<Json<Moved>> {
    let current = card(&state.db, card_id).await?;
    let now = crate::now();

    let title = match patch.title {
        Some(title) if title.trim().is_empty() => {
            return Err(AppError::BadRequest("a card needs a title".into()))
        }
        Some(title) => title.trim().to_string(),
        None => current.title.clone(),
    };
    if let Some(node_id) = patch.node_id {
        linkable(&state.db, node_id).await?;
    }
    let repeat = match patch.repeat {
        Some(Some(rule)) => Some(
            Repeat::parse(&rule)
                .ok_or_else(|| {
                    AppError::BadRequest(format!(
                        "not a rule: {} — try daily, weekly, monthly, yearly or +2w",
                        rule.trim()
                    ))
                })?
                .name(),
        ),
        Some(None) => None,
        None => current.repeat.clone(),
    };
    let checklist = match patch.checklist {
        Some(tasks) => tidy_checklist(tasks)?,
        None => current.checklist.clone(),
    };
    let labels = match patch.labels {
        Some(labels) => tidy_labels(labels)?,
        None => current.labels.clone(),
    };

    let mut tx = crate::db::begin_write(&state.db).await?;
    sqlx::query(
        "UPDATE cards SET title = ?1, body = ?2, node_id = ?3, due_at = ?4, repeat = ?5,
                          checklist = ?6, labels = ?7, updated_at = ?8
         WHERE id = ?9",
    )
    .bind(&title)
    .bind(patch.body.as_ref().unwrap_or(&current.body))
    .bind(patch.node_id.unwrap_or(current.node_id))
    .bind(patch.due_at.unwrap_or(current.due_at))
    .bind(&repeat)
    .bind(to_json(&checklist))
    .bind(to_json(&labels))
    .bind(now)
    .bind(card_id)
    .execute(&mut *tx)
    .await?;
    if let Some(body) = &patch.body {
        links::index_card(&mut tx, card_id, body).await?;
    }
    // The move below sees the card as it is now, so a card given its rule
    // and finished in one request comes back by that rule.
    let current = Card {
        title,
        body: patch.body.unwrap_or(current.body),
        node_id: patch.node_id.unwrap_or(current.node_id),
        due_at: patch.due_at.unwrap_or(current.due_at),
        repeat,
        checklist,
        labels,
        ..current
    };

    // `done` is where the card stands, not a flag beside it: finishing a card
    // moves it into the last list, reopening one moves it back to the first,
    // and the stamp follows from the move as it does for any other.
    let mut again = None;
    if let Some(done) = patch.done.filter(|&done| done != current.done_at.is_some()) {
        let space_id = list_space(&state.db, current.list_id).await?;
        let lists: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM board_lists WHERE space_id = ?1 ORDER BY position, id",
        )
        .bind(space_id)
        .fetch_all(&mut *tx)
        .await?;
        let (Some(&first), Some(&last)) = (lists.first(), lists.last()) else {
            return Err(AppError::NotFound);
        };
        if lists.len() < 2 {
            return Err(AppError::BadRequest(
                "a board needs two lists for one to mean done".into(),
            ));
        }
        let to = if done { last } else { first };
        again = relocate(
            &mut tx,
            &current,
            to,
            last,
            None,
            state.config.mail.timezone,
            now,
        )
        .await?;
    }
    tx.commit().await?;

    state.backup.signal();
    moved(&state.db, card_id, again).await.map(Json)
}

pub async fn remove_card(
    _: Session,
    State(state): State<AppState>,
    Path(card_id): Path<i64>,
) -> AppResult<Json<serde_json::Value>> {
    let affected = sqlx::query("DELETE FROM cards WHERE id = ?1")
        .bind(card_id)
        .execute(&state.db)
        .await?
        .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound);
    }

    state.backup.signal();
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// Move a card to a list and a place in it. Positions are rewritten densely
/// afterwards, so they stay the small numbers the board reads them as instead
/// of drifting into gaps nobody can reason about.
pub async fn move_card(
    _: Session,
    State(state): State<AppState>,
    Path(card_id): Path<i64>,
    Json(input): Json<MoveInput>,
) -> AppResult<Json<Moved>> {
    let current = card(&state.db, card_id).await?;
    let from = list_space(&state.db, current.list_id).await?;
    let to = list_space(&state.db, input.list_id).await?;
    if from != to {
        // Boards are per space on purpose; a card that could cross would make
        // "what am I working on in work" a lie.
        return Err(AppError::BadRequest(
            "a card cannot move to another space's board".into(),
        ));
    }

    let mut tx = crate::db::begin_write(&state.db).await?;
    let now = crate::now();
    let last = last_list(&mut *tx, to)
        .await?
        .map_or(input.list_id, |(id, _)| id);
    let tz = state.config.mail.timezone;
    let again = relocate(
        &mut tx,
        &current,
        input.list_id,
        last,
        input.position,
        tz,
        now,
    )
    .await?;
    tx.commit().await?;

    state.backup.signal();
    moved(&state.db, card_id, again).await.map(Json)
}

async fn moved(db: &SqlitePool, id: i64, again: Option<i64>) -> AppResult<Moved> {
    Ok(Moved {
        card: card(db, id).await?,
        again: match again {
            Some(again) => Some(card(db, again).await?),
            None => None,
        },
    })
}

/// Put a card on `to` at `position` (the end, if none), stamping or clearing
/// its done time by where it lands, and close the ranks of both lists. A
/// repeating card that this finishes comes back: the id is of its copy.
async fn relocate(
    db: &mut SqliteConnection,
    current: &Card,
    to: i64,
    last: i64,
    position: Option<i64>,
    tz: Tz,
    now: i64,
) -> AppResult<Option<i64>> {
    let card_id = current.id;
    let done_at = done_after_move(current.list_id, to, last, current.done_at, now);
    let again = match current.done_at.is_none() && done_at.is_some() {
        true => come_back(&mut *db, current, tz, now).await?,
        false => None,
    };

    sqlx::query("UPDATE cards SET list_id = ?1, done_at = ?2, updated_at = ?3 WHERE id = ?4")
        .bind(to)
        .bind(done_at)
        .bind(now)
        .bind(card_id)
        .execute(&mut *db)
        .await?;

    let ids: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM cards WHERE list_id = ?1 ORDER BY position, id")
            .bind(to)
            .fetch_all(&mut *db)
            .await?;

    let target = position.unwrap_or(i64::MAX).max(0) as usize;
    for (position, id) in reordered(&ids, card_id, target).iter().enumerate() {
        sqlx::query("UPDATE cards SET position = ?1 WHERE id = ?2")
            .bind(position as i64)
            .bind(id)
            .execute(&mut *db)
            .await?;
    }

    // The list it came from keeps a hole where the card was. Harmless for
    // ordering, but it would grow with every move.
    if current.list_id != to {
        let left: Vec<i64> =
            sqlx::query_scalar("SELECT id FROM cards WHERE list_id = ?1 ORDER BY position, id")
                .bind(current.list_id)
                .fetch_all(&mut *db)
                .await?;
        for (position, id) in left.iter().enumerate() {
            sqlx::query("UPDATE cards SET position = ?1 WHERE id = ?2")
                .bind(position as i64)
                .bind(id)
                .execute(&mut *db)
                .await?;
        }
    }
    Ok(again)
}

/// The next one of a repeating card that has just been finished, at the
/// bottom of its board's inbox: the same title, notes, document, labels and
/// rule, and the same checklist with every step open again, due on the
/// rule's next date from today on. A card without a date counts
/// from the moment it was finished. The finished card gives its rule up, so
/// taking it out of done and finishing it again leaves no second copy.
async fn come_back(
    db: &mut SqliteConnection,
    card: &Card,
    tz: Tz,
    now: i64,
) -> AppResult<Option<i64>> {
    let Some(rule) = card.repeat.as_deref().and_then(Repeat::parse) else {
        return Ok(None);
    };
    let today = crate::notify::day_bounds(tz, crate::notify::local_day(tz, now)).0;
    let Some(due_at) = rule.next(tz, card.due_at.unwrap_or(now), today) else {
        return Ok(None);
    };

    let inbox: i64 = sqlx::query_scalar(
        "SELECT id FROM board_lists
         WHERE space_id = (SELECT space_id FROM board_lists WHERE id = ?1)
         ORDER BY position, id LIMIT 1",
    )
    .bind(card.list_id)
    .fetch_one(&mut *db)
    .await?;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO cards (list_id, node_id, title, body, due_at, repeat, checklist, labels,
                            position, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                 (SELECT COALESCE(MAX(position), -1) + 1 FROM cards WHERE list_id = ?1),
                 ?9, ?9)
         RETURNING id",
    )
    .bind(inbox)
    .bind(card.node_id)
    .bind(&card.title)
    .bind(&card.body)
    .bind(due_at)
    .bind(rule.name())
    .bind(to_json(&reopened(&card.checklist)))
    .bind(to_json(&card.labels))
    .bind(now)
    .fetch_one(&mut *db)
    .await?;
    links::index_card(db, id, &card.body).await?;

    sqlx::query("UPDATE cards SET repeat = NULL WHERE id = ?1")
        .bind(card.id)
        .execute(&mut *db)
        .await?;
    Ok(Some(id))
}

/// A checklist to go through again: the same steps, none of them ticked.
fn reopened(tasks: &[Task]) -> Vec<Task> {
    tasks
        .iter()
        .map(|task| Task {
            done: false,
            ..task.clone()
        })
        .collect()
}

async fn card(db: &SqlitePool, id: i64) -> AppResult<Card> {
    let row = sqlx::query(concat!(
        "SELECT ",
        card_columns!(),
        "
         FROM cards c WHERE c.id = ?1"
    ))
    .bind(id)
    .fetch_optional(db)
    .await?
    .ok_or(AppError::NotFound)?;

    Ok(row_to_card(&row))
}

#[cfg(test)]
mod tests {
    use super::{
        agenda_cards, card, done_after_move, load, relocate, reordered, restore, settle,
        tidy_checklist, tidy_labels, Task,
    };
    use crate::routes::manifest::{CardMeta, ListMeta};
    use std::collections::HashMap;

    #[tokio::test]
    async fn a_description_links_by_title_as_a_document_does() {
        let state = crate::routes::tests::state_in(".".into()).await;
        let db = &state.db;
        let work: i64 = sqlx::query_scalar("SELECT id FROM nodes WHERE slug = 'work'")
            .fetch_one(db)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO nodes (parent_id, kind, name, slug, body, created_at, updated_at)
             VALUES (?1, 'document', 'Runbook', 'runbook', '', 0, 0)",
        )
        .bind(work)
        .execute(db)
        .await
        .unwrap();
        let list: i64 = sqlx::query_scalar(
            "INSERT INTO board_lists (space_id, name, position, created_at)
             VALUES (?1, 'inbox', 0, 0) RETURNING id",
        )
        .bind(work)
        .fetch_one(db)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO cards (list_id, title, body, position, created_at, updated_at)
             VALUES (?1, 'rotate', 'see [[Runbook]], then [[Nowhere]]', 0, 0, 0)",
        )
        .bind(list)
        .execute(db)
        .await
        .unwrap();

        let board = load(&state, work, "work".into()).await.unwrap();
        let card = board
            .lists
            .iter()
            .flat_map(|list| &list.cards)
            .find(|card| card.list_id == list)
            .unwrap();
        assert_eq!(
            card.wikilinks.iter().collect::<Vec<_>>(),
            [(&"Runbook".to_string(), &"work/runbook".to_string())]
        );
    }

    /// A space with a board of `inbox`, `doing`, `done` on a fresh database.
    async fn board() -> (sqlx::SqlitePool, i64, Vec<i64>) {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&db).await.unwrap();
        let space: i64 = sqlx::query_scalar(
            "INSERT INTO nodes (parent_id, kind, name, slug, body, has_board, created_at, updated_at)
             VALUES (NULL, 'space', 'job', 'job', '', 1, 0, 0) RETURNING id",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        let mut lists = Vec::new();
        for (position, name) in ["inbox", "doing", "done"].iter().enumerate() {
            let id: i64 = sqlx::query_scalar(
                "INSERT INTO board_lists (space_id, name, position, created_at)
                 VALUES (?1, ?2, ?3, 0) RETURNING id",
            )
            .bind(space)
            .bind(name)
            .bind(position as i64)
            .fetch_one(&db)
            .await
            .unwrap();
            lists.push(id);
        }
        (db, space, lists)
    }

    async fn finish(db: &sqlx::SqlitePool, id: i64, to: i64, last: i64, now: i64) -> Option<i64> {
        let current = card(db, id).await.unwrap();
        let mut conn = db.acquire().await.unwrap();
        relocate(&mut conn, &current, to, last, None, chrono_tz::UTC, now)
            .await
            .unwrap()
    }

    /// Finishing a weekly card leaves next week's in the inbox, once: the
    /// finished one gives its rule up, so reopening it and finishing it again
    /// makes nothing new.
    #[tokio::test]
    async fn a_repeating_card_comes_back_once() {
        let (db, _, lists) = board().await;
        let (inbox, doing, done) = (lists[0], lists[1], lists[2]);
        let day = 86400;
        let id: i64 = sqlx::query_scalar(
            r#"INSERT INTO cards (list_id, title, body, due_at, repeat, checklist, labels,
                                position, created_at, updated_at)
             VALUES (?1, 'water the plants', 'the big one too', ?2, 'weekly',
                     '[{"text":"ferns","done":true},{"text":"cactus","done":false}]',
                     '["home"]', 0, 0, 0) RETURNING id"#,
        )
        .bind(doing)
        .bind(10 * day + 12 * 3600)
        .fetch_one(&db)
        .await
        .unwrap();

        // Finished on day 11, a day late: back on day 17, a week after it was due.
        let again = finish(&db, id, done, done, 11 * day).await.expect("a copy");
        let copy = card(&db, again).await.unwrap();
        assert_eq!(copy.list_id, inbox);
        assert_eq!(copy.title, "water the plants");
        assert_eq!(copy.body, "the big one too");
        assert_eq!(copy.due_at, Some(17 * day + 12 * 3600));
        assert_eq!(copy.repeat.as_deref(), Some("weekly"));
        assert_eq!(copy.done_at, None);
        // The same steps to go through again, none of them ticked yet.
        let steps: Vec<(&str, bool)> = copy
            .checklist
            .iter()
            .map(|t| (t.text.as_str(), t.done))
            .collect();
        assert_eq!(steps, [("ferns", false), ("cactus", false)]);
        assert_eq!(copy.labels, ["home"]);
        assert_eq!(card(&db, id).await.unwrap().repeat, None);

        assert_eq!(finish(&db, id, inbox, done, 12 * day).await, None);
        assert_eq!(finish(&db, id, done, done, 12 * day).await, None);
        let cards: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cards")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(cards, 2);

        // Weeks late, it comes back on the first of its days from today on.
        assert_eq!(
            card(&db, finish(&db, again, done, done, 40 * day).await.unwrap())
                .await
                .unwrap()
                .due_at,
            Some(45 * day + 12 * 3600)
        );
    }

    /// A card without a rule, or one moved between open lists, stays one card.
    #[tokio::test]
    async fn only_finishing_a_repeating_card_makes_a_copy() {
        let (db, _, lists) = board().await;
        let (inbox, doing, done) = (lists[0], lists[1], lists[2]);
        let insert = |repeat: Option<&'static str>| {
            let db = db.clone();
            async move {
                sqlx::query_scalar::<_, i64>(
                    "INSERT INTO cards (list_id, title, due_at, repeat, position, created_at, updated_at)
                     VALUES (?1, 'c', 0, ?2, 0, 0, 0) RETURNING id",
                )
                .bind(inbox)
                .bind(repeat)
                .fetch_one(&db)
                .await
                .unwrap()
            }
        };
        let plain = insert(None).await;
        assert_eq!(finish(&db, plain, done, done, 100).await, None);
        let weekly = insert(Some("weekly")).await;
        assert_eq!(finish(&db, weekly, doing, done, 100).await, None);
        assert_eq!(
            card(&db, weekly).await.unwrap().repeat.as_deref(),
            Some("weekly")
        );
    }

    /// An export carries the rule, and an import puts it back — a rule it
    /// cannot read is dropped rather than stored.
    #[tokio::test]
    async fn an_import_keeps_the_rule() {
        let (db, space, lists) = board().await;
        let meta = |title: &str, repeat: Option<&str>| CardMeta {
            title: title.into(),
            body: String::new(),
            due_at: Some(0),
            done_at: None,
            repeat: repeat.map(String::from),
            checklist: Vec::new(),
            labels: Vec::new(),
            created_at: 0,
            updated_at: 0,
            document: None,
        };
        let manifest = vec![ListMeta {
            name: "inbox".into(),
            cards: vec![
                meta("rent", Some("+1m")),
                meta("odd", Some("sometimes")),
                meta("once", None),
            ],
        }];
        let json = serde_json::to_string(&manifest).unwrap();
        assert!(json.contains(r#""repeat":"+1m""#));
        assert_eq!(
            json.matches("repeat").count(),
            2,
            "absent when there is none"
        );
        let manifest: Vec<ListMeta> = serde_json::from_str(&json).unwrap();

        let mut conn = db.acquire().await.unwrap();
        restore(&mut conn, space, &manifest, &HashMap::new())
            .await
            .unwrap();
        drop(conn);
        let rules: Vec<(String, Option<String>)> =
            sqlx::query_as("SELECT title, repeat FROM cards WHERE list_id = ?1 ORDER BY position")
                .bind(lists[0])
                .fetch_all(&db)
                .await
                .unwrap();
        assert_eq!(
            rules,
            vec![
                ("rent".into(), Some("monthly".into())),
                ("odd".into(), None),
                ("once".into(), None)
            ]
        );
    }

    /// The agenda asks for what is due soon or in hand, and never for what is
    /// finished, parked in the inbox without a date, or due far off.
    #[tokio::test]
    async fn the_agenda_holds_what_is_due_or_in_hand() {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("an in-memory database");
        sqlx::migrate!("./migrations")
            .run(&db)
            .await
            .expect("migrations apply");

        let space: i64 = sqlx::query_scalar(
            "INSERT INTO nodes (parent_id, kind, name, slug, body, has_board, created_at, updated_at)
             VALUES (NULL, 'space', 'job', 'job', '', 1, 0, 0) RETURNING id",
        )
        .fetch_one(&db)
        .await
        .expect("a space");
        let mut lists = Vec::new();
        for (position, name) in ["inbox", "doing", "done"].iter().enumerate() {
            let id: i64 = sqlx::query_scalar(
                "INSERT INTO board_lists (space_id, name, position, created_at)
                 VALUES (?1, ?2, ?3, 0) RETURNING id",
            )
            .bind(space)
            .bind(name)
            .bind(position as i64)
            .fetch_one(&db)
            .await
            .expect("a list");
            lists.push(id);
        }
        let card = |list: i64, title: &'static str, due: Option<i64>, done: Option<i64>| {
            let db = db.clone();
            async move {
                sqlx::query(
                    "INSERT INTO cards (list_id, title, due_at, done_at, position, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, 0, 0, 0)",
                )
                .bind(list)
                .bind(title)
                .bind(due)
                .bind(done)
                .execute(&db)
                .await
                .expect("a card");
            }
        };
        card(lists[0], "someday", None, None).await;
        card(lists[0], "due soon", Some(500), None).await;
        card(lists[0], "due later", Some(5000), None).await;
        card(lists[1], "in hand", None, None).await;
        card(lists[2], "finished", Some(500), Some(600)).await;

        let titles: Vec<(String, &str)> = agenda_cards(&db, 1000)
            .await
            .expect("the agenda")
            .into_iter()
            .map(|c| (c.card.title, c.role))
            .collect();
        assert_eq!(
            titles,
            vec![
                ("due soon".to_string(), "inbox"),
                ("in hand".to_string(), "doing")
            ]
        );
    }

    /// Whatever changes — a card moved, a list added after a lone one, the
    /// last list deleted — a card is finished exactly when it is in the last.
    #[tokio::test]
    async fn done_is_where_a_card_stands() {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&db).await.unwrap();
        let space: i64 = sqlx::query_scalar(
            "INSERT INTO nodes (parent_id, kind, name, slug, body, has_board, created_at, updated_at)
             VALUES (NULL, 'space', 'job', 'job', '', 1, 0, 0) RETURNING id",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        let list = |position: i64| {
            let db = db.clone();
            async move {
                sqlx::query_scalar::<_, i64>(
                    "INSERT INTO board_lists (space_id, name, position, created_at)
                     VALUES (?1, 'l', ?2, 0) RETURNING id",
                )
                .bind(space)
                .bind(position)
                .fetch_one(&db)
                .await
                .unwrap()
            }
        };
        let (inbox, done) = (list(0).await, list(1).await);
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO cards (list_id, title, position, created_at, updated_at)
             VALUES (?1, 'c', 0, 0, 0) RETURNING id",
        )
        .bind(inbox)
        .fetch_one(&db)
        .await
        .unwrap();
        let done_at = |db: sqlx::SqlitePool| async move { card(&db, id).await.unwrap().done_at };

        let current = card(&db, id).await.unwrap();
        let mut conn = db.acquire().await.unwrap();
        relocate(&mut conn, &current, done, done, None, chrono_tz::UTC, 100)
            .await
            .unwrap();
        drop(conn);
        assert_eq!(done_at(db.clone()).await, Some(100));

        // A list added after the last one becomes "done", so the card that
        // was finished in the old last list is open again.
        let later = list(2).await;
        settle(&mut db.acquire().await.unwrap(), space)
            .await
            .unwrap();
        assert_eq!(done_at(db.clone()).await, None);

        // Deleting the new last list hands "done" back to the old one.
        sqlx::query("DELETE FROM board_lists WHERE id = ?1")
            .bind(later)
            .execute(&db)
            .await
            .unwrap();
        settle(&mut db.acquire().await.unwrap(), space)
            .await
            .unwrap();
        assert!(done_at(db.clone()).await.is_some());
    }

    #[test]
    fn a_checklist_keeps_its_steps_trimmed_and_drops_the_empty_ones() {
        let task = |text: &str, done| Task {
            text: text.into(),
            done,
        };
        let kept = tidy_checklist(vec![
            task("  buy paint ", true),
            task("   ", false),
            task("paint", false),
        ])
        .unwrap();
        assert_eq!(kept, [task("buy paint", true), task("paint", false)]);
        assert!(tidy_checklist(vec![task("x", false); 201]).is_err());
    }

    #[test]
    fn labels_are_lower_case_and_each_once() {
        let labels = |names: &[&str]| tidy_labels(names.iter().map(|n| n.to_string()).collect());
        assert_eq!(
            labels(&["Bug", " bug ", "", "uni"]).unwrap(),
            ["bug", "uni"]
        );
        assert!(labels(&[&"x".repeat(33)]).is_err());
        let many: Vec<String> = (0..13).map(|n| n.to_string()).collect();
        assert!(tidy_labels(many).is_err());
    }

    #[test]
    fn arriving_in_the_last_list_finishes_a_card_and_leaving_reopens_it() {
        let (inbox, doing, done) = (1, 2, 3);
        assert_eq!(done_after_move(doing, done, done, None, 100), Some(100));
        // Finished earlier and shuffled back in: the first stamp is the true one.
        assert_eq!(done_after_move(inbox, done, done, Some(50), 100), Some(50));
        assert_eq!(done_after_move(done, doing, done, Some(50), 100), None);
        // Reordering inside a list is not a change of state.
        assert_eq!(done_after_move(done, done, done, Some(50), 100), Some(50));
        assert_eq!(done_after_move(inbox, doing, done, None, 100), None);
    }

    #[test]
    fn a_card_lands_where_it_was_asked_to() {
        let ids = [1, 2, 3, 4];
        assert_eq!(reordered(&ids, 4, 0), vec![4, 1, 2, 3]);
        assert_eq!(reordered(&ids, 1, 2), vec![2, 3, 1, 4]);
        // Past the end is the end, not an error: dragging below the last card
        // is how a card is sent to the bottom.
        assert_eq!(reordered(&ids, 1, 99), vec![2, 3, 4, 1]);
    }

    #[test]
    fn a_card_from_another_list_is_inserted_rather_than_moved() {
        // The card is not in `ids` yet — the usual case when it comes from the
        // list next door.
        assert_eq!(reordered(&[1, 2], 9, 1), vec![1, 9, 2]);
        assert_eq!(reordered(&[], 9, 0), vec![9]);
    }

    #[test]
    fn a_card_kept_in_place_does_not_drift() {
        let ids = [1, 2, 3];
        assert_eq!(reordered(&ids, 2, 1), vec![1, 2, 3]);
    }

    /// The agenda says which day it counts as today, in the workspace's
    /// timezone, with the bounds the browser groups the cards by.
    #[tokio::test]
    async fn the_agenda_says_what_today_is() {
        let state = crate::routes::tests::state_in(".".into()).await;
        let agenda = super::agenda(
            crate::auth::Session { id: "test".into() },
            axum::extract::State(state.clone()),
        )
        .await
        .unwrap()
        .0;
        let tz = state.config.mail.timezone;
        let now = crate::now();
        let day = &agenda.today;
        assert_eq!(
            day.date,
            crate::notify::local_day(tz, now)
                .format("%Y-%m-%d")
                .to_string()
        );
        assert!(
            day.start <= now && now < day.end,
            "{} {} {}",
            day.start,
            now,
            day.end
        );
        // Seven more local days, each 23 to 25 hours long.
        assert!((day.week_end - day.end - 7 * 86400).abs() <= 3600);
    }
}
