use std::{collections::HashMap, convert::Infallible, sync::Mutex};

use axum::{
    extract::{FromRequestParts, OptionalFromRequestParts, Request},
    http::{header, request::Parts, HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::{config::Config, error::AppError, state::AppState};

// Kept from before the rename: the name is never seen, and changing it would
// sign every browser out to no purpose.
pub const COOKIE_NAME: &str = "diary_session";
pub const SECURE_COOKIE_NAME: &str = "__Host-diary_session";

type HmacSha256 = Hmac<Sha256>;

pub(crate) fn sign(secret: &[u8], payload: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("hmac accepts any key length");
    mac.update(payload.as_bytes());
    B64.encode(mac.finalize().into_bytes())
}

/// A session token is `base64("{expires}:{epoch}:{id}").base64(hmac)`.
/// The signature keeps anyone from making one up; the id names the
/// [`Sessions`] row it belongs to, and a cookie is good only while that row
/// is there, it has not expired, and the epoch it was issued under is the
/// current one. Rotating `WORKSPACE_SECRET` still invalidates every session.
pub fn issue_token(expires: i64, epoch: i64, id: &str, config: &Config) -> String {
    let payload = B64.encode(format!("{expires}:{epoch}:{id}"));
    let signature = sign(&config.secret, &payload);
    format!("{payload}.{signature}")
}

/// When a session started now runs out.
pub fn session_expiry(config: &Config) -> i64 {
    crate::now() + config.session_days * 86_400
}

/// The session id a token carries, if it is genuine, unexpired and of this
/// epoch. A cookie from before sessions had ids carries none, and is refused.
pub fn verify_token(config: &Config, token: &str, epoch: i64) -> Option<String> {
    let (payload, signature) = token.split_once('.')?;
    let expected = sign(&config.secret, payload);
    if !bool::from(expected.as_bytes().ct_eq(signature.as_bytes())) {
        return None;
    }
    let decoded = String::from_utf8(B64.decode(payload).ok()?).ok()?;
    let mut fields = decoded.splitn(3, ':');
    let expires = fields.next()?.parse::<i64>().ok()?;
    let issued_in = fields.next()?.parse::<i64>().ok()?;
    let id = fields.next().filter(|id| !id.is_empty())?;
    (expires > crate::now() && issued_in == epoch).then(|| id.to_string())
}

/// How often a session that is in use gets its `last_seen` written down.
/// In between, it is kept in memory only.
const SEEN_EVERY: i64 = 300;

struct Live {
    expires_at: i64,
    last_seen: i64,
    written: i64,
}

/// The signed-in sessions, mirrored in memory from the `sessions` table so
/// a request costs no query to check.
#[derive(Default)]
pub struct Sessions {
    live: Mutex<HashMap<String, Live>>,
}

impl Sessions {
    pub async fn load(db: &sqlx::SqlitePool) -> anyhow::Result<Self> {
        let rows: Vec<(String, i64, i64)> =
            sqlx::query_as("SELECT id, expires_at, last_seen FROM sessions WHERE expires_at > ?1")
                .bind(crate::now())
                .fetch_all(db)
                .await?;
        let live = rows
            .into_iter()
            .map(|(id, expires_at, last_seen)| {
                let live = Live {
                    expires_at,
                    last_seen,
                    written: last_seen,
                };
                (id, live)
            })
            .collect();
        Ok(Self {
            live: Mutex::new(live),
        })
    }

    /// A new session, for a browser that just logged in.
    pub async fn open(
        &self,
        db: &sqlx::SqlitePool,
        user_agent: &str,
        expires_at: i64,
    ) -> anyhow::Result<String> {
        let id = B64.encode(rand::random::<[u8; 16]>());
        let now = crate::now();
        let user_agent: String = user_agent.chars().take(200).collect();
        sqlx::query(
            "INSERT INTO sessions (id, created_at, last_seen, expires_at, user_agent)
             VALUES (?1, ?2, ?2, ?3, ?4)",
        )
        .bind(&id)
        .bind(now)
        .bind(expires_at)
        .bind(&user_agent)
        .execute(db)
        .await?;
        let mut live = self.live.lock().expect("sessions lock");
        live.retain(|_, session| session.expires_at > now);
        live.insert(
            id.clone(),
            Live {
                expires_at,
                last_seen: now,
                written: now,
            },
        );
        Ok(id)
    }

    /// Whether `id` is signed in, noting that it was just seen. `Some(true)`
    /// asks the caller to write that down.
    fn seen(&self, id: &str) -> Option<bool> {
        let now = crate::now();
        let mut live = self.live.lock().expect("sessions lock");
        let session = live.get_mut(id).filter(|s| s.expires_at > now)?;
        session.last_seen = now;
        let due = now - session.written >= SEEN_EVERY;
        if due {
            session.written = now;
        }
        Some(due)
    }

    /// When `id` was last seen, fresher than the table may have it.
    pub fn last_seen(&self, id: &str) -> Option<i64> {
        let live = self.live.lock().expect("sessions lock");
        live.get(id).map(|session| session.last_seen)
    }

    /// Sign one session out. Says whether there was one to sign out.
    pub async fn end(&self, db: &sqlx::SqlitePool, id: &str) -> anyhow::Result<bool> {
        let removed = sqlx::query("DELETE FROM sessions WHERE id = ?1")
            .bind(id)
            .execute(db)
            .await?
            .rows_affected();
        let held = self
            .live
            .lock()
            .expect("sessions lock")
            .remove(id)
            .is_some();
        Ok(removed > 0 || held)
    }

    /// Sign every session out.
    pub async fn end_all(&self, db: &sqlx::SqlitePool) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM sessions").execute(db).await?;
        self.live.lock().expect("sessions lock").clear();
        Ok(())
    }
}

/// The session epoch: what every cookie must have been signed under.
pub struct Epoch;

impl Epoch {
    /// A keyed hash of the login password, so a change to it can be told
    /// from a restart without the password being written down anywhere.
    fn password_hash(config: &Config) -> String {
        sign(&config.secret, &format!("password:{}", config.password))
    }

    /// The epoch to start with: the stored one, raised when the password
    /// is not the one it was stored with.
    pub async fn load(db: &sqlx::SqlitePool, config: &Config) -> anyhow::Result<i64> {
        let hash = Self::password_hash(config);
        let stored: Option<(i64, String)> =
            sqlx::query_as("SELECT epoch, password_hash FROM auth WHERE id = 1")
                .fetch_optional(db)
                .await?;
        let epoch = match stored {
            None => 0,
            Some((epoch, previous)) if previous == hash => return Ok(epoch),
            Some((epoch, _)) => {
                tracing::info!("the login password changed: every device is signed out");
                sqlx::query("DELETE FROM sessions").execute(db).await?;
                epoch + 1
            }
        };
        sqlx::query(
            "INSERT INTO auth (id, epoch, password_hash) VALUES (1, ?1, ?2)
             ON CONFLICT (id) DO UPDATE SET epoch = excluded.epoch, password_hash = excluded.password_hash",
        )
        .bind(epoch)
        .bind(&hash)
        .execute(db)
        .await?;
        Ok(epoch)
    }

    /// Sign every device out: the epoch moves on, no cookie has it yet, and
    /// no session row is left either.
    pub async fn advance(state: &AppState) -> anyhow::Result<i64> {
        state.sessions.end_all(&state.db).await?;
        let epoch: i64 =
            sqlx::query_scalar("UPDATE auth SET epoch = epoch + 1 WHERE id = 1 RETURNING epoch")
                .fetch_one(&state.db)
                .await?;
        state
            .epoch
            .store(epoch, std::sync::atomic::Ordering::SeqCst);
        Ok(epoch)
    }
}

/// Both sides are compared as MACs of the same length, so the time it takes
/// says nothing about how long the real password is either.
pub fn credentials_match(config: &Config, username: &str, password: &str) -> bool {
    let digest = |value: &str| {
        let mut mac =
            HmacSha256::new_from_slice(&config.secret).expect("hmac accepts any key length");
        mac.update(value.as_bytes());
        mac.finalize().into_bytes()
    };
    let user_ok = digest(username).ct_eq(&digest(&config.username));
    let pass_ok = digest(password).ct_eq(&digest(&config.password));
    bool::from(user_ok & pass_ok)
}

/// Over HTTPS the cookie takes the `__Host-` prefix: the browser then keeps
/// it to this exact host, sent only over a secure connection, and no sibling
/// subdomain can plant one of the same name.
pub fn cookie_name(config: &Config) -> &'static str {
    if config.secure_cookie {
        SECURE_COOKIE_NAME
    } else {
        COOKIE_NAME
    }
}

