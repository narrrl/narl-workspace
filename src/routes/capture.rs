//! Writing something down from outside the app: a phone shortcut, a script,
//! `curl`. One request, with a token from `:token add` or a session, and no
//! screen to answer questions on — so where the text goes is decided here,
//! by the same rules the app follows when it is asked the same thing.

use axum::{extract::State, Json};
use chrono::{TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use sqlx::{SqliteConnection, SqlitePool};

use crate::{
    auth::Capturer,
    error::{AppError, AppResult},
    routes::{
        boards::{self, CardInput},
        diary, links, media,
        nodes::{self, DOCUMENT},
        revisions, scratch, trash,
    },
    state::AppState,
};

#[derive(Deserialize, Serialize, Default, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum To {
    /// A card in an inbox: something to do.
    #[default]
    Card,
    /// A line in today's diary entry: something that happened.
    Diary,
    /// A note on the scratchpad: not clear yet where it belongs.
    Note,
    /// A paragraph at the end of a document: more of what it is about.
    Document,
}

#[derive(Deserialize)]
pub struct CaptureInput {
    pub text: String,
    #[serde(default)]
    pub to: To,
    /// The slug of the space whose inbox a card goes to, or where a
    /// document's title is looked for first. Without it, the first space that
    /// has a board, and a title in any space.
    #[serde(default)]
    pub space: Option<String>,
    /// The document text goes to the end of, as a link to it is written:
    /// `Plan`, `[[Plan]]` or `/n/work/plan`.
    #[serde(default)]
    pub document: Option<String>,
    /// The scratch note the text comes from, which goes once the text has
    /// landed: the way a note leaves the scratchpad.
    #[serde(default)]
    pub note: Option<i64>,
}

#[derive(Serialize, Debug)]
pub struct Captured {
    pub to: To,
    /// The card, the note, or the document — the diary entry among them.
    pub id: i64,
    /// Where it went, for the shortcut to say: `work/backlog`, `diary/untitled`,
    /// `scratch`.
    #[serde(rename = "where")]
    pub place: String,
}

/// `POST /api/capture`.
pub async fn capture(
    who: Capturer,
    State(state): State<AppState>,
    Json(input): Json<CaptureInput>,
) -> AppResult<Json<Captured>> {
    let text = input.text.trim();
    if text.is_empty() {
        return Err(AppError::BadRequest("there is no text to capture".into()));
    }
    let captured = match input.to {
        To::Card => card(&state.db, text, input.space.as_deref()).await?,
        To::Diary => diary(&state.db, state.config.mail.timezone, text, crate::now()).await?,
        To::Note => note(&state.db, text).await?,
        To::Document => {
            let Some(target) = input.document.as_deref() else {
                return Err(AppError::BadRequest(
                    "which document? `document` names it, as a link would".into(),
                ));
            };
            document(&state.db, text, target, input.space.as_deref()).await?
        }
    };
    // Made first, dropped after: a failure in between leaves two copies,
    // never none.
    if let Some(note) = input.note.filter(|_| captured.to != To::Note) {
        scratch::consume(&state.db, note).await?;
    }
    if let Capturer::Token(token) = who {
        tracing::info!(token, to = ?captured.to, "captured into {}", captured.place);
    }
    state.backup.signal();
    Ok(Json(captured))
}

/// The first line is the card's title, and anything after it its notes.
async fn card(db: &SqlitePool, text: &str, space: Option<&str>) -> AppResult<Captured> {
    let inbox = boards::inbox_of(db, space).await?;
    let (title, body) = text
        .split_once('\n')
        .map(|(title, body)| (title.trim(), body.trim()))
        .unwrap_or((text, ""));
    let input = CardInput {
        title: title.to_string(),
        body: body.to_string(),
        node_id: None,
        due_at: None,
    };
    let id = boards::insert_card(db, inbox.space_id, inbox.list_id, &input).await?;
    Ok(Captured {
        to: To::Card,
        id,
        place: format!("{}/{}", inbox.space_slug, inbox.list_name),
    })
}

async fn note(db: &SqlitePool, text: &str) -> AppResult<Captured> {
    let note = scratch::insert(db, text).await?;
    Ok(Captured {
        to: To::Note,
        id: note.id,
        place: "scratch".into(),
    })
}

