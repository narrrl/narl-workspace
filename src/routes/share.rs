use axum::{
    extract::{Path, State},
    http::{header, HeaderMap},
    response::{AppendHeaders, IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::Row;
use subtle::ConstantTimeEq;

use crate::{
    auth::{self, Session},
    config::Config,
    error::{AppError, AppResult},
    routes::media,
    state::AppState,
};

#[derive(Serialize)]
pub struct SharedDocument {
    pub name: String,
    pub body: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub token: String,
}

fn random_secret() -> String {
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    B64.encode(bytes)
}

fn hash_key(key: &str) -> String {
    B64.encode(Sha256::digest(key.as_bytes()))
}

/// Constant-time check of a presented key against the stored hash. A share with
/// no hash at all is unreadable rather than open: that combination should not
/// exist, and failing closed is the safe reading of it.
fn key_matches(stored: Option<&str>, presented: &str) -> bool {
    let Some(stored) = stored else { return false };
    bool::from(hash_key(presented).as_bytes().ct_eq(stored.as_bytes()))
}

/// Publish a document, or mint a fresh key for one that is already published.
///
/// The plaintext key is returned exactly once, here. Nothing but its hash is
/// stored, so a link that the author loses cannot be recovered — only replaced,
/// which is what calling this again does. Rotating the key kills the old link
/// while leaving the token, so a revoked reader loses access even if they kept
/// the path.
pub async fn enable(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<serde_json::Value>> {
    let row = sqlx::query("SELECT kind, share_token FROM nodes WHERE id = ?1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::NotFound)?;

    // A share link serves one body. A folder has none, and publishing a whole
    // subtree behind one key is a different feature with different consequences.
    if row.get::<String, _>("kind") != super::nodes::DOCUMENT {
        return Err(AppError::BadRequest("only a document can be shared".into()));
    }

    let token = match row.get::<Option<String>, _>("share_token") {
        Some(token) => token,
        None => random_secret(),
    };
    let key = random_secret();

    sqlx::query("UPDATE nodes SET share_token = ?1, share_key_hash = ?2 WHERE id = ?3")
        .bind(&token)
        .bind(hash_key(&key))
        .bind(id)
        .execute(&state.db)
        .await?;

    // A share link never serves a sealed file, so a reader of this one sees
    // a gap wherever the document embeds one. Worth saying while sharing.
    let sealed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM node_media nm JOIN media m ON m.id = nm.media_id
         WHERE nm.node_id = ?1 AND m.encrypted",
    )
    .bind(id)
    .fetch_one(&state.db)
    .await?;

    state.backup.signal();
    Ok(Json(json!({
        "token": token,
        "key": key,
        "path": format!("/s/{token}#{key}"),
        "sealed": sealed,
    })))
}

pub async fn disable(
    _: Session,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<serde_json::Value>> {
    let affected =
        sqlx::query("UPDATE nodes SET share_token = NULL, share_key_hash = NULL WHERE id = ?1")
            .bind(id)
            .execute(&state.db)
            .await?
            .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound);
    }
    state.backup.signal();
    Ok(Json(json!({ "ok": true })))
}

/// The cookie a share page gets for its media. `<img>` and `<video>` cannot
/// send the key in a header, and a key in their URLs would be written into
/// every proxy's access log; so reading the document hands out this, scoped
/// to the share's own path, and media asks for it instead.
const SHARE_COOKIE: &str = "diary_share";
const SECURE_SHARE_COOKIE: &str = "__Secure-diary_share";
const SHARE_COOKIE_HOURS: i64 = 12;

fn share_cookie_name(config: &Config) -> &'static str {
    if config.secure_cookie {
        SECURE_SHARE_COOKIE
    } else {
        SHARE_COOKIE
    }
}

/// Signed over the key's hash, so minting a new key or unsharing the
/// document voids every cookie handed out under the old one.
fn share_signature(config: &Config, token: &str, key_hash: &str, expires: i64) -> String {
    auth::sign(
        &config.secret,
        &format!("share:{token}:{key_hash}:{expires}"),
    )
}

fn share_cookie(config: &Config, token: &str, key_hash: &str) -> String {
    let max_age = SHARE_COOKIE_HOURS * 3600;
    let expires = crate::now() + max_age;
    let signature = share_signature(config, token, key_hash, expires);
    let name = share_cookie_name(config);
    let secure = if config.secure_cookie { "; Secure" } else { "" };
    format!(
        "{name}={expires}.{signature}; Path=/api/share/{token}/; HttpOnly; SameSite=Strict; \
         Max-Age={max_age}{secure}"
    )
}

/// Whether the request carries a share cookie for this token and key.
fn holds_share_cookie(config: &Config, headers: &HeaderMap, token: &str, key_hash: &str) -> bool {
    let Some((expires, signature)) =
        auth::cookie_value(headers, share_cookie_name(config)).and_then(|v| v.split_once('.'))
    else {
        return false;
    };
    let Ok(expires) = expires.parse::<i64>() else {
        return false;
    };
    let expected = share_signature(config, token, key_hash, expires);
    expires > crate::now() && bool::from(expected.as_bytes().ct_eq(signature.as_bytes()))
}

#[derive(Deserialize)]
pub struct ReadRequest {
    pub key: String,
}

/// `POST /api/share/{token}` — public: no session required. The credential is
/// the pair: the token from the link's path, and the key the reader's
/// browser lifted out of its fragment and sends in the body, where no log
/// on the way sees it.
pub async fn read(
    State(state): State<AppState>,
    Path(token): Path<String>,
    Json(body): Json<ReadRequest>,
) -> AppResult<impl IntoResponse> {
    let row = sqlx::query(
        "SELECT name, body, created_at, updated_at, share_key_hash
         FROM nodes WHERE share_token = ?1",
    )
    .bind(&token)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound)?;

    let stored: Option<String> = row.get("share_key_hash");
    let Some(stored) = stored.filter(|stored| key_matches(Some(stored), &body.key)) else {
        // Deliberately the same answer as an unknown token: a wrong key must not
        // confirm that the token itself is live.
        return Err(AppError::NotFound);
    };

    let cookie = share_cookie(&state.config, &token, &stored);
    Ok((
        AppendHeaders([(header::SET_COOKIE, cookie)]),
        Json(SharedDocument {
            name: row.get("name"),
            body: row.get("body"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
            token,
        }),
    ))
}

/// `GET /api/share/{token}/media/{id}` — public: serves a media file only
/// while it is embedded in a shared document, and only to a browser that
/// read that document with its key and holds the cookie it got for it.
pub async fn serve_media(
    State(state): State<AppState>,
    Path((token, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> AppResult<Response> {
    let stored: Option<String> = sqlx::query(
        "SELECT n.share_key_hash
         FROM node_media nm
         JOIN nodes n ON n.id = nm.node_id
         WHERE nm.media_id = ?1 AND n.share_token = ?2",
    )
    .bind(&id)
    .bind(&token)
    .fetch_optional(&state.db)
    .await?
    .and_then(|row| row.get("share_key_hash"));

    let allowed =
        stored.is_some_and(|stored| holds_share_cookie(&state.config, &headers, &token, &stored));
    if !allowed {
        return Err(AppError::NotFound);
    }
    media::stream_media(&state, &id, false, headers.get(header::RANGE)).await
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{header, Request, StatusCode},
        Router,
    };
    use tower::ServiceExt;

    use super::{hash_key, key_matches, random_secret};
    use crate::state::AppState;

    struct Shared {
        app: Router,
        state: AppState,
        _dir: tempfile::TempDir,
        token: String,
        key: String,
        media: String,
    }

    /// A shared document embedding one file.
    async fn shared() -> Shared {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("uploads")).unwrap();
        let state = crate::routes::tests::state_in(dir.path().into()).await;
        let (token, key) = (random_secret(), random_secret());
        let doc: i64 = sqlx::query_scalar(
            "INSERT INTO nodes (kind, name, slug, body, created_at, updated_at, share_token, share_key_hash)
             VALUES ('document', 'shared', 'shared', 'see', 1, 1, ?1, ?2) RETURNING id",
        )
        .bind(&token)
        .bind(hash_key(&key))
        .fetch_one(&state.db)
        .await
        .unwrap();
        let media = uuid::Uuid::new_v4().to_string();
        std::fs::write(state.config.uploads_dir().join(&media), b"picture").unwrap();
        sqlx::query("INSERT INTO media (id, filename, mime, size, created_at) VALUES (?1, 'a.png', 'image/png', 7, 1)")
            .bind(&media)
            .execute(&state.db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO node_media (node_id, media_id) VALUES (?1, ?2)")
            .bind(doc)
            .bind(&media)
            .execute(&state.db)
            .await
            .unwrap();
        let app = crate::routes::api_router(1024).with_state(state.clone());
        Shared {
            app,
            state,
            _dir: dir,
            token,
            key,
            media,
        }
    }

    impl Shared {
        async fn read(&self, key: &str) -> (StatusCode, Option<String>) {
            let request = Request::post(format!("/share/{}", self.token))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({ "key": key }).to_string()))
                .unwrap();
            let response = self.app.clone().oneshot(request).await.unwrap();
            let cookie = response
                .headers()
                .get(header::SET_COOKIE)
                .map(|v| v.to_str().unwrap().to_string());
            (response.status(), cookie)
        }

        async fn media(&self, cookie: Option<&str>) -> StatusCode {
            let mut request = Request::get(format!("/share/{}/media/{}", self.token, self.media));
            if let Some(cookie) = cookie {
                request = request.header(header::COOKIE, cookie);
            }
            let response = self
                .app
                .clone()
                .oneshot(request.body(Body::empty()).unwrap())
                .await;
            response.unwrap().status()
        }
    }

    #[tokio::test]
    async fn media_needs_the_cookie_reading_with_the_key_gave() {
        let s = shared().await;
        assert_eq!(
            s.read(&random_secret()).await,
            (StatusCode::NOT_FOUND, None)
        );

        let (status, set) = s.read(&s.key).await;
        assert_eq!(status, StatusCode::OK);
        let set = set.unwrap();
        assert!(
            set.contains(&format!("Path=/api/share/{}/", s.token)),
            "{set}"
        );
        assert!(
            set.contains("HttpOnly") && set.contains("SameSite=Strict"),
            "{set}"
        );
        let cookie = set.split(';').next().unwrap().to_string();

        assert_eq!(s.media(Some(&cookie)).await, StatusCode::OK);
        assert_eq!(s.media(None).await, StatusCode::NOT_FOUND);
        let (expires, _) = cookie.split_once('.').unwrap();
        let forged = format!("{expires}.{}", random_secret());
        assert_eq!(s.media(Some(&forged)).await, StatusCode::NOT_FOUND);

        // A new key, minted with `y`, voids the cookie the old one gave.
        sqlx::query("UPDATE nodes SET share_key_hash = ?1")
            .bind(hash_key(&random_secret()))
            .execute(&s.state.db)
            .await
            .unwrap();
        assert_eq!(s.media(Some(&cookie)).await, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn the_key_is_not_taken_from_the_path() {
        let s = shared().await;
        let request = Request::get(format!("/share/{}/{}", s.token, s.key))
            .body(Body::empty())
            .unwrap();
        let response = s.app.clone().oneshot(request).await.unwrap();
        assert!(response.status().is_client_error(), "{}", response.status());
        let old_media = format!("/share/{}/{}/media/{}", s.token, s.key, s.media);
        let response = s
            .app
            .clone()
            .oneshot(Request::get(old_media).body(Body::empty()).unwrap())
            .await;
        assert!(response.unwrap().status().is_client_error());
    }

    #[test]
    fn a_key_matches_only_its_own_hash() {
        let key = random_secret();
        assert!(key_matches(Some(&hash_key(&key)), &key));
        assert!(!key_matches(Some(&hash_key(&key)), &random_secret()));
        assert!(!key_matches(Some(&hash_key(&key)), ""));
    }

    #[test]
    fn the_stored_hash_is_not_the_key() {
        let key = random_secret();
        let stored = hash_key(&key);
        assert_ne!(stored, key);
        // The hash itself must not open the link either.
        assert!(!key_matches(Some(&stored), &stored));
    }

    #[test]
    fn a_share_without_a_hash_stays_shut() {
        assert!(!key_matches(None, &random_secret()));
        assert!(!key_matches(None, ""));
    }

    #[test]
    fn secrets_do_not_repeat() {
        assert_ne!(random_secret(), random_secret());
    }
}