pub fn session_cookie(config: &Config, token: &str) -> String {
    let max_age = config.session_days * 86_400;
    let name = cookie_name(config);
    let secure = if config.secure_cookie { "; Secure" } else { "" };
    format!("{name}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}")
}

pub fn clear_cookie(config: &Config) -> String {
    let name = cookie_name(config);
    let secure = if config.secure_cookie { "; Secure" } else { "" };
    format!("{name}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{secure}")
}

pub fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| k.trim() == name)
        .map(|(_, v)| v.trim())
}

/// The cookie a browser keeps once it has logged in, so the login can tell
/// it from a stranger; see [`crate::throttle::LoginThrottles`]. It opens
/// nothing, outlives logging out, and only ever picks a throttle.
const DEVICE_COOKIE: &str = "diary_device";
const SECURE_DEVICE_COOKIE: &str = "__Host-diary_device";
const DEVICE_DAYS: i64 = 400;

fn device_cookie_name(config: &Config) -> &'static str {
    if config.secure_cookie {
        SECURE_DEVICE_COOKIE
    } else {
        DEVICE_COOKIE
    }
}

/// The device id a request's cookie carries, if it is one this server signed.
pub fn device_id(config: &Config, headers: &HeaderMap) -> Option<String> {
    let (id, signature) = cookie_value(headers, device_cookie_name(config))?.split_once('.')?;
    let expected = sign(&config.secret, &format!("device:{id}"));
    bool::from(expected.as_bytes().ct_eq(signature.as_bytes())).then(|| id.to_string())
}

