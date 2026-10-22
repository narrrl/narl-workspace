//! Writing something down from outside the app: a phone shortcut, a script,
//! `curl`. One request, with a token from `:token add` or a session, and no
//! screen to answer questions on — so where the text goes is decided here,
//! by the same rules the app follows when it is asked the same thing.

use axum::{extract::State, Json};
use chrono::{TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::{
    auth::Capturer,
    error::{AppError, AppResult},
    routes::{
        boards::{self, CardInput},
        diary, links, media, nodes, revisions,
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
}

#[derive(Deserialize)]
pub struct CaptureInput {
    pub text: String,
    #[serde(default)]
    pub to: To,
    /// The slug of the space whose inbox a card goes to. Without it, the
    /// first space that has a board.
    #[serde(default)]
    pub space: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct Captured {
    pub to: To,
    /// The card, or the diary entry.
    pub id: i64,
    /// Where it went, for the shortcut to say: `work/backlog`, `diary/untitled`.
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
    };
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

/// `- 14:05 text`, at the end of today's entry: the one `T` opens. With none
/// yet, the line starts it the way `T` would, from `templates/day` if there
/// is one.
async fn diary(db: &SqlitePool, tz: Tz, text: &str, now: i64) -> AppResult<Captured> {
    let line = format!("- {} {}", clock(tz, now), item(text));

    let mut tx = crate::db::begin_write(db).await?;
    let today = diary::entry_of(&mut tx, tz, crate::notify::local_day(tz, now), now).await?;
    let id = today.id;
    let body = appended(&today.body, &line);
    // What an entry just started replaces is its template, not worth keeping.
    if !today.created {
        revisions::keep(&mut tx, id, &body, revisions::Reason::Edit).await?;
    }
    sqlx::query("UPDATE nodes SET body = ?1, updated_at = ?2 WHERE id = ?3")
        .bind(&body)
        .bind(now)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    media::link_to_node(&mut tx, id, &body).await?;
    links::index(&mut tx, id, &body).await?;
    tx.commit().await?;

    let place = nodes::path_of(db, id)
        .await?
        .iter()
        .map(|crumb| crumb.slug.as_str())
        .collect::<Vec<_>>()
        .join("/");
    Ok(Captured {
        to: To::Diary,
        id,
        place,
    })
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
