//! The stored Proton session: everything needed to come back up unattended.
//!
//! A login is worth persisting whole, because none of it can be re-derived
//! without a human: the tokens (the session itself) and the key passphrases
//! (which rebuild the key chain on every start — the SDK needs them in memory
//! to decrypt anything).
//!
//! The passphrases are what the password is for: each is
//! `bcrypt(mailbox password, key salt)` and unlocks one account key. Keeping
//! them instead of the password means the store never holds the password
//! itself, which on a single-password account is the login password. They
//! read the account all the same, so they are no less secret — but they cannot
//! log in, change the password or turn 2FA off.
//!
//! They are derived at login and only then. Deriving needs the key salts, and
//! `core/v4/keys/salts` requires the `locked` scope, which only the access
//! token minted by a *password* login carries; once that token has been rotated
//! through `auth/v4/refresh` — as it is on every restart after the first — the
//! endpoint answers 403.
//!
//! Refresh tokens are single-use: a rotation that is not written back leaves a
//! dead token on disk and the next resume fails. Hence the refresh hook in
//! [`super::proton`], which persists on every rotation.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Nonce,
};
use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use proton_drive_rs::KeySalt;
use proton_sdk::account::KeyPassphrases;
use proton_sdk::session::{PasswordMode, ProtonApiSession, ResumeParameters};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::config::Config;

#[derive(Serialize, Deserialize, Clone)]
pub struct StoredSession {
    pub session_id: String,
    pub username: String,
    pub user_id: String,
    pub access_token: String,
    pub refresh_token: String,
    pub scopes: Vec<String>,
    /// `1` = single password, `2` = dual (Proton's wire values).
    pub password_mode: u8,
    /// Key id → the passphrase that unlocks it, derived at login. Empty only
    /// in a session written before they were, which still has the two fields
    /// below.
    #[serde(default, with = "passphrases")]
    pub key_passphrases: KeyPassphrases,
    /// What a session stored before the passphrases kept instead of them.
    /// Read once, to derive the passphrases, and never written again.
    #[serde(default, skip_serializing)]
    pub mailbox_password: Option<String>,
    #[serde(default, skip_serializing)]
    pub key_salts: Vec<KeySalt>,
}

impl StoredSession {
    pub fn from_session(
        session: &ProtonApiSession,
        tokens: proton_sdk::http::Tokens,
        key_passphrases: KeyPassphrases,
    ) -> Self {
        Self {
            session_id: session.session_id().as_str().to_owned(),
            username: session.username().to_owned(),
            user_id: session.user_id().as_str().to_owned(),
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
            scopes: session.scopes().to_vec(),
            password_mode: match session.password_mode() {
                PasswordMode::Single => 1,
                PasswordMode::Dual => 2,
            },
            key_passphrases,
            mailbox_password: None,
            key_salts: Vec::new(),
        }
    }

    pub fn resume_parameters(&self) -> ResumeParameters {
        ResumeParameters {
            session_id: self.session_id.clone().into(),
            username: self.username.clone(),
            user_id: self.user_id.clone().into(),
            access_token: self.access_token.clone(),
            refresh_token: self.refresh_token.clone(),
            scopes: self.scopes.clone(),
            is_waiting_for_second_factor_code: false,
            password_mode: match self.password_mode {
                1 => PasswordMode::Single,
                _ => PasswordMode::Dual,
            },
        }
    }
}

/// Passphrases are bytes; the session file is JSON. Each one is kept as
/// base64 under its key id.
mod passphrases {
    use std::collections::BTreeMap;

    use base64::{engine::general_purpose::STANDARD, Engine};
    use proton_sdk::account::KeyPassphrases;
    use serde::{de::Error, Deserialize, Deserializer, Serializer};
    use zeroize::Zeroizing;