/// `- 14:05 text`, at the end of today's entry: the one `T` opens. With none
/// yet, the line starts it the way `T` would, from `templates/day` if there
/// is one.
async fn diary(db: &SqlitePool, tz: Tz, text: &str, now: i64) -> AppResult<Captured> {
    let line = format!("- {} {}", clock(tz, now), item(text));

    let mut tx = crate::db::begin_write(db).await?;
    let today = diary::entry_of(&mut tx, tz, crate::notify::local_day(tz, now), now).await?;
    let body = appended(&today.body, &line);
    // What an entry just started replaces is its template, not worth keeping.
    write(&mut tx, today.id, &body, !today.created, now).await?;
    tx.commit().await?;
    landed(db, To::Diary, today.id).await
}

/// The text as paragraphs of their own at the end of a document, found the
/// way a link to it written there would find it.
async fn document(
    db: &SqlitePool,
    text: &str,
    target: &str,
    space: Option<&str>,
) -> AppResult<Captured> {
    let mut tx = crate::db::begin_write(db).await?;
    let space = match space {
        Some(slug) => links::find(&mut tx, &format!("/n/{slug}"), None).await?,
        None => None,
    };
    let id = links::find(&mut tx, target, space)
        .await?
        .ok_or_else(|| AppError::BadRequest(format!("no document {target}")))?;
    let (kind, body): (String, String) =
        sqlx::query_as("SELECT kind, body FROM nodes WHERE id = ?1")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    if kind != DOCUMENT || trash::in_trash(&mut tx, id).await? {
        return Err(AppError::BadRequest(format!("{target} is not a document")));
    }
    let body = match body.trim_end() {
        "" => format!("{text}\n"),
        before => format!("{before}\n\n{text}\n"),
    };
    write(&mut tx, id, &body, true, crate::now()).await?;
    tx.commit().await?;
    landed(db, To::Document, id).await
}

/// A document's text written, as a save writes it: the revision before it
/// kept when it is worth keeping, its media and links read again.
async fn write(
    tx: &mut SqliteConnection,
    id: i64,
    body: &str,
    keep: bool,
    now: i64,
) -> AppResult<()> {
    if keep {
        revisions::keep(tx, id, body, revisions::Reason::Edit).await?;
    }
    sqlx::query("UPDATE nodes SET body = ?1, updated_at = ?2 WHERE id = ?3")
        .bind(body)
        .bind(now)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    media::link_to_node(tx, id, body).await?;
    links::index(tx, id, body).await
}

/// Where it went into a document, said by its path.
async fn landed(db: &SqlitePool, to: To, id: i64) -> AppResult<Captured> {
    let place = nodes::path_of(db, id)
        .await?
        .iter()
        .map(|crumb| crumb.slug.as_str())
        .collect::<Vec<_>>()
        .join("/");
    Ok(Captured { to, id, place })
}

/// `HH:MM` on the wall clock of the configured timezone.
fn clock(tz: Tz, at: i64) -> String {
    Utc.timestamp_opt(at, 0)
        .single()
        .unwrap_or_else(Utc::now)
        .with_timezone(&tz)
        .format("%H:%M")
        .to_string()
}

/// Text of several lines, indented to stay inside its list item.
fn item(text: &str) -> String {
    text.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n  ")
}

/// `body` with `line` after it. A list item right after a paragraph would be
/// read as part of it in some renderers, so a blank line goes between them;
/// after another item it continues the list.
fn appended(body: &str, line: &str) -> String {
    let body = body.trim_end();
    if body.is_empty() {
        return format!("{line}\n");
    }
    let last = body.lines().last().unwrap_or_default();
    let gap = if last.trim_start().starts_with("- ") || last.starts_with("  ") {
        "\n"
    } else {
        "\n\n"
    };
    format!("{body}{gap}{line}\n")
}

#[cfg(test)]
pub(crate) mod tests {
    use axum::{
        body::Body,
        http::{header, Request, StatusCode},
        Router,
    };
    use chrono::TimeZone;
    use chrono_tz::Europe::Berlin;
    use tower::ServiceExt;

    use super::{appended, card, diary, To};
    use crate::{auth::token_hash, state::AppState};

    const TOKEN: &str = "nwc_a-token-for-the-tests";

