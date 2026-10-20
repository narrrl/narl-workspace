//! Talking to Proton: logging in once, resuming forever, owning one device.

use anyhow::{anyhow, Context, Result};
use proton_drive_rs::{Device, DeviceType, ProtonDriveClient};
use proton_sdk::account::AccountClient;
use proton_sdk::api::{HumanVerificationCredential, ResponseCode};
use proton_sdk::config::ProtonClientConfiguration;
use proton_sdk::error::ProtonError;
use proton_sdk::session::ProtonApiSession;
use zeroize::Zeroizing;

use super::session::{SessionStore, StoredSession};

/// What Proton asks a third-party client to tell anyone it takes credentials
/// from. Shown by `narl-workspace proton-login` and `:backup login` before
/// either asks for anything.
pub const DISCLOSURE: &str =
    "This is a third-party application not officially supported by Proton.";

/// How this client identifies itself in `x-pm-appversion`.
///
/// Proton's third-party rules require the shape
/// `external-drive-{name}@{semver}-{channel}`, where the name is lowercase
/// letters and underscores, and require that the value honestly describe the
/// application — a header that misrepresents its client, or imitates a
/// first-party Proton one, is a rule violation, not a cosmetic detail, and a
/// malformed one is answered with a 422 "unusual activity" that no login
/// survives. So: `narl_workspace` is the application, the version is the crate's
/// own, and the channel follows it — a `0.x` release is not a stable one, and
/// saying otherwise would be the same misrepresentation in miniature.
const APP_NAME: &str = "narl_workspace";

fn app_version() -> String {
    let version = env!("CARGO_PKG_VERSION");
    let channel = if version.starts_with("0.") {
        "alpha"
    } else {
        "stable"
    };
    format!("external-drive-{APP_NAME}@{version}-{channel}")
}

/// The identification headers. Overridable only for the case the shape itself
/// changes on Proton's side; the honest default is the one above.
fn client_config() -> ProtonClientConfiguration {
    let version = crate::config::var("PROTON_APP_VERSION").unwrap_or_else(app_version);
    let agent = format!("narl-workspace/{}", env!("CARGO_PKG_VERSION"));
    ProtonClientConfiguration::new(version).with_user_agent(agent)
}

/// How long a login may wait for its second factor or its human check. The
/// password is held in memory for that long and no longer.
const PENDING_FOR: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// A login half done, waiting on the person at the keyboard: for a
/// two-factor code, with the session Proton opened for it, or for a solved
/// human check, after which the login starts over with the proof attached.
pub struct Pending {
    username: String,
    password: Zeroizing<String>,
    session: Option<ProtonApiSession>,
    started: std::time::Instant,
}

impl Pending {
    pub fn expired(&self) -> bool {
        self.started.elapsed() > PENDING_FOR
    }
}

/// What a login needs next.
pub enum Next {
    /// A code from the account's authenticator app.
    Totp,
    /// Proton's human check, solved on the page at this address.
    Captcha(String),
}

/// Start a login: SRP with the password, and with a solved human check when
/// there is one. Stored and done when the account asks for nothing more;
/// otherwise what it asks for, and the login waiting on it.
pub async fn start(
    store: &SessionStore,
    username: &str,
    password: Zeroizing<String>,
    verified: Option<&HumanVerificationCredential>,
) -> Result<Option<(Next, Pending)>> {
    let begun =
        ProtonApiSession::begin_verified(client_config(), username, password.as_bytes(), verified)
            .await;
    let pending = |session| Pending {
        username: username.to_string(),
        password: password.clone(),
        session,
        started: std::time::Instant::now(),
    };
    let session = match begun {
        Ok(session) => session,
        Err(ProtonError::Api(e)) if e.is_human_verification_required() => {
            let Some(check) = e
                .human_verification()
                .filter(|check| check.supports_captcha())
            else {
                return Err(anyhow!(
                    "Proton wants a human check this server cannot show — log in once at \
                     proton.me from this network, then try again"
                ));
            };
            return Ok(Some((
                Next::Captcha(check.verification_url()),
                pending(None),
            )));
        }
        Err(ProtonError::Api(e)) if e.code == ResponseCode::IncorrectLoginCredentials => {
            return Err(anyhow!("Proton rejected the username or password"));
        }
        Err(e @ ProtonError::Transport(_)) => {
            return Err(anyhow::Error::from(e).context("could not reach Proton"))
        }
        Err(e) => return Err(anyhow::Error::from(e).context("Proton rejected the login")),
    };

    if session.is_waiting_for_second_factor() {
        if !session.second_factor_methods().totp {
            return Err(anyhow!(
                "this account's second factor is a security key only — add an authenticator \
                 app to it in Proton's settings, which this server can use"
            ));
        }
        return Ok(Some((Next::Totp, pending(Some(session)))));
    }
    finish(store, &session, &password).await?;
    Ok(None)
}