    pub fn serialize<S: Serializer>(value: &KeyPassphrases, out: S) -> Result<S::Ok, S::Error> {
        out.collect_map(
            value
                .iter()
                .map(|(id, passphrase)| (id, STANDARD.encode(passphrase))),
        )
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(input: D) -> Result<KeyPassphrases, D::Error> {
        let encoded = BTreeMap::<String, Zeroizing<String>>::deserialize(input)?;
        let mut passphrases = KeyPassphrases::new();
        for (id, passphrase) in encoded {
            let bytes = STANDARD
                .decode(passphrase.as_bytes())
                .map_err(D::Error::custom)?;
            passphrases.insert(id, bytes);
        }
        Ok(passphrases)
    }
}

/// Where a session is kept.
///
/// The OS keyring is the right home for this and is what the desktop client
/// uses — but it is a Secret Service on D-Bus, and the workspace's supported
/// deployment is a container, where there is neither. So the default is a
/// `0600` file next to the database: on the same volume as the workspace it is
/// protecting, and no weaker than the thing it sits beside. Build with
/// `--features keyring` on a host that has a session bus to use the keyring
/// instead, with the file as the fallback when it cannot be reached.
///
/// What a keyring adds is that a copy of the volume does not carry the
/// session. `WORKSPACE_SESSION_KEY_FILE` gives a container the same: the file
/// is sealed with AES-256-GCM under a key that lives somewhere else, a Docker
/// secret being the obvious place. A plain file found while a key is set is
/// sealed the first time it is read.
#[derive(Clone)]
pub struct SessionStore {
    path: PathBuf,
    key: Option<Arc<Zeroizing<[u8; 32]>>>,
}

/// Starts a sealed session file. Nothing that is JSON can start with it.
const MAGIC: &[u8; 8] = b"NARLSES1";
const NONCE: usize = 12;

impl SessionStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            key: None,
        }
    }

    /// The store the configuration describes: the usual path, sealed when a
    /// key file is set. A key file that is missing or malformed is an error
    /// rather than a quiet fall back to plain JSON.
    pub fn open(config: &Config) -> Result<Self> {
        let mut store = Self::new(config.proton_session_path());
        if let Some(file) = &config.backup.session_key_file {
            store.key = Some(Arc::new(read_key(file)?));
        }
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Option<StoredSession>> {
        #[cfg(feature = "keyring")]
        if let Some(stored) = keyring_load()? {
            return Ok(Some(stored));
        }

        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => Zeroizing::new(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(e).with_context(|| format!("could not read {}", self.path.display()))
            }
        };

        let sealed = bytes.starts_with(MAGIC);
        let json = match (sealed, &self.key) {
            (true, Some(key)) => open(key, &bytes).with_context(|| {
                format!(
                    "{} does not open with WORKSPACE_SESSION_KEY_FILE — was the key replaced?",
                    self.path.display()
                )
            })?,
            (true, None) => bail!(
                "{} is sealed — set WORKSPACE_SESSION_KEY_FILE to the key it was sealed with",
                self.path.display()
            ),
            (false, _) => bytes,
        };
        let stored = serde_json::from_slice(&json).with_context(|| {
            format!(
                "{} is not a session this build understands",
                self.path.display()
            )
        })?;

        if !sealed && self.key.is_some() {
            self.save(&stored)?;
            tracing::info!(
                "the stored Proton session is now sealed under WORKSPACE_SESSION_KEY_FILE"
            );
        }
        Ok(Some(stored))
    }

    pub fn save(&self, stored: &StoredSession) -> Result<()> {
        let json = Zeroizing::new(serde_json::to_vec(stored)?);

        #[cfg(feature = "keyring")]
        if keyring_save(std::str::from_utf8(&json)?).is_ok() {
            return Ok(());
        }

        match &self.key {
            Some(key) => write_private(&self.path, &seal(key, &json)),
            None => write_private(&self.path, &json),
        }
    }

    pub fn clear(&self) -> Result<()> {
        #[cfg(feature = "keyring")]
        let _ = keyring_clear();

        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).with_context(|| format!("could not remove {}", self.path.display())),
        }
    }
}

/// 32 random bytes, base64 — what `openssl rand -base64 32` writes.
fn read_key(file: &Path) -> Result<Zeroizing<[u8; 32]>> {
    let text = Zeroizing::new(
        std::fs::read_to_string(file)
            .with_context(|| format!("could not read the session key {}", file.display()))?,
    );
    let bytes = Zeroizing::new(STANDARD.decode(text.trim()).unwrap_or_default());
    let key: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
        anyhow!(
            "{} must hold 32 random bytes in base64 — `openssl rand -base64 32` makes one",
            file.display()
        )
    })?;
    Ok(Zeroizing::new(key))
}

/// `MAGIC ‖ nonce ‖ ciphertext`, with the magic bound in as associated data.
fn seal(key: &[u8; 32], json: &[u8]) -> Vec<u8> {
    let mut nonce = [0u8; NONCE];
    rand::thread_rng().fill_bytes(&mut nonce);
    let ciphertext = Aes256Gcm::new(key.into())
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: json,
                aad: MAGIC,
            },
        )
        .expect("AES-GCM seals any input this size");

    let mut out = Vec::with_capacity(MAGIC.len() + NONCE + ciphertext.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    out
}

fn open(key: &[u8; 32], sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    let rest = &sealed[MAGIC.len()..];
    if rest.len() < NONCE {
        bail!("the sealed session is cut short");
    }
    let (nonce, ciphertext) = rest.split_at(NONCE);
    Aes256Gcm::new(key.into())
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad: MAGIC,
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| anyhow!("the key does not open it, or it was altered"))
}

/// Write `bytes` to `path` as `0600`, and atomically: a crash mid-write must
/// not leave a truncated session behind, because a half-written refresh token
/// is indistinguishable from a revoked one and costs an interactive re-login.
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;

    let temporary = path.with_extension("tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options
        .open(&temporary)
        .with_context(|| format!("could not write {}", temporary.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);

    std::fs::rename(&temporary, path)
        .with_context(|| format!("could not replace {}", path.display()))?;
    Ok(())
}

#[cfg(feature = "keyring")]
// Names an entry that may already exist in the OS keyring; renaming it would
// lose the session it points at.
const KEYRING_SERVICE: &str = "narl-diary";
#[cfg(feature = "keyring")]
const KEYRING_USER: &str = "proton-session";

#[cfg(feature = "keyring")]
fn keyring_load() -> Result<Option<StoredSession>> {
    match keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).and_then(|e| e.get_password()) {
        Ok(json) => Ok(Some(serde_json::from_str(&json)?)),
        // No entry, no keyring, locked keyring: all mean "ask the file".
        Err(_) => Ok(None),
    }
}

