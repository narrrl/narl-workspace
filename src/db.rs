//! What every writer to the database shares.

use sqlx::{Sqlite, SqlitePool, Transaction};

/// A transaction that takes the write lock up front. SQLite's default
/// `BEGIN` is deferred: a transaction that reads before it writes holds only
/// a read snapshot, and if another connection commits in between, the first
/// write fails at once with `SQLITE_BUSY` — `busy_timeout` cannot help,
/// because waiting would not make the snapshot current. Taking the lock at
/// `BEGIN` makes the reads and the writes one consistent step, and turns
/// contention into a wait instead of an error.
pub async fn begin_write(db: &SqlitePool) -> sqlx::Result<Transaction<'static, Sqlite>> {
    db.begin_with("BEGIN IMMEDIATE").await
}
