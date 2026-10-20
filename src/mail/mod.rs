//! The outbox and the SMTP transport.
//!
//! Nothing in the application sends a mail directly. A mail is *enqueued* with
//! a `dedupe_key` that names the occurrence it belongs to, and the sweep in
//! [`crate::notify`] hands whatever is unsent to the relay. The split buys the
//! two properties a reminder needs: a restart cannot resend, because the row is
//! already there, and a relay that refuses cannot lose, because the row is
//! still there.
//!
//! Mail is dormant until `WORKSPACE_SMTP_URL` is set. Unconfigured, nothing is even
//! enqueued — an outbox nobody drains would only grow.

use anyhow::{Context, Result};
use lettre::{
    message::Mailbox, transport::smtp::authentication::Credentials, AsyncSmtpTransport,
    AsyncTransport, Message, Tokio1Executor,
};
use sqlx::{Row, SqlitePool};

use crate::config::{self, MailConfig};

/// Mails are plain text and short. Nothing here is worth an HTML part, and a
/// diary reminder that renders as a newsletter would be faintly absurd.
pub struct Mailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
    to: Mailbox,
}

/// How often a refused mail is retried before it is left alone: roughly a day
/// of doubling. A relay that has said no ten times is misconfigured, not busy,
/// and the row keeps its last error for whoever goes looking.
const MAX_ATTEMPTS: i64 = 10;
const FIRST_RETRY_SECS: i64 = 60;
const MAX_RETRY_SECS: i64 = 3600;

impl Mailer {
    /// `Ok(None)` when no SMTP URL is configured — that is a state, not a fault.
    pub fn new(config: &MailConfig) -> Result<Option<Self>> {
        let Some(url) = config.url.as_deref() else {
            return Ok(None);
        };

        // lettre reads the scheme, the port and any credentials out of the URL:
        // `smtps://` is implicit TLS, `smtp://host?tls=required` is STARTTLS,
        // and a bare `smtp://` is plain, for a local catcher and nothing else.
        let transport = AsyncSmtpTransport::<Tokio1Executor>::from_url(url)
            .context("WORKSPACE_SMTP_URL is not a usable SMTP URL")?;

        // A URL with credentials has already set them; this only covers the
        // relays that want them and a URL that keeps them out of the string.
        let transport = match (config::var("SMTP_USER"), config::var("SMTP_PASSWORD")) {
            (Some(user), Some(password)) => transport.credentials(Credentials::new(user, password)),
            _ => transport,
        }
        .build();

        Ok(Some(Self {
            transport,
            from: config
                .from
                .parse()
                .context("WORKSPACE_MAIL_FROM is not an email address")?,
            to: config
                .to
                .parse()
                .context("WORKSPACE_MAIL_TO is not an email address")?,
        }))
    }

    pub async fn send(&self, subject: &str, body: &str) -> Result<()> {
        let message = Message::builder()
            .from(self.from.clone())
            .to(self.to.clone())
            .subject(subject)
            .header(lettre::message::header::ContentType::TEXT_PLAIN)
            .body(body.to_string())
            .context("could not build the message")?;

        self.transport
            .send(message)
            .await
            .context("the SMTP relay refused the message")?;
        Ok(())
    }
}

