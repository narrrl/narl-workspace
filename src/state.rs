use std::sync::{atomic::AtomicI64, Arc};

use sqlx::SqlitePool;

use crate::{
    auth::Sessions,
    backup::Backup,
    config::Config,
    notify::Notifier,
    throttle::{LoginThrottle, LoginThrottles},
    vault::Vault,
};

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub config: Arc<Config>,
    pub login_throttle: Arc<LoginThrottles>,
    /// The vault password gets the same slowing-down as the login, kept apart
    /// so guessing at one does not slow the other.
    pub vault_throttle: Arc<LoginThrottle>,
    /// Sealed files. Always present; holds no key until it is unlocked.
    pub vault: Arc<Vault>,
    /// The Proton Drive mirror. Always present; dormant until it has a session.
    pub backup: Arc<Backup>,
    /// Notification mail. Always present; dormant until WORKSPACE_SMTP_URL is set.
    pub notify: Arc<Notifier>,
    /// Signed into every session cookie, which must carry the current value
    /// to be accepted; see [`crate::auth::Epoch`].
    pub epoch: Arc<AtomicI64>,
    /// Every signed-in browser; a cookie is good only while its row is here.
    pub sessions: Arc<Sessions>,
}