/// Answer a login's two-factor question. A wrong code gives the login back,
/// to be asked again.
pub async fn second_factor(
    store: &SessionStore,
    mut pending: Pending,
    code: &str,
) -> std::result::Result<(), (anyhow::Error, Option<Box<Pending>>)> {
    let Some(session) = pending.session.as_mut() else {
        return Err((
            anyhow!("this login is waiting for a human check, not a code"),
            Some(Box::new(pending)),
        ));
    };
    if let Err(e) = session.apply_second_factor_code(code.trim()).await {
        let e = anyhow::Error::from(e).context("Proton rejected the two-factor code");
        return Err((e, Some(Box::new(pending))));
    }
    let session = pending.session.take().expect("checked above");
    finish(store, &session, &pending.password)
        .await
        .map_err(|e| (e, None))
}

/// The login again from the start, now with a solved human check.
pub async fn verified(
    store: &SessionStore,
    pending: Pending,
    token: &str,
) -> Result<Option<(Next, Pending)>> {
    let proof = HumanVerificationCredential::captcha(token);
    start(store, &pending.username, pending.password, Some(&proof)).await
}

/// Store the session of a finished login.
async fn finish(store: &SessionStore, session: &ProtonApiSession, password: &str) -> Result<()> {
    // The passphrases are derived here and only here: this access token still
    // carries the `locked` scope the key salts need, and no refreshed one ever
    // will. Unlocking checks them against the account's keys before anything
    // is stored, so a wrong mailbox password fails now and not at the first
    // backup.
    let client = ProtonDriveClient::new(session, password.as_bytes().to_vec());
    let account = client.account();
    let key_passphrases = account
        .key_passphrases()
        .await
        .context("could not derive the account key passphrases")?;
    account.unlock().await.map_err(|e| match e {
        ProtonError::KeysLocked => anyhow!("that password does not unlock the account's keys"),
        e => anyhow::Error::from(e).context("could not unlock the account's keys"),
    })?;

    let tokens = session.current_tokens().await;
    store.save(&StoredSession::from_session(
        session,
        tokens,
        key_passphrases,
    ))?;
    Ok(())
}

/// Sign the stored session out on Proton's side as well, as far as Proton
/// answers; the caller forgets it locally whatever comes of this.
pub async fn end(store: &SessionStore) -> Result<()> {
    let Some(stored) = store.load()? else {
        return Ok(());
    };
    let session = ProtonApiSession::resume(client_config(), stored.resume_parameters())
        .context("the stored Proton session could not be resumed")?;
    session
        .end()
        .await
        .context("Proton did not end the session")?;
    Ok(())
}

/// Rebuild a Drive client from the stored session, or `None` when nothing has
/// been stored yet — that is the ordinary "backups are not set up" state, not
/// an error.
pub async fn resume(store: &SessionStore) -> Result<Option<ProtonDriveClient>> {
    let Some(mut stored) = store.load()? else {
        return Ok(None);
    };

    let session = ProtonApiSession::resume(client_config(), stored.resume_parameters())
        .context("the stored Proton session could not be resumed")?;

    if stored.key_passphrases.is_empty() {
        stored = upgrade(store, &session, stored).await?;
    }

    // Every 401 refresh mints a new refresh token and invalidates the old one,
    // so the rotation has to reach disk before the process can next restart.
    let store_for_hook = store.clone();
    let stored_for_hook = stored.clone();
    session.http().set_on_tokens_refreshed(move |tokens| {
        let refreshed = StoredSession {
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
            ..stored_for_hook.clone()
        };
        if let Err(e) = store_for_hook.save(&refreshed) {
            tracing::error!(error = %e, "could not persist refreshed Proton tokens — the next start will need a new login");
        }
    });

    let client = ProtonDriveClient::with_key_passphrases(&session, stored.key_passphrases)
        // A workspace is mostly small files; one atomic request beats the
        // draft/block/commit dance for anything that fits in a single block.
        .with_small_file_upload(true);

    Ok(Some(client))
}