/// Put a mail in the outbox. Returns whether this call is the one that created
/// it: a `dedupe_key` that is already there means the occurrence was handled,
/// whether or not the mail has left yet.
pub async fn enqueue(
    db: &SqlitePool,
    kind: &str,
    dedupe_key: &str,
    subject: &str,
    body: &str,
) -> Result<bool> {
    let result = sqlx::query(
        "INSERT OR IGNORE INTO notifications (kind, dedupe_key, subject, body, created_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(kind)
    .bind(dedupe_key)
    .bind(subject)
    .bind(body)
    .bind(crate::now())
    .execute(db)
    .await?;
    Ok(result.rows_affected() > 0)
}

/// Whether a mail that has already failed `attempts` times may be tried again.
/// 1m, 2m, 4m… up to an hour, measured from the last attempt rather than from
/// the count, because the count alone cannot say how long ago that was.
fn retry_due(attempts: i64, last_attempt_at: Option<i64>, now: i64) -> bool {
    let Some(last) = last_attempt_at else {
        return true;
    };
    if attempts >= MAX_ATTEMPTS {
        return false;
    }
    let wait = FIRST_RETRY_SECS
        .checked_shl(attempts.max(1) as u32 - 1)
        .unwrap_or(MAX_RETRY_SECS)
        .min(MAX_RETRY_SECS);
    now - last >= wait
}

/// Send everything the outbox is holding. Failures are recorded on the row and
/// retried later; one bad message does not block the ones behind it.
pub async fn send_pending(db: &SqlitePool, mailer: &Mailer) -> Result<usize> {
    // A mail the relay refused for good stays in the table for mail-status to
    // show, and out of this query: counted towards the limit, enough of them
    // would crowd out every mail queued after them, for ever.
    let rows = sqlx::query(
        "SELECT id, subject, body, attempts, last_attempt_at FROM notifications
         WHERE sent_at IS NULL AND attempts < ?1 ORDER BY created_at LIMIT 50",
    )
    .bind(MAX_ATTEMPTS)
    .fetch_all(db)
    .await?;

    let now = crate::now();
    let mut sent = 0;

    for row in rows {
        let id: i64 = row.get("id");
        let attempts: i64 = row.get("attempts");
        if !retry_due(attempts, row.get("last_attempt_at"), now) {
            continue;
        }

        let subject: String = row.get("subject");
        let body: String = row.get("body");
        match mailer.send(&subject, &body).await {
            Ok(()) => {
                sqlx::query(
                    "UPDATE notifications
                     SET sent_at = ?, attempts = attempts + 1, last_attempt_at = ?, last_error = NULL
                     WHERE id = ?",
                )
                .bind(crate::now())
                .bind(crate::now())
                .bind(id)
                .execute(db)
                .await?;
                sent += 1;
            }
            Err(e) => {
                let message = format!("{e:#}");
                tracing::warn!(id, error = %message, "could not send a notification");
                sqlx::query(
                    "UPDATE notifications
                     SET attempts = attempts + 1, last_attempt_at = ?, last_error = ?
                     WHERE id = ?",
                )
                .bind(crate::now())
                .bind(&message)
                .bind(id)
                .execute(db)
                .await?;
            }
        }
    }

    Ok(sent)
}

/// How long a mail that is done with — sent, or given up on — is kept.
const KEEP_DAYS: i64 = 90;

/// Forget mails that are done with and can never be asked about again. A key
/// that names a day, a week or an hour is never asked again once that has
/// passed; a card's key stays as long as its card does, because an overdue
/// card is asked about every day it stays overdue.
pub async fn prune(db: &SqlitePool, now: i64) -> Result<u64> {
    let removed = sqlx::query(
        "DELETE FROM notifications
         WHERE created_at < ?1
           AND (sent_at IS NOT NULL OR attempts >= ?2)
           AND (kind <> 'card_due' OR NOT EXISTS (
               SELECT 1 FROM cards c
               WHERE c.id = CAST(substr(dedupe_key, 10, instr(substr(dedupe_key, 10), ':') - 1) AS INTEGER)
           ))",
    )
    .bind(now - KEEP_DAYS * 86_400)
    .bind(MAX_ATTEMPTS)
    .execute(db)
    .await?
    .rows_affected();
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::{retry_due, MAX_ATTEMPTS};

    #[test]
    fn a_mail_that_has_never_been_tried_goes_out_at_once() {
        assert!(retry_due(0, None, 1_000));
    }

    #[test]
    fn a_refused_mail_waits_longer_every_time() {
        assert!(!retry_due(1, Some(1_000), 1_030));
        assert!(retry_due(1, Some(1_000), 1_060));
        // Four failures in, the wait is eight minutes.
        assert!(!retry_due(4, Some(1_000), 1_000 + 7 * 60));
        assert!(retry_due(4, Some(1_000), 1_000 + 8 * 60));
        // And it never grows past an hour.
        assert!(retry_due(9, Some(1_000), 1_000 + 3_600));
    }

    #[test]
    fn a_mail_the_relay_keeps_refusing_is_eventually_left_alone() {
        assert!(!retry_due(MAX_ATTEMPTS, Some(1_000), 1_000 + 86_400));
    }

    /// The property the whole outbox rests on: the tick asks about the same
    /// occurrence once a minute, all day, and only the first ask queues a mail.
    #[tokio::test]
    async fn an_occurrence_is_only_queued_once() {
        let db = sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("an in-memory database");
        sqlx::migrate!("./migrations")
            .run(&db)
            .await
            .expect("migrations apply");

        let queue = || super::enqueue(&db, "reminder", "reminder:2026-09-18", "subject", "body");
        assert!(queue().await.expect("the first ask queues"));
        assert!(!queue().await.expect("the second does not"));

        let queued: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications")
            .fetch_one(&db)
            .await
            .expect("counting works");
        assert_eq!(queued, 1);
    }

    async fn outbox() -> sqlx::SqlitePool {
        let db = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&db).await.unwrap();
        db
    }

    #[tokio::test]
    async fn mails_given_up_on_do_not_hold_up_the_rest() {
        let db = outbox().await;
        for n in 0..60 {
            super::enqueue(&db, "reminder", &format!("dead:{n}"), "s", "b")
                .await
                .unwrap();
        }
        sqlx::query("UPDATE notifications SET attempts = ?1, last_attempt_at = 0")
            .bind(MAX_ATTEMPTS)
            .execute(&db)
            .await
            .unwrap();
        super::enqueue(&db, "reminder", "reminder:2026-10-20", "s", "b")
            .await
            .unwrap();

        let next: Vec<String> = sqlx::query_scalar(
            "SELECT dedupe_key FROM notifications WHERE sent_at IS NULL AND attempts < ?1
             ORDER BY created_at LIMIT 50",
        )
        .bind(MAX_ATTEMPTS)
        .fetch_all(&db)
        .await
        .unwrap();
        assert_eq!(next, ["reminder:2026-10-20"]);
    }

    #[tokio::test]
    async fn old_mails_go_but_a_live_cards_key_stays() {
        let db = outbox().await;
        let space: i64 = sqlx::query_scalar(
            "INSERT INTO nodes (parent_id, kind, name, slug, has_board, created_at, updated_at)
             VALUES (NULL, 'space', 'job', 'job', 1, 0, 0) RETURNING id",
        )
        .fetch_one(&db)
        .await
        .unwrap();
        let list: i64 = sqlx::query_scalar(
            "INSERT INTO board_lists (space_id, name, position, created_at) VALUES (?1, 'l', 0, 0) RETURNING id",
        )
        .bind(space)
        .fetch_one(&db)
        .await
        .unwrap();
        let card: i64 = sqlx::query_scalar(
            "INSERT INTO cards (list_id, title, position, created_at, updated_at)
             VALUES (?1, 'c', 0, 0, 0) RETURNING id",
        )
        .bind(list)
        .fetch_one(&db)
        .await
        .unwrap();
        for key in [
            "reminder:2026-01-01".to_string(),
            format!("card_due:{card}:2026-01-01:overdue"),
            "card_due:999999:2026-01-01:overdue".to_string(),
        ] {
            super::enqueue(&db, "x", &key, "s", "b").await.unwrap();
        }
        sqlx::query("UPDATE notifications SET sent_at = 1, created_at = 1, kind = CASE WHEN dedupe_key LIKE 'card_due:%' THEN 'card_due' ELSE 'reminder' END")
            .execute(&db)
            .await
            .unwrap();

        assert_eq!(super::prune(&db, crate::now()).await.unwrap(), 2);
        let left: Vec<String> = sqlx::query_scalar("SELECT dedupe_key FROM notifications")
            .fetch_all(&db)
            .await
            .unwrap();
        assert_eq!(left, [format!("card_due:{card}:2026-01-01:overdue")]);
    }
}