/// A fresh device id, for a browser that logged in without one.
pub fn new_device_id() -> String {
    B64.encode(rand::random::<[u8; 16]>())
}

pub fn device_cookie(config: &Config, id: &str) -> String {
    let name = device_cookie_name(config);
    let signature = sign(&config.secret, &format!("device:{id}"));
    let max_age = DEVICE_DAYS * 86_400;
    let secure = if config.secure_cookie { "; Secure" } else { "" };
    format!("{name}={id}.{signature}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}{secure}")
}

/// Extractor that rejects the request unless a valid session cookie is present.
pub struct Session {
    /// The `sessions` row the cookie belongs to.
    pub id: String,
}

impl FromRequestParts<AppState> for Session {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = cookie_value(&parts.headers, cookie_name(&state.config))
            .ok_or(AppError::Unauthorized)?;
        let epoch = state.epoch.load(std::sync::atomic::Ordering::SeqCst);
        let id = verify_token(&state.config, token, epoch).ok_or(AppError::Unauthorized)?;
        let due = state.sessions.seen(&id).ok_or(AppError::Unauthorized)?;
        if due {
            let (db, id) = (state.db.clone(), id.clone());
            tokio::spawn(async move {
                let written = sqlx::query("UPDATE sessions SET last_seen = ?1 WHERE id = ?2")
                    .bind(crate::now())
                    .bind(&id)
                    .execute(&db)
                    .await;
                if let Err(e) = written {
                    tracing::warn!("could not note a session as seen: {e}");
                }
            });
        }
        Ok(Session { id })
    }
}

/// For a handler that behaves differently for a caller who is logged in, but
/// must also answer one who is not.
impl OptionalFromRequestParts<AppState> for Session {
    type Rejection = Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Option<Self>, Self::Rejection> {
        Ok(
            <Session as FromRequestParts<AppState>>::from_request_parts(parts, state)
                .await
                .ok(),
        )
    }
}