#[cfg(feature = "keyring")]
fn keyring_save(json: &str) -> Result<()> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)?.set_password(json)?;
    Ok(())
}

#[cfg(feature = "keyring")]
fn keyring_clear() -> Result<()> {
    match keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map(|e| e.delete_credential()) {
        Ok(Ok(())) | Ok(Err(keyring::Error::NoEntry)) | Err(_) => Ok(()),
        Ok(Err(e)) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(key_passphrases: KeyPassphrases) -> StoredSession {
        StoredSession {
            session_id: "session".into(),
            username: "someone".into(),
            user_id: "user".into(),
            access_token: "access".into(),
            refresh_token: "refresh".into(),
            scopes: vec!["full".into()],
            password_mode: 1,
            key_passphrases,
            mailbox_password: None,
            key_salts: Vec::new(),
        }
    }

    #[test]
    fn passphrases_survive_the_file_and_the_password_never_reaches_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().join("proton-session.json"));
        let passphrases = KeyPassphrases::from_iter([
            ("key-1", b"one\0\xff".to_vec()),
            ("key-2", b"two".to_vec()),
        ]);

        // Even a session that still holds the old fields in memory writes
        // neither of them.
        let mut session = stored(passphrases.clone());
        session.mailbox_password = Some("hunter2hunter2".into());
        store.save(&session).unwrap();

        let json = std::fs::read_to_string(store.path()).unwrap();
        assert!(!json.contains("hunter2"), "{json}");
        assert!(!json.contains("mailbox_password"), "{json}");
        assert!(!json.contains("key_salts"), "{json}");

        let loaded = store.load().unwrap().unwrap();
        assert!(loaded.key_passphrases == passphrases);
        assert!(loaded.mailbox_password.is_none());
    }

    #[test]
    fn a_session_from_before_the_passphrases_still_loads() {
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

        let loaded = SessionStore::new(path).load().unwrap().unwrap();
        assert!(loaded.key_passphrases.is_empty());
        assert_eq!(loaded.mailbox_password.as_deref(), Some("hunter2hunter2"));
        assert_eq!(loaded.key_salts.len(), 1);
    }

    fn keyed(path: &Path, key: [u8; 32]) -> SessionStore {
        SessionStore {
            path: path.to_owned(),
            key: Some(Arc::new(Zeroizing::new(key))),
        }
    }

    #[test]
    fn a_keyed_store_seals_the_file_and_only_its_key_opens_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proton-session.json");
        let passphrases = KeyPassphrases::from_iter([("key-1", b"secret".to_vec())]);

        keyed(&path, [7; 32])
            .save(&stored(passphrases.clone()))
            .unwrap();
        let raw = std::fs::read(&path).unwrap();
        assert!(raw.starts_with(MAGIC));
        assert!(
            !raw.windows(7).any(|w| w == b"refresh"),
            "tokens left in the clear"
        );

        let loaded = keyed(&path, [7; 32]).load().unwrap().unwrap();
        assert!(loaded.key_passphrases == passphrases);

        let wrong = keyed(&path, [8; 32]).load().err().unwrap();
        assert!(format!("{wrong:#}").contains("does not open"), "{wrong:#}");
        let none = SessionStore::new(&path).load().err().unwrap();
        assert!(
            none.to_string().contains("WORKSPACE_SESSION_KEY_FILE"),
            "{none}"
        );

        let mut flipped = raw.clone();
        *flipped.last_mut().unwrap() ^= 1;
        std::fs::write(&path, &flipped).unwrap();
        assert!(keyed(&path, [7; 32]).load().is_err());
    }

    #[test]
    fn a_plain_file_is_sealed_the_first_time_a_key_reads_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proton-session.json");
        SessionStore::new(&path)
            .save(&stored(KeyPassphrases::new()))
            .unwrap();
        assert!(std::fs::read(&path).unwrap().starts_with(b"{"));

        let loaded = keyed(&path, [7; 32]).load().unwrap().unwrap();
        assert_eq!(loaded.refresh_token, "refresh");
        assert!(std::fs::read(&path).unwrap().starts_with(MAGIC));
        assert!(keyed(&path, [7; 32]).load().unwrap().is_some());
    }

    #[test]
    fn a_key_file_must_hold_32_bytes_of_base64() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("session.key");

        std::fs::write(&file, format!("{}\n", STANDARD.encode([3u8; 32]))).unwrap();
        assert_eq!(*read_key(&file).unwrap(), [3u8; 32]);

        for bad in ["", "not base64 at all", &STANDARD.encode([3u8; 16])] {
            std::fs::write(&file, bad).unwrap();
            let error = read_key(&file).err().unwrap();
            assert!(
                error.to_string().contains("openssl rand -base64 32"),
                "{error}"
            );
        }
        assert!(read_key(&dir.path().join("missing")).is_err());
    }
}
