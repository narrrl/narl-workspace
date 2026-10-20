use std::{net::SocketAddr, path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use chrono::Weekday;
use chrono_tz::Tz;

/// The placeholders `.env.example` ships with, refused when left in place.
const EXAMPLE_SECRET: &str = "replace-with-32-random-characters";
const EXAMPLE_PASSWORD: &str = "change-me";

/// Runtime configuration. Everything comes from the environment (`.env` is
/// loaded on startup), because this is a deliberately single-user application.
#[derive(Clone)]
pub struct Config {
    pub username: String,
    pub password: String,
    pub secret: Vec<u8>,
    pub bind: SocketAddr,
    pub data_dir: PathBuf,
    pub session_days: i64,
    pub max_upload_bytes: usize,
    pub secure_cookie: bool,
    /// How long the vault stays open with nothing using it.
    pub vault_idle: std::time::Duration,
    /// How many days a deleted node waits in the trash before it is purged.
    pub trash_days: i64,
    pub backup: BackupConfig,
    pub mail: MailConfig,
    pub sftp: SftpConfig,
}

/// Written by hand so that a `{:?}` anywhere — a log line, a panic — never
/// prints the password, the cookie secret or the relay's credentials.
impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("secret", &"<redacted>")
            .field("bind", &self.bind)
            .field("data_dir", &self.data_dir)
            .field("session_days", &self.session_days)
            .field("max_upload_bytes", &self.max_upload_bytes)
            .field("secure_cookie", &self.secure_cookie)
            .field("vault_idle", &self.vault_idle)
            .field("trash_days", &self.trash_days)
            .field("backup", &self.backup)
            .field("mail", &self.mail)
            .field("sftp", &self.sftp)
            .finish()
    }
}

/// A whole number from the environment, `default` when it is not set, and an
/// error at startup — not an overflow later — when it is outside `allowed`.
fn number(key: &str, default: u64, allowed: std::ops::RangeInclusive<u64>) -> Result<u64> {
    let Some(raw) = var(key) else {
        return Ok(default);
    };
    let value: u64 = raw
        .trim()
        .parse()
        .with_context(|| format!("WORKSPACE_{key} must be a whole number, not {raw:?}"))?;
    if !allowed.contains(&value) {
        anyhow::bail!(
            "WORKSPACE_{key} must be between {} and {}, not {value}",
            allowed.start(),
            allowed.end()
        );
    }
    Ok(value)
}

/// A switch. Anything that is not clearly on or clearly off is an error:
/// `WORKSPACE_SECURE_COOKIE=yes` quietly meaning "no" is how a cookie ends up
/// going out over plain HTTP.
fn flag(key: &str) -> Result<bool> {
    let Some(raw) = var(key) else {
        return Ok(false);
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => anyhow::bail!(
            "WORKSPACE_{key} must be 1 or 0 (true/false, yes/no, on/off), not {raw:?}"
        ),
    }
}

/// The SFTP mount. Dormant until `WORKSPACE_SFTP_BIND` is set — same shape as
/// the mail relay, because a workspace with no key registered yet should not
/// refuse to start over a feature nobody has turned on.
#[derive(Debug, Clone)]
pub struct SftpConfig {
    pub bind: Option<SocketAddr>,
}

impl SftpConfig {
    fn from_env() -> Result<Self> {
        let bind = var("SFTP_BIND")
            .map(|v| v.parse())
            .transpose()
            .context("WORKSPACE_SFTP_BIND must look like 127.0.0.1:2222")?;
        Ok(Self { bind })
    }
}

/// Time of day in the configured timezone, as `HH:MM`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeOfDay {
    pub hour: u32,
    pub minute: u32,
}

/// Notification mail. Dormant until `WORKSPACE_SMTP_URL` is set — the same
/// dormant-until-configured shape the Proton mirror has, because a workspace
/// that refuses to start over an unconfigured mail relay would be a bad trade.
#[derive(Clone)]
pub struct MailConfig {
    /// `smtp://host:587` or `smtps://user:pass@host:465`. `None` turns every
    /// notification off, including the queue that would otherwise pile up.
    pub url: Option<String>,
    pub from: String,
    pub to: String,
    /// The timezone every "today" in the notifier is asked in.
    pub timezone: Tz,
    /// When to ask about the diary, if no document was written that day.
    pub reminder_at: Option<TimeOfDay>,
    /// When to mail about cards that are due or overdue.
    pub card_due_at: Option<TimeOfDay>,
    /// Weekday and time of the digest.
    pub digest_at: Option<(Weekday, TimeOfDay)>,
    /// The kinds that are allowed to be sent: `reminder`, `card_due`, `digest`,
    /// `login`. Listing them explicitly is how a single kind is switched off.
    pub kinds: Vec<String>,
}