/// Refuse a write that another site's page asked the browser to make.
/// `SameSite=Lax` already keeps the cookie off a cross-site POST, but not off
/// one from a sibling subdomain, which counts as the same site — and a
/// multipart form needs no preflight to get sent. A browser says where a
/// request came from; a request that says nothing is not from a browser page
/// and carries no ambient cookie worth stealing.
pub async fn same_origin(request: Request, next: Next) -> Response {
    if !request.method().is_safe() && !is_same_origin(request.headers()) {
        tracing::warn!(
            "refused a cross-origin {} {}",
            request.method(),
            request.uri().path()
        );
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "cross-origin request refused" })),
        )
            .into_response();
    }
    next.run(request).await
}

fn is_same_origin(headers: &HeaderMap) -> bool {
    // Every current browser sends this, and it cannot be set from a page.
    if let Some(site) = headers.get("sec-fetch-site") {
        return matches!(site.as_bytes(), b"same-origin" | b"none");
    }
    // Older browsers: the Origin they send against the host they asked.
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let origin_host = origin
        .to_str()
        .ok()
        .and_then(|origin| origin.split_once("://"))
        .map(|(_, host)| host);
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok());
    matches!((origin_host, host), (Some(a), Some(b)) if a.eq_ignore_ascii_case(b))
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue};

    use super::{
        clear_cookie, credentials_match, is_same_origin, issue_token, session_cookie,
        session_expiry, verify_token, Sessions,
    };
    use crate::config::Config;

    fn config(session_days: i64) -> Config {
        Config::for_tests(session_days)
    }

    fn token(config: &Config, epoch: i64) -> String {
        issue_token(session_expiry(config), epoch, "sid", config)
    }

    #[test]
    fn a_fresh_token_verifies() {
        let config = config(30);
        assert_eq!(
            verify_token(&config, &token(&config, 0), 0).as_deref(),
            Some("sid")
        );
    }

    #[test]
    fn an_expired_token_does_not() {
        let config = config(-1);
        assert!(verify_token(&config, &token(&config, 0), 0).is_none());
    }

    #[test]
    fn rotating_the_secret_invalidates_every_session() {
        let token = token(&config(30), 0);
        let mut rotated = config(30);
        rotated.secret = b"a-different-secret-entirely".to_vec();
        assert!(verify_token(&rotated, &token, 0).is_none());
    }

    #[test]
    fn over_https_the_cookie_is_kept_to_the_host() {
        let mut config = config(30);
        assert!(session_cookie(&config, "t").starts_with("diary_session=t;"));
        config.secure_cookie = true;
        let cookie = session_cookie(&config, "t");
        assert!(cookie.starts_with("__Host-diary_session=t;"), "{cookie}");
        // The prefix holds only with Secure, Path=/ and no Domain.
        assert!(
            cookie.contains("; Secure") && cookie.contains("Path=/") && !cookie.contains("Domain")
        );
        assert!(clear_cookie(&config).starts_with("__Host-diary_session=;"));
    }

    #[test]
    fn a_tampered_token_does_not_verify() {
        let config = config(30);
        let token = token(&config, 0);
        let (payload, signature) = token.split_once('.').unwrap();

        // A far-future expiry the holder signed themselves.
        let forged = base64::Engine::encode(&super::B64, "99999999999:0:sid");
        assert!(verify_token(&config, &format!("{forged}.{signature}"), 0).is_none());

        // The real expiry with the last byte of the signature flipped.
        let mut bytes = signature.as_bytes().to_vec();
        bytes[0] = if bytes[0] == b'A' { b'B' } else { b'A' };
        let flipped = String::from_utf8(bytes).unwrap();
        assert!(verify_token(&config, &format!("{payload}.{flipped}"), 0).is_none());
    }

    #[test]
    fn malformed_tokens_are_rejected_rather_than_panicking() {
        let config = config(30);
        for token in ["", ".", "no-dot", "!!!.!!!", "a.b.c", &".".repeat(100)] {
            assert!(verify_token(&config, token, 0).is_none(), "{token:?}");
        }
    }

    #[test]
    fn credentials_must_match_exactly() {
        let config = config(30);
        assert!(credentials_match(&config, "narl", "hunter2"));
        assert!(!credentials_match(&config, "narl", "hunter"));
        assert!(!credentials_match(&config, "narl", "hunter22"));
        assert!(!credentials_match(&config, "nar", "hunter2"));
        assert!(!credentials_match(&config, "", ""));
    }

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, HeaderValue::from_static(value));
        }
        map
    }

    #[test]
    fn only_same_origin_writes_get_through() {
        assert!(is_same_origin(&headers(&[(
            "sec-fetch-site",
            "same-origin"
        )])));
        assert!(!is_same_origin(&headers(&[(
            "sec-fetch-site",
            "same-site"
        )])));
        assert!(!is_same_origin(&headers(&[(
            "sec-fetch-site",
            "cross-site"
        )])));

        let host = ("host", "diary.example.org");
        assert!(is_same_origin(&headers(&[
            ("origin", "https://diary.example.org"),
            host
        ])));
        assert!(!is_same_origin(&headers(&[
            ("origin", "https://evil.example.org"),
            host
        ])));
        assert!(!is_same_origin(&headers(&[("origin", "null"), host])));

        // curl, the CLI, a script with its own credentials.
        assert!(is_same_origin(&headers(&[host])));
    }

    #[test]
    fn a_cookie_from_an_earlier_epoch_is_refused() {
        let config = config(30);
        let token = token(&config, 3);
        assert!(verify_token(&config, &token, 3).is_some());
        assert!(verify_token(&config, &token, 4).is_none());
    }

    #[test]
    fn only_a_device_cookie_this_server_signed_counts() {
        use axum::http::{header, HeaderMap};
        let config = config(30);
        let with = |cookie: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(header::COOKIE, cookie.parse().unwrap());
            super::device_id(&config, &headers)
        };
        let set = super::device_cookie(&config, "laptop");
        let pair = set.split(';').next().unwrap();
        assert_eq!(with(pair).as_deref(), Some("laptop"));
        assert_eq!(with("diary_device=laptop"), None);
        assert_eq!(with(&pair.replace("laptop.", "phone.")), None);

        let mut other = Config::for_tests(30);
        other.secret = b"a-different-secret-entirely".to_vec();
        let foreign = super::device_cookie(&other, "laptop");
        assert_eq!(with(foreign.split(';').next().unwrap()), None);
    }

    #[test]
    fn a_cookie_without_a_session_id_is_refused() {
        let config = config(30);
        let expires = crate::now() + 3600;
        // From before epochs, and from before session rows.
        for payload in [
            expires.to_string(),
            format!("{expires}:0"),
            format!("{expires}:0:"),
        ] {
            let payload = base64::Engine::encode(&super::B64, payload);
            let legacy = format!("{payload}.{}", super::sign(&config.secret, &payload));
            assert!(verify_token(&config, &legacy, 0).is_none(), "{payload}");
        }
    }

    async fn db() -> sqlx::SqlitePool {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&db).await.unwrap();
        db
    }

    #[tokio::test]
    async fn one_session_ends_and_the_others_stay() {
        let db = db().await;
        let sessions = Sessions::default();
        let expires = crate::now() + 3600;
        let phone = sessions.open(&db, "phone", expires).await.unwrap();
        let laptop = sessions.open(&db, "laptop", expires).await.unwrap();
        assert_ne!(phone, laptop);
        assert_eq!(sessions.seen(&phone), Some(false));

        assert!(sessions.end(&db, &phone).await.unwrap());
        assert_eq!(sessions.seen(&phone), None);
        assert!(sessions.seen(&laptop).is_some());
        assert!(!sessions.end(&db, &phone).await.unwrap());

        // What is kept in memory is what a restart reads back.
        let reloaded = Sessions::load(&db).await.unwrap();
        assert_eq!(reloaded.seen(&phone), None);
        assert!(reloaded.seen(&laptop).is_some());

        sessions.end_all(&db).await.unwrap();
        assert_eq!(sessions.seen(&laptop), None);
        let left: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions")
            .fetch_one(&db)
            .await
            .unwrap();
        assert_eq!(left, 0);
    }

    #[tokio::test]
    async fn an_expired_session_is_not_signed_in() {
        let db = db().await;
        let sessions = Sessions::default();
        let gone = sessions.open(&db, "old", crate::now() - 1).await.unwrap();
        assert_eq!(sessions.seen(&gone), None);
        assert!(Sessions::load(&db).await.unwrap().seen(&gone).is_none());
    }
}