    /// A line in the diary in Berlin at `now`, for tests elsewhere: the id of
    /// the entry it went into.
    pub(crate) async fn diary_at(db: &sqlx::SqlitePool, text: &str, now: i64) -> i64 {
        diary(db, Berlin, text, now).await.unwrap().id
    }

    async fn state() -> AppState {
        let state = crate::routes::tests::state_in(".".into()).await;
        sqlx::query("INSERT INTO api_tokens (name, hash, created_at) VALUES ('phone', ?1, 0)")
            .bind(token_hash(TOKEN))
            .execute(&state.db)
            .await
            .unwrap();
        state
    }

    async fn call(app: &Router, uri: &str, token: &str, body: &str) -> (StatusCode, String) {
        let request = Request::builder()
            .method(if body.is_empty() { "GET" } else { "POST" })
            .uri(uri)
            .header("content-type", "application/json")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&body).into_owned())
    }

    #[tokio::test]
    async fn a_token_captures_and_opens_nothing_else() {
        let state = state().await;
        let app = crate::routes::api_router(1024).with_state(state.clone());

        let (status, answer) = call(&app, "/capture", TOKEN, r#"{"text":"buy milk"}"#).await;
        assert_eq!(status, StatusCode::OK, "{answer}");
        assert!(answer.contains(r#""where":"work/backlog""#), "{answer}");
        let used: Option<i64> = sqlx::query_scalar("SELECT last_used_at FROM api_tokens")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert!(used.is_some());

        for uri in ["/nodes", "/spaces", "/tokens", "/me"] {
            assert_eq!(
                call(&app, uri, TOKEN, "").await.0,
                StatusCode::UNAUTHORIZED,
                "{uri}"
            );
        }
    }

    #[tokio::test]
    async fn a_wrong_token_is_slowed_down_and_a_removed_one_refused() {
        let state = state().await;
        let app = crate::routes::api_router(1024).with_state(state.clone());
        let text = r#"{"text":"x"}"#;

        for _ in 0..crate::throttle::FREE_ATTEMPTS + 1 {
            let (status, _) = call(&app, "/capture", "nwc_guess", text).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
        assert_eq!(
            call(&app, "/capture", "nwc_guess", text).await.0,
            StatusCode::TOO_MANY_REQUESTS
        );

        // A fresh line, so the wait above does not stand in for the refusal.
        let app = crate::routes::api_router(1024).with_state(AppState {
            token_throttle: Default::default(),
            ..state.clone()
        });
        assert_eq!(call(&app, "/capture", TOKEN, text).await.0, StatusCode::OK);
        sqlx::query("DELETE FROM api_tokens")
            .execute(&state.db)
            .await
            .unwrap();
        assert_eq!(
            call(&app, "/capture", TOKEN, text).await.0,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn a_card_lands_in_the_inbox_it_names() {
        let state = state().await;
        let first = card(&state.db, "call the bank\nabout the card", None)
            .await
            .unwrap();
        assert_eq!((first.to, first.place.as_str()), (To::Card, "work/backlog"));
        let named = card(&state.db, "read chapter 3", Some("uni"))
            .await
            .unwrap();
        assert_eq!(named.place, "uni/backlog");

        let (title, body): (String, String) =
            sqlx::query_as("SELECT title, body FROM cards WHERE id = ?1")
                .bind(first.id)
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!(
            (title.as_str(), body.as_str()),
            ("call the bank", "about the card")
        );

        // The diary has no board, and a slug nobody has is not a guess.
        for space in ["diary", "nowhere"] {
            assert!(card(&state.db, "x", Some(space)).await.is_err(), "{space}");
        }
    }

    #[tokio::test]
    async fn the_diary_starts_todays_entry_then_adds_to_it() {
        let state = state().await;
        let at = |h, m| {
            Berlin
                .with_ymd_and_hms(2026, 10, 20, h, m, 0)
                .unwrap()
                .timestamp()
        };

        let first = diary(&state.db, Berlin, "woke up early", at(7, 5))
            .await
            .unwrap();
        assert!(first.place.starts_with("diary/"), "{}", first.place);
        let second = diary(&state.db, Berlin, "lunch\nwith Anna", at(12, 30))
            .await
            .unwrap();
        assert_eq!(first.id, second.id);
        // Past midnight in Berlin, though not yet in UTC: the next day.
        let tomorrow = diary(&state.db, Berlin, "late", at(23, 59) + 120)
            .await
            .unwrap();
        assert_ne!(tomorrow.id, first.id);

        let body: String = sqlx::query_scalar("SELECT body FROM nodes WHERE id = ?1")
            .bind(first.id)
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(body, "- 07:05 woke up early\n- 12:30 lunch\n  with Anna\n");
        let found: i64 =
            sqlx::query_scalar("SELECT count(*) FROM nodes_fts WHERE nodes_fts MATCH 'anna'")
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!(found, 1);
    }

    /// A note goes on the scratchpad, and search finds it there.
    #[tokio::test]
    async fn a_note_lands_on_the_scratchpad_and_is_found() {
        let state = state().await;
        let app = crate::routes::api_router(1024).with_state(state.clone());
        let (status, answer) = call(
            &app,
            "/capture",
            TOKEN,
            r#"{"text":"ask about the lease","to":"note"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{answer}");
        assert!(answer.contains(r#""where":"scratch""#), "{answer}");

        let found: Vec<String> = sqlx::query_scalar(
            "SELECT n.body FROM scratch_notes n JOIN scratch_fts ON scratch_fts.rowid = n.id
             WHERE scratch_fts MATCH '\"lea\"*'",
        )
        .fetch_all(&state.db)
        .await
        .unwrap();
        assert_eq!(found, ["ask about the lease"]);
    }

    /// A note leaves the scratchpad by landing somewhere else: at the end of
    /// a document named as a link names it, or on a card.
    #[tokio::test]
    async fn a_note_leaves_for_a_document_or_a_card() {
        let state = state().await;
        let work: i64 =
            sqlx::query_scalar("SELECT id FROM nodes WHERE slug = 'work' AND parent_id IS NULL")
                .fetch_one(&state.db)
                .await
                .unwrap();
        let input = serde_json::from_value(serde_json::json!({
            "parent_id": work, "kind": "document", "name": "Plan", "body": "the plan\n",
        }))
        .unwrap();
        let session = crate::auth::Session { id: "test".into() };
        let plan = crate::routes::nodes::create(
            session,
            axum::extract::State(state.clone()),
            axum::Json(input),
        )
        .await
        .unwrap()
        .0
        .id;
        let note = |body: &'static str| super::scratch::insert(&state.db, body);
        let first = note("and a step more").await.unwrap().id;
        let second = note("pay rent\nbefore the 3rd").await.unwrap().id;

        let app = crate::routes::api_router(1024).with_state(state.clone());
        let body = serde_json::json!({
            "text": "and a step more", "to": "document", "document": "[[plan]]", "note": first,
        });
        let (status, answer) = call(&app, "/capture", TOKEN, &body.to_string()).await;
        assert_eq!(status, StatusCode::OK, "{answer}");
        assert!(answer.contains(r#""where":"work/plan""#), "{answer}");
        let text: String = sqlx::query_scalar("SELECT body FROM nodes WHERE id = ?1")
            .bind(plan)
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(text, "the plan\n\nand a step more\n");

        let body = serde_json::json!({ "text": "pay rent\nbefore the 3rd", "note": second });
        assert_eq!(
            call(&app, "/capture", TOKEN, &body.to_string()).await.0,
            StatusCode::OK
        );
        let left: i64 = sqlx::query_scalar("SELECT count(*) FROM scratch_notes")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(left, 0);

        // Nothing it could mean, and the note stays where it was.
        let kept = note("still here").await.unwrap().id;
        for target in ["Nowhere", "/n/work"] {
            let body = serde_json::json!({
                "text": "x", "to": "document", "document": target, "note": kept,
            });
            let (status, _) = call(&app, "/capture", TOKEN, &body.to_string()).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{target}");
        }
        let left: i64 = sqlx::query_scalar("SELECT count(*) FROM scratch_notes")
            .fetch_one(&state.db)
            .await
            .unwrap();
        assert_eq!(left, 1);
    }

    #[test]
    fn a_line_goes_after_a_list_or_a_blank_line_after_prose() {
        assert_eq!(appended("", "- 10:00 x"), "- 10:00 x\n");
        assert_eq!(
            appended("- 09:00 a\n", "- 10:00 x"),
            "- 09:00 a\n- 10:00 x\n"
        );
        assert_eq!(
            appended("A long day.\n\n", "- 10:00 x"),
            "A long day.\n\n- 10:00 x\n"
        );
    }
}
