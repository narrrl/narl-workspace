//! Talking to Proton: logging in once, resuming forever, owning one device.

use anyhow::{anyhow, Context, Result};
use proton_drive_rs::{Device, DeviceType, ProtonDriveClient};
use proton_sdk::account::AccountClient;
use proton_sdk::config::ProtonClientConfiguration;
use proton_sdk::error::ProtonError;
use proton_sdk::session::ProtonApiSession;

use super::session::{SessionStore, StoredSession};

/// What Proton asks a third-party client to tell anyone it takes credentials
/// from. Shown by `narl-workspace proton-login` before it asks for anything.
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

/// Run an SRP login, complete 2FA if the account asks for one, and persist the
/// result. `get_totp` is only called when a second factor is actually required,
/// so accounts without one are never prompted.
pub async fn login(
    store: &SessionStore,
    username: &str,
    password: &str,
    get_totp: impl FnOnce() -> Result<String>,
) -> Result<()> {
    let mut session = ProtonApiSession::begin(client_config(), username, password.as_bytes())
        .await
        .context("Proton rejected the login")?;

    if session.is_waiting_for_second_factor() {
        let code = get_totp()?;
        session
            .apply_second_factor_code(code.trim())
            .await
            .context("Proton rejected the second-factor code")?;
    }

    // The passphrases are derived here and only here: this access token still
    // carries the `locked` scope the key salts need, and no refreshed one ever
    // will. Unlocking checks them against the account's keys before anything
    // is stored, so a wrong mailbox password fails now and not at the first
    // backup.
    let client = ProtonDriveClient::new(&session, password.as_bytes().to_vec());
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
        &session,
        tokens,
        key_passphrases,
    ))?;
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