impl std::fmt::Debug for MailConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MailConfig")
            // The URL carries the relay's user and password.
            .field("url", &self.url.as_ref().map(|_| "<redacted>"))
            .field("from", &self.from)
            .field("to", &self.to)
            .field("timezone", &self.timezone)
            .field("reminder_at", &self.reminder_at)
            .field("card_due_at", &self.card_due_at)
            .field("digest_at", &self.digest_at)
            .field("kinds", &self.kinds)
            .finish()
    }
}

impl MailConfig {
    /// Whether mail is configured at all.
    pub fn enabled(&self) -> bool {
        self.url.is_some()
    }

    pub fn sends(&self, kind: &str) -> bool {
        self.enabled() && self.kinds.iter().any(|k| k == kind)
    }
}

/// `20:00`, or `off` for nothing at all.
fn parse_time(key: &str, raw: &str) -> Result<Option<TimeOfDay>> {
    if raw.eq_ignore_ascii_case("off") || raw.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    let (hour, minute) = raw
        .split_once(':')
        .with_context(|| format!("{key} must look like 20:00, or be `off`"))?;
    let time = TimeOfDay {
        hour: hour
            .trim()
            .parse()
            .with_context(|| format!("{key}: {hour:?} is not an hour"))?,
        minute: minute
            .trim()
            .parse()
            .with_context(|| format!("{key}: {minute:?} is not a minute"))?,
    };
    if time.hour > 23 || time.minute > 59 {
        anyhow::bail!("{key} must be between 00:00 and 23:59");
    }
    Ok(Some(time))
}

fn parse_weekday(raw: &str) -> Option<Weekday> {
    match raw.to_ascii_lowercase().as_str() {
        "mon" | "monday" => Some(Weekday::Mon),
        "tue" | "tuesday" => Some(Weekday::Tue),
        "wed" | "wednesday" => Some(Weekday::Wed),
        "thu" | "thursday" => Some(Weekday::Thu),
        "fri" | "friday" => Some(Weekday::Fri),
        "sat" | "saturday" => Some(Weekday::Sat),
        "sun" | "sunday" => Some(Weekday::Sun),
        _ => None,
    }
}

/// `sun 18:00`, or `off`.
fn parse_digest(raw: &str) -> Result<Option<(Weekday, TimeOfDay)>> {
    let key = "WORKSPACE_DIGEST_AT";
    let Some((day, time)) = raw.split_once(char::is_whitespace) else {
        if raw.eq_ignore_ascii_case("off") || raw.eq_ignore_ascii_case("none") {
            return Ok(None);
        }
        anyhow::bail!("{key} must look like `sun 18:00`, or be `off`");
    };
    let day = parse_weekday(day.trim())
        .with_context(|| format!("{key}: {day:?} is not a weekday like `sun`"))?;
    Ok(parse_time(key, time.trim())?.map(|time| (day, time)))
}

/// The Proton Drive mirror. Dormant until a session has been stored by
/// `narl-workspace proton-login`; these values only shape how often it runs.
#[derive(Debug, Clone)]
pub struct BackupConfig {
    /// Name of the Drive device (its sync-root folder) this server owns.
    pub device_name: String,
    /// Backstop between runs. `None` disables the timer, leaving the mirror to
    /// whatever `POST /api/backup` asks for.
    pub interval: Option<Duration>,
    /// How long the workspace must be quiet after a change before it is mirrored,
    /// so a writing session is uploaded once rather than after every keystroke.
    pub debounce: Duration,
    /// Whether a file deleted here is trashed there. Off by default: a backup
    /// that forgets on command is one accident away from being no backup.
    pub prune: bool,
    /// A file holding the key `proton-session.json` is sealed under, kept off
    /// the data volume (a Docker secret, say) so a copy of the volume alone
    /// does not carry a usable session. `None` leaves the file as plain JSON.
    pub session_key_file: Option<PathBuf>,
}