/// Rewrite a session stored before the passphrases were, which kept the
/// mailbox password and the key salts instead. The salts are already known, so
/// the passphrases derive locally, with no request and no new login, and the
/// password is gone from the store once the new session is written.
async fn upgrade(
    store: &SessionStore,
    session: &ProtonApiSession,
    stored: StoredSession,
) -> Result<StoredSession> {
    let (Some(password), false) = (&stored.mailbox_password, stored.key_salts.is_empty()) else {
        return Err(anyhow!(
            "the stored Proton session has no key passphrases — run `narl-workspace proton-login` again"
        ));
    };

    let account = AccountClient::with_key_salts(
        session,
        password.clone().into_bytes(),
        stored.key_salts.clone(),
    );
    let key_passphrases = account
        .key_passphrases()
        .await
        .context("could not derive key passphrases from the stored session")?;

    let upgraded = StoredSession {
        key_passphrases,
        mailbox_password: None,
        key_salts: Vec::new(),
        ..stored
    };
    store.save(&upgraded)?;
    tracing::info!("the stored Proton session now keeps key passphrases instead of the password");
    Ok(upgraded)
}

/// The device this server owns, registered on first use.
///
/// A device is a sync root with its own share, which is what makes the workspace
/// appear in Proton Drive as a machine rather than as a folder someone dropped
/// in My Files — and keeps it out of the way of everything else in the account.
pub async fn ensure_device(client: &ProtonDriveClient, name: &str) -> Result<Device> {
    let devices = client
        .enumerate_devices()
        .await
        .context("could not list the account's devices")?;

    // A device whose name failed to decrypt is not ours to claim: adopting it
    // would mirror the workspace into a stranger of a folder.
    if let Some(device) = devices
        .into_iter()
        .find(|d| d.name.as_deref().map(|n| n == name).unwrap_or(false))
    {
        return Ok(device);
    }

    tracing::info!(device = name, "registering a new Proton Drive device");
    client
        .create_device(name, DeviceType::Linux)
        .await
        .with_context(|| format!("could not register the device {name:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No request leaves for this: resuming only builds the HTTP client, and
    /// with the salts already stored the passphrases derive locally.
    #[tokio::test]
    async fn an_old_session_trades_its_password_for_passphrases() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proton-session.json");
        std::fs::write(
            &path,
            r#"{"session_id":"s","username":"u","user_id":"i","access_token":"a",
               "refresh_token":"r","scopes":[],"password_mode":1,
               "mailbox_password":"hunter2hunter2",
               "key_salts":[{"ID":"key-1","KeySalt":"c2FsdHNhbHRzYWx0c2FsdA=="}]}"#,
        )
        .unwrap();
        let store = SessionStore::new(&path);
        let stored = store.load().unwrap().unwrap();
        let session =
            ProtonApiSession::resume(client_config(), stored.resume_parameters()).unwrap();

        let upgraded = upgrade(&store, &session, stored).await.unwrap();
        assert_eq!(upgraded.key_passphrases.len(), 1);
        assert!(upgraded.key_passphrases.get("key-1").is_some());

        let json = std::fs::read_to_string(&path).unwrap();
        assert!(!json.contains("hunter2"), "{json}");
        let reloaded = store.load().unwrap().unwrap();
        assert!(reloaded.key_passphrases == upgraded.key_passphrases);
    }

    #[tokio::test]
    async fn a_session_with_nothing_to_unlock_with_asks_for_a_login() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proton-session.json");
        std::fs::write(
            &path,
            r#"{"session_id":"s","username":"u","user_id":"i","access_token":"a",
               "refresh_token":"r","scopes":[],"password_mode":1}"#,
        )
        .unwrap();
        let store = SessionStore::new(&path);
        let stored = store.load().unwrap().unwrap();
        let session =
            ProtonApiSession::resume(client_config(), stored.resume_parameters()).unwrap();

        let error = upgrade(&store, &session, stored).await.err().unwrap();
        assert!(error.to_string().contains("proton-login"), "{error}");
    }

    #[test]
    fn identifies_itself_the_way_proton_requires() {
        let version = app_version();
        let (prefix, rest) = version.split_once('@').expect("name@version");
        // Lowercase letters and underscores after the fixed `external-drive-`.
        let name = prefix
            .strip_prefix("external-drive-")
            .expect("the required prefix");
        assert!(!name.is_empty());
        assert!(
            name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
            "{name}"
        );

        let (semver, channel) = rest.rsplit_once('-').expect("version-channel");
        assert!(matches!(channel, "stable" | "beta" | "alpha"), "{channel}");
        assert_eq!(semver.split('.').count(), 3, "{semver}");
        assert!(
            semver.split('.').all(|part| part.parse::<u32>().is_ok()),
            "{semver}"
        );
    }
}
