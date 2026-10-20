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
    extract::{Path, State},
    Json,
};
use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqliteConnection, SqlitePool};

use crate::{
    auth::Session,
    error::{AppError, AppResult},
    routes::{
        manifest::ListMeta,
        nodes::{DOCUMENT, SPACE},
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
    pub position: i64,
    pub created_at: i64,
    pub updated_at: i64,
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
    pub done: Option<bool>,
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
}

#[derive(Serialize)]
pub struct AgendaCard {
    #[serde(flatten)]
    pub card: Card,
    pub space_id: i64,
    pub space_slug: String,
    pub list_name: String,
    /// `inbox` for the first list, `doing` for any list between it and done.
    pub role: &'static str,
}

/// Every unfinished card that is due within a week (or overdue), or that sits
/// in a list between a board's inbox and its done list — work in hand. The
/// last list of each board is done by definition, so nothing there is asked.
pub async fn agenda(_: Session, State(state): State<AppState>) -> AppResult<Json<Agenda>> {
    let cards = agenda_cards(&state.db, crate::now() + (AGENDA_DAYS + 1) * 86400).await?;

    let has_diary: Option<i64> = sqlx::query_scalar("SELECT id FROM nodes WHERE role = 'diary'")
        .fetch_optional(&state.db)
        .await?;
    let diary_today = match has_diary {
        Some(_) => {
            let tz = state.config.mail.timezone;
            let today = crate::notify::local_day(tz, crate::now());
            Some(crate::notify::diary_written(&state.db, tz, today).await?)
        }
        None => None,
    };

    Ok(Json(Agenda { cards, diary_today }))
}

/// The agenda's cards, due before `horizon` or in hand. Apart from the handler
/// so the selection can be tested against a real schema.
async fn agenda_cards(db: &SqlitePool, horizon: i64) -> AppResult<Vec<AgendaCard>> {
    let rows = sqlx::query(
        "WITH bounds AS (
             SELECT space_id, MIN(position) AS first, MAX(position) AS last
             FROM board_lists GROUP BY space_id
         )
         SELECT c.id, c.list_id, c.title, c.body, c.node_id, c.due_at, c.done_at,
                c.position, c.created_at, c.updated_at,
                (SELECT n.name FROM nodes n WHERE n.id = c.node_id) AS node_name,
                s.id AS space_id, s.slug AS space_slug, l.name AS list_name,
                l.position = b.first AS is_first
         FROM cards c
         JOIN board_lists l ON l.id = c.list_id
         JOIN bounds b ON b.space_id = l.space_id
         JOIN nodes s ON s.id = l.space_id
         WHERE s.has_board <> 0 AND s.parent_id IS NULL
           AND c.done_at IS NULL
           AND (l.position < b.last OR b.first = b.last)
           AND ((c.due_at IS NOT NULL AND c.due_at < ?1) OR (l.position > b.first))
         ORDER BY c.due_at IS NULL, c.due_at, s.position, l.position, c.position",
    )
    .bind(horizon)
    .fetch_all(db)
    .await?;

    Ok(rows
        .iter()
        .map(|row| AgendaCard {
            card: row_to_card(row),
            space_id: row.get("space_id"),
            space_slug: row.get("space_slug"),
            list_name: row.get("list_name"),
            role: if row.get::<bool, _>("is_first") {
                "inbox"
            } else {
                "doing"
            },
        })
        .collect())
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
        position: row.get("position"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
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
            sqlx::query(
                "INSERT INTO cards (list_id, node_id, title, body, due_at, done_at, position,
                                    created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )
            .bind(id)
            .bind(card.document.as_ref().and_then(|path| documents.get(path)))
            .bind(&card.title)
            .bind(&card.body)
            .bind(card.due_at)
            .bind(card.done_at)
            .bind(position)
            .bind(card.created_at)
            .bind(card.updated_at)
            .execute(&mut *db)
            .await?;
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
    let card_rows = sqlx::query(
        "SELECT c.id, c.list_id, c.title, c.body, c.node_id, c.due_at, c.done_at,
                c.position, c.created_at, c.updated_at,
                (SELECT n.name FROM nodes n WHERE n.id = c.node_id) AS node_name
         FROM cards c
         JOIN board_lists l ON l.id = c.list_id
         WHERE l.space_id = ?1
         ORDER BY c.position, c.id",
    )
    .bind(space_id)
    .fetch_all(&state.db)
    .await?;

    let lists = list_rows
        .iter()
        .map(|row| {
            let id: i64 = row.get("id");
            BoardList {
                id,
                name: row.get("name"),
                position: row.get("position"),
                cards: card_rows
                    .iter()
                    .filter(|card| card.get::<i64, _>("list_id") == id)
                    .map(row_to_card)
                    .collect(),
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
    let title = input.title.trim();
    if title.is_empty() {
        return Err(AppError::BadRequest("a card needs a title".into()));
    }
    linkable(&state.db, input.node_id).await?;

    let mut tx = crate::db::begin_write(&state.db).await?;
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
    // A card made straight into the done list is finished as it lands.
    settle(&mut tx, space_id).await?;
    tx.commit().await?;

    state.backup.signal();
    card(&state.db, id).await.map(Json)
}

pub async fn update_card(
    _: Session,
    State(state): State<AppState>,
    Path(card_id): Path<i64>,
    Json(patch): Json<CardPatch>,
) -> AppResult<Json<Card>> {
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

    let mut tx = crate::db::begin_write(&state.db).await?;

    // `done` is where the card stands, not a flag beside it: finishing a card
    // moves it into the last list, reopening one moves it back to the first,
    // and the stamp follows from the move as it does for any other.
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
        relocate(&mut tx, &current, to, last, None, now).await?;
    }

    sqlx::query(
        "UPDATE cards SET title = ?1, body = ?2, node_id = ?3, due_at = ?4, updated_at = ?5
         WHERE id = ?6",
    )
    .bind(&title)
    .bind(patch.body.unwrap_or(current.body))
    .bind(patch.node_id.unwrap_or(current.node_id))
    .bind(patch.due_at.unwrap_or(current.due_at))
    .bind(now)
    .bind(card_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    state.backup.signal();
    card(&state.db, card_id).await.map(Json)
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
) -> AppResult<Json<Card>> {
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
    relocate(&mut tx, &current, input.list_id, last, input.position, now).await?;
    tx.commit().await?;

    state.backup.signal();
    card(&state.db, card_id).await.map(Json)
}

/// Put a card on `to` at `position` (the end, if none), stamping or clearing
/// its done time by where it lands, and close the ranks of both lists.
async fn relocate(
    db: &mut SqliteConnection,
    current: &Card,
    to: i64,
    last: i64,
    position: Option<i64>,
    now: i64,
) -> AppResult<()> {
    let card_id = current.id;
    let done_at = done_after_move(current.list_id, to, last, current.done_at, now);

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
    Ok(())
}

async fn card(db: &SqlitePool, id: i64) -> AppResult<Card> {
    let row = sqlx::query(
        "SELECT c.id, c.list_id, c.title, c.body, c.node_id, c.due_at, c.done_at,
                c.position, c.created_at, c.updated_at,
                (SELECT n.name FROM nodes n WHERE n.id = c.node_id) AS node_name
         FROM cards c WHERE c.id = ?1",
    )
    .bind(id)
    .fetch_optional(db)
    .await?
    .ok_or(AppError::NotFound)?;

    Ok(row_to_card(&row))
}

#[cfg(test)]
mod tests {
    use super::{agenda_cards, card, done_after_move, relocate, reordered, settle};

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
        relocate(&mut conn, &current, done, done, None, 100)
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
}