/// Look a setting up in the environment.
///
/// `WORKSPACE_*` is the name to use. `DIARY_*` is still read behind it, because
/// this application was called narl-diary until it grew spaces and the running
/// deployment's `.env` still says so; a rename that logs the owner out of their
/// own server would be a poor trade for a tidier prefix.
pub fn var(key: &str) -> Option<String> {
    std::env::var(format!("WORKSPACE_{key}"))
        .or_else(|_| std::env::var(format!("DIARY_{key}")))
        .ok()
        .filter(|v| !v.trim().is_empty())
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let username = var("USER").context("WORKSPACE_USER must be set (see .env.example)")?;
        let password =
            var("PASSWORD").context("WORKSPACE_PASSWORD must be set (see .env.example)")?;
        let secret = var("SECRET")
            .context("WORKSPACE_SECRET must be set (see .env.example)")?
            .into_bytes();
        if secret.len() < 16 {
            anyhow::bail!("WORKSPACE_SECRET must be at least 16 characters");
        }
        // The examples are public: with the secret anyone can sign a session
        // cookie, with the password anyone can log in.
        if secret == EXAMPLE_SECRET.as_bytes() {
            anyhow::bail!(
                "WORKSPACE_SECRET is still the example from .env.example — \
                 set it to something random: openssl rand -base64 32"
            );
        }
        if secret.len() < 32 {
            tracing::warn!("WORKSPACE_SECRET is shorter than 32 characters; a longer one is safer");
        }
        if password == EXAMPLE_PASSWORD {
            anyhow::bail!("WORKSPACE_PASSWORD is still the example from .env.example — change it");
        }

        let bind = var("BIND")
            .unwrap_or_else(|| "127.0.0.1:4242".to_string())
            .parse()
            .context("WORKSPACE_BIND must look like 127.0.0.1:4242")?;

        let data_dir = PathBuf::from(var("DATA_DIR").unwrap_or_else(|| "./data".into()));

        // Ten years at most: a session is a cookie, and the expiry is
        // arithmetic on it that must not overflow.
        let session_days = number("SESSION_DAYS", 30, 1..=3650)? as i64;
        // Up to a terabyte, in bytes.
        let max_upload_bytes =
            usize::try_from(number("MAX_UPLOAD_MB", 1024, 1..=1024 * 1024)? * 1024 * 1024)
                .context("WORKSPACE_MAX_UPLOAD_MB is more than this machine can address")?;
        // A vault that locks the moment it opens could not be used; one that
        // stays open for more than a day is not much of a vault.
        let vault_idle = Duration::from_secs(number("VAULT_IDLE_MIN", 15, 1..=24 * 60)? * 60);
        // A day at least, so an undo is never raced by the sweep; ten years
        // at most, as for the sessions.
        let trash_days = number("TRASH_DAYS", 30, 1..=3650)? as i64;
        let secure_cookie = flag("SECURE_COOKIE")?;

        Ok(Self {
            username,
            password,
            secret,
            bind,
            data_dir,
            session_days,
            max_upload_bytes,
            secure_cookie,
            vault_idle,
            trash_days,
            backup: BackupConfig::from_env()?,
            mail: MailConfig::from_env()?,
            sftp: SftpConfig::from_env()?,
        })
    }

    pub fn uploads_dir(&self) -> PathBuf {
        self.data_dir.join("uploads")
    }

    /// Where the Proton session blob lives. Next to the database, because the
    /// two are equally sensitive and equally worth putting on the same volume.
    pub fn proton_session_path(&self) -> PathBuf {
        self.data_dir.join("proton-session.json")
    }

    /// The SFTP server's own identity, generated once on first boot. Losing it
    /// only costs a host-key warning on every client, never data.
    pub fn sftp_host_key_path(&self) -> PathBuf {
        self.data_dir.join("sftp_host_key")
    }
}

