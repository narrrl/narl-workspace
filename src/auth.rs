use std::convert::Infallible;

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

type HmacSha256 = Hmac<Sha256>;

fn sign(secret: &[u8], payload: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("hmac accepts any key length");
    mac.update(payload.as_bytes());
    B64.encode(mac.finalize().into_bytes())
}

/// A session token is `base64(expiry_unix_seconds).base64(hmac)`. There is no
/// server-side session store: rotating `WORKSPACE_SECRET` invalidates every session.
/// A cookie is good until it expires and for as long as the epoch it was
/// issued under is the current one.
pub fn issue_token(config: &Config, epoch: i64) -> String {
    let expires = crate::now() + config.session_days * 86_400;
    let payload = B64.encode(format!("{expires}:{epoch}"));
    let signature = sign(&config.secret, &payload);
    format!("{payload}.{signature}")
}

pub fn verify_token(config: &Config, token: &str, epoch: i64) -> bool {
    let Some((payload, signature)) = token.split_once('.') else {
        return false;
    };
    let expected = sign(&config.secret, payload);
    if !bool::from(expected.as_bytes().ct_eq(signature.as_bytes())) {
        return false;
    }
    let Ok(decoded) = B64.decode(payload) else {
        return false;
    };
    let decoded = String::from_utf8_lossy(&decoded);
    // A cookie from before epochs existed carries only the expiry, and
    // belongs to the first epoch.
    let (expires, issued_in) = decoded.split_once(':').unwrap_or((&decoded, "0"));
    let (Ok(expires), Ok(issued_in)) = (expires.parse::<i64>(), issued_in.parse::<i64>()) else {
        return false;
    };
    expires > crate::now() && issued_in == epoch
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

    /// Sign every device out: the epoch moves on, and no cookie has it yet.
    pub async fn advance(state: &AppState) -> anyhow::Result<i64> {
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

pub fn credentials_match(config: &Config, username: &str, password: &str) -> bool {
    let user_ok = username.as_bytes().ct_eq(config.username.as_bytes());
    let pass_ok = password.as_bytes().ct_eq(config.password.as_bytes());
    bool::from(user_ok & pass_ok)
}

pub fn session_cookie(config: &Config, token: &str) -> String {
    let max_age = config.session_days * 86_400;
    let secure = if config.secure_cookie { "; Secure" } else { "" };
    format!("{COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}")
}

pub fn clear_cookie(config: &Config) -> String {
    let secure = if config.secure_cookie { "; Secure" } else { "" };
    format!("{COOKIE_NAME}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{secure}")
}

fn cookie_value<'a>(parts: &'a Parts, name: &str) -> Option<&'a str> {
    parts
        .headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| k.trim() == name)
        .map(|(_, v)| v.trim())
}

/// Extractor that rejects the request unless a valid session cookie is present.
pub struct Session;

impl FromRequestParts<AppState> for Session {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = cookie_value(parts, COOKIE_NAME).ok_or(AppError::Unauthorized)?;
        let epoch = state.epoch.load(std::sync::atomic::Ordering::SeqCst);
        if verify_token(&state.config, token, epoch) {
            Ok(Session)
        } else {
            Err(AppError::Unauthorized)
        }
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

    use super::{credentials_match, is_same_origin, issue_token, verify_token};
    use crate::config::{BackupConfig, Config, MailConfig, SftpConfig};

    fn config(session_days: i64) -> Config {
        Config {
            username: "narl".into(),
            password: "hunter2".into(),
            secret: b"a-secret-of-at-least-16-bytes".to_vec(),
            bind: "127.0.0.1:0".parse().unwrap(),
            data_dir: ".".into(),
            session_days,
            max_upload_bytes: 1,
            secure_cookie: false,
            vault_idle: std::time::Duration::from_secs(60),
            backup: BackupConfig {
                device_name: "narl-diary".into(),
                interval: None,
                debounce: std::time::Duration::from_secs(0),
                prune: false,
                session_key_file: None,
            },
            mail: MailConfig {
                url: None,
                from: String::new(),
                to: String::new(),
                timezone: chrono_tz::UTC,
                reminder_at: None,
                card_due_at: None,
                digest_at: None,
                kinds: Vec::new(),
            },
            sftp: SftpConfig { bind: None },
        }
    }

    #[test]
    fn a_fresh_token_verifies() {
        let config = config(30);
        assert!(verify_token(&config, &issue_token(&config, 0), 0));
    }

    #[test]
    fn an_expired_token_does_not() {
        let config = config(-1);
        assert!(!verify_token(&config, &issue_token(&config, 0), 0));
    }

    #[test]
    fn rotating_the_secret_invalidates_every_session() {
        let token = issue_token(&config(30), 0);
        let mut rotated = config(30);
        rotated.secret = b"a-different-secret-entirely".to_vec();
        assert!(!verify_token(&rotated, &token, 0));
    }

    #[test]
    fn a_tampered_token_does_not_verify() {
        let config = config(30);
        let token = issue_token(&config, 0);
        let (payload, signature) = token.split_once('.').unwrap();

        // A far-future expiry the holder signed themselves.
        let forged = base64::Engine::encode(&super::B64, "99999999999");
        assert!(!verify_token(&config, &format!("{forged}.{signature}"), 0));

        // The real expiry with the last byte of the signature flipped.
        let mut bytes = signature.as_bytes().to_vec();
        bytes[0] = if bytes[0] == b'A' { b'B' } else { b'A' };
        let flipped = String::from_utf8(bytes).unwrap();
        assert!(!verify_token(&config, &format!("{payload}.{flipped}"), 0));
    }

    #[test]
    fn malformed_tokens_are_rejected_rather_than_panicking() {
        let config = config(30);
        for token in ["", ".", "no-dot", "!!!.!!!", "a.b.c", &".".repeat(100)] {
            assert!(!verify_token(&config, token, 0), "{token:?}");
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
        let token = issue_token(&config, 3);
        assert!(verify_token(&config, &token, 3));
        assert!(!verify_token(&config, &token, 4));

        // Issued before epochs: only the expiry, and good in the first one.
        let expires = crate::now() + 3600;
        let payload = base64::Engine::encode(&super::B64, expires.to_string());
        let legacy = format!("{payload}.{}", super::sign(&config.secret, &payload));
        assert!(verify_token(&config, &legacy, 0));
        assert!(!verify_token(&config, &legacy, 1));
    }
}