#[cfg(test)]
impl Config {
    /// A configuration with nothing switched on, for tests.
    pub fn for_tests(session_days: i64) -> Self {
        Self {
            username: "narl".into(),
            password: "hunter2".into(),
            secret: b"a-secret-of-at-least-16-bytes".to_vec(),
            bind: "127.0.0.1:0".parse().unwrap(),
            data_dir: ".".into(),
            session_days,
            max_upload_bytes: 1,
            secure_cookie: false,
            vault_idle: std::time::Duration::from_secs(60),
            trash_days: 30,
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
}

impl MailConfig {
    fn from_env() -> Result<Self> {
        let url = var("SMTP_URL");

        // Both addresses are the owner's: this mails the workspace to its writer.
        let to = var("MAIL_TO").unwrap_or_default();
        let from = var("MAIL_FROM").unwrap_or_else(|| to.clone());
        if url.is_some() && (from.is_empty() || to.is_empty()) {
            anyhow::bail!("WORKSPACE_SMTP_URL is set, so WORKSPACE_MAIL_TO must be set too");
        }

        let timezone = var("TIMEZONE")
            .unwrap_or_else(|| "Europe/Berlin".to_string())
            .parse::<Tz>()
            .map_err(|e| anyhow::anyhow!("WORKSPACE_TIMEZONE: {e}"))?;

        let reminder_at = parse_time(
            "WORKSPACE_REMINDER_AT",
            &var("REMINDER_AT").unwrap_or_else(|| "20:00".into()),
        )?;
        let card_due_at = parse_time(
            "WORKSPACE_CARD_DUE_AT",
            &var("CARD_DUE_AT").unwrap_or_else(|| "08:00".into()),
        )?;
        let digest_at = parse_digest(&var("DIGEST_AT").unwrap_or_else(|| "sun 18:00".into()))?;

        let kinds = var("MAIL_KINDS")
            .unwrap_or_else(|| "reminder,card_due,digest,login".into())
            .split(',')
            .map(|k| k.trim().to_ascii_lowercase())
            .filter(|k| !k.is_empty() && k != "off" && k != "none")
            .collect::<Vec<_>>();
        if let Some(unknown) = kinds
            .iter()
            .find(|k| !["reminder", "card_due", "digest", "login"].contains(&k.as_str()))
        {
            anyhow::bail!(
                "WORKSPACE_MAIL_KINDS: {unknown:?} is not one of reminder, card_due, digest, login"
            );
        }

        Ok(Self {
            url,
            from,
            to,
            timezone,
            reminder_at,
            card_due_at,
            digest_at,
            kinds,
        })
    }
}

impl BackupConfig {
    fn from_env() -> Result<Self> {
        // The default keeps the old name on purpose: it names a device that
        // already exists in Proton Drive, and a new name there would mean a
        // second device and a fresh upload of everything in it.
        let device_name = var("PROTON_DEVICE").unwrap_or_else(|| "narl-diary".to_string());

        // 0 turns the backstop off; a week is the longest it waits.
        let minutes = number("BACKUP_INTERVAL_MIN", 60, 0..=7 * 24 * 60)?;
        let interval = (minutes > 0).then(|| Duration::from_secs(minutes * 60));
        let debounce = Duration::from_secs(number("BACKUP_DEBOUNCE_SEC", 300, 0..=24 * 60 * 60)?);
        let prune = flag("BACKUP_PRUNE")?;

        Ok(Self {
            device_name,
            interval,
            debounce,
            prune,
            session_key_file: var("SESSION_KEY_FILE").map(PathBuf::from),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{flag, number, parse_digest, parse_time, TimeOfDay};
    use chrono::Weekday;

    /// Each case under a name of its own: tests run in parallel and share
    /// one environment.
    fn with(key: &str, value: &str) -> String {
        std::env::set_var(format!("WORKSPACE_{key}"), value);
        key.to_string()
    }

    #[test]
    fn numbers_out_of_bounds_stop_the_start() {
        assert_eq!(number("TEST_UNSET_NUMBER", 7, 1..=10).unwrap(), 7);
        assert_eq!(
            number(&with("TEST_NUMBER_OK", " 9 "), 7, 1..=10).unwrap(),
            9
        );
        assert!(number(&with("TEST_NUMBER_ZERO", "0"), 7, 1..=10).is_err());
        assert!(number(&with("TEST_NUMBER_HUGE", "99999999999999999999"), 7, 1..=10).is_err());
        assert!(number(&with("TEST_NUMBER_NEGATIVE", "-1"), 7, 1..=10).is_err());
    }

    #[test]
    fn a_switch_is_on_off_or_an_error() {
        assert!(!flag("TEST_UNSET_FLAG").unwrap());
        assert!(flag(&with("TEST_FLAG_YES", "yes")).unwrap());
        assert!(flag(&with("TEST_FLAG_ONE", "1")).unwrap());
        assert!(!flag(&with("TEST_FLAG_OFF", "off")).unwrap());
        assert!(flag(&with("TEST_FLAG_TYPO", "ture")).is_err());
    }

    #[test]
    fn a_time_of_day_is_read_as_written() {
        assert_eq!(
            parse_time("T", "20:00").unwrap(),
            Some(TimeOfDay {
                hour: 20,
                minute: 0
            })
        );
        assert_eq!(
            parse_time("T", "07:45").unwrap(),
            Some(TimeOfDay {
                hour: 7,
                minute: 45
            })
        );
        assert_eq!(parse_time("T", "off").unwrap(), None);
    }

    #[test]
    fn a_time_that_does_not_exist_is_refused_rather_than_rounded() {
        for raw in ["24:00", "20:60", "8", "eight", "20:00:00"] {
            assert!(parse_time("T", raw).is_err(), "{raw:?}");
        }
    }

    #[test]
    fn a_digest_is_a_weekday_and_a_time() {
        assert_eq!(
            parse_digest("sun 18:00").unwrap(),
            Some((
                Weekday::Sun,
                TimeOfDay {
                    hour: 18,
                    minute: 0
                }
            ))
        );
        assert_eq!(
            parse_digest("Monday 09:30").unwrap(),
            Some((
                Weekday::Mon,
                TimeOfDay {
                    hour: 9,
                    minute: 30
                }
            ))
        );
        assert_eq!(parse_digest("off").unwrap(), None);
        assert!(parse_digest("someday 18:00").is_err());
        assert!(parse_digest("sun").is_err());
    }
}
