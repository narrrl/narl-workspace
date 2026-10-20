//! The vault: files encrypted at rest under a key that only a password unlocks.
//!
//! Everything else in the workspace is protected by the login and by where the
//! data directory lives. A file in the vault is protected even from someone who
//! has both — a copy of the volume, the Proton account the mirror writes to, a
//! stolen session — because its bytes on disk are ciphertext and the key is
//! held in memory only while the vault is open.
//!
//! Keys:
//!
//! - a random 256-bit **master key**, made once at setup and never stored in
//!   the clear: the `vault` row holds it sealed under a key derived from the
//!   password with Argon2id. Changing the password re-seals the master key and
//!   touches no file;
//! - a random 256-bit **file key** per file, sealed under the master key in
//!   the file's own header, so no two files share a key and nonces never need
//!   counting across files.
//!
//! A sealed file is
//!
//! ```text
//! "NARLVLT1"                    8 bytes, format and version
//! wrap nonce                   12 bytes
//! file key, sealed             48 bytes: AES-256-GCM under the master key,
//!                                        the media id as associated data
//! stream nonce                  7 bytes
//! records                      AES-256-GCM STREAM (BE32): 64 KiB of plaintext
//!                              plus a 16-byte tag each, the last one shorter
//!                              and marked last, so a truncated file is caught
//! ```
//!
//! Binding the media id means a sealed file renamed to another id refuses to
//! open rather than showing up as somebody else. The password is never stored
//! and cannot be recovered: forgetting it loses every file in the vault, which
//! is the point.

use std::{
    io::Read,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use aes_gcm::{
    aead::{
        stream::{EncryptorBE32, NewStream, StreamBE32, StreamPrimitive},
        Aead, KeyInit, Payload,
    },
    Aes256Gcm, Nonce,
};
use anyhow::{anyhow, Context, Result};
use rand::RngCore;
use sqlx::{Row, SqlitePool};
use zeroize::Zeroizing;

pub const MAGIC: &[u8; 8] = b"NARLVLT1";
/// Plaintext per record. Large enough that the tags cost nothing to speak of,
/// small enough to hold one in memory per open download.
pub const CHUNK: usize = 64 * 1024;
const TAG: usize = 16;
/// A record on disk: a chunk of plaintext and its tag.
pub const RECORD: usize = CHUNK + TAG;
const WRAP_NONCE: usize = 12;
const STREAM_NONCE: usize = 7;
pub const HEADER: usize = MAGIC.len() + WRAP_NONCE + 32 + TAG + STREAM_NONCE;

/// What the master key is sealed with, besides the password-derived key.
const MASTER_AAD: &[u8] = b"narl-vault-master-key";

/// The shortest password the vault takes. It guards everything in it, and
/// unlike the login it cannot be reset by editing a config file.
pub const MIN_PASSWORD: usize = 10;

/// Argon2id costs for new vaults: 64 MiB, three passes, one lane. A fraction
/// of a second on a small server, which is the budget for an unlock. Kept per
/// vault, so raising them later does not lock anybody out.
const M_COST: u32 = 64 * 1024;
const T_COST: u32 = 3;
const P_COST: u32 = 1;

pub type Key = Zeroizing<[u8; 32]>;

fn random<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    rand::thread_rng().fill_bytes(&mut out);
    out
}

/// The key the password stands for, at the costs the vault was made with.
fn derive(password: &str, salt: &[u8], m: u32, t: u32, p: u32) -> Result<Key> {
    let params =
        argon2::Params::new(m, t, p, Some(32)).map_err(|e| anyhow!("argon2 params: {e}"))?;
    let argon = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; 32]);
    argon
        .hash_password_into(password.as_bytes(), salt, key.as_mut())
        .map_err(|e| anyhow!("argon2: {e}"))?;
    Ok(key)
}

fn seal_master(kek: &Key, master: &Key) -> Result<([u8; WRAP_NONCE], Vec<u8>)> {
    let nonce = random::<WRAP_NONCE>();
    let sealed = Aes256Gcm::new(kek.as_ref().into())
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: master.as_ref(),
                aad: MASTER_AAD,
            },
        )
        .map_err(|_| anyhow!("could not seal the master key"))?;
    Ok((nonce, sealed))
}

/// `None` is a wrong password: the tag does not verify.
fn open_master(kek: &Key, nonce: &[u8], sealed: &[u8]) -> Option<Key> {
    let plain = Zeroizing::new(
        Aes256Gcm::new(kek.as_ref().into())
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: sealed,
                    aad: MASTER_AAD,
                },
            )
            .ok()?,
    );
    let mut key = Zeroizing::new([0u8; 32]);
    key.copy_from_slice(plain.get(..32)?);
    Some(key)
}

/// Encrypts one file as it streams past: the header first, then a record for
/// every [`CHUNK`] that has arrived, and the short last one at [`finish`](Self::finish).
pub struct Sealer {
    stream: EncryptorBE32<Aes256Gcm>,
    pending: Vec<u8>,
}

impl Sealer {
    /// A sealer for `media_id`, and the header that has to be written first.
    pub fn new(master: &Key, media_id: &str) -> Result<(Self, Vec<u8>)> {
        let file_key: Key = Zeroizing::new(random::<32>());
        let wrap_nonce = random::<WRAP_NONCE>();
        let stream_nonce = random::<STREAM_NONCE>();

        let sealed_key = Aes256Gcm::new(master.as_ref().into())
            .encrypt(
                Nonce::from_slice(&wrap_nonce),
                Payload {
                    msg: file_key.as_ref(),
                    aad: &file_aad(media_id),
                },
            )
            .map_err(|_| anyhow!("could not seal a file key"))?;

        let mut header = Vec::with_capacity(HEADER);
        header.extend_from_slice(MAGIC);
        header.extend_from_slice(&wrap_nonce);
        header.extend_from_slice(&sealed_key);
        header.extend_from_slice(&stream_nonce);

        let stream = EncryptorBE32::from_aead(
            Aes256Gcm::new(file_key.as_ref().into()),
            stream_nonce.as_slice().into(),
        );
        Ok((
            Self {
                stream,
                pending: Vec::with_capacity(CHUNK),
            },
            header,
        ))
    }

    /// Take more plaintext; returns whatever whole records it completes. A
    /// record is only sealed once more data follows it, because only the last
    /// one may be marked last and the last is not known until `finish`.
    pub fn push(&mut self, mut data: &[u8]) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        while !data.is_empty() {
            if self.pending.len() == CHUNK {
                let record = self
                    .stream
                    .encrypt_next(self.pending.as_slice())
                    .map_err(|_| anyhow!("could not seal a record"))?;
                out.extend_from_slice(&record);
                self.pending.clear();
            }
            let take = (CHUNK - self.pending.len()).min(data.len());
            self.pending.extend_from_slice(&data[..take]);
            data = &data[take..];
        }
        Ok(out)
    }

    /// The last record. Always present, empty for an empty file, so even a
    /// file cut off exactly at a record boundary is caught.
    pub fn finish(self) -> Result<Vec<u8>> {
        self.stream
            .encrypt_last(self.pending.as_slice())
            .map_err(|_| anyhow!("could not seal the last record"))
    }
}

fn file_aad(media_id: &str) -> Vec<u8> {
    let mut aad = MAGIC.to_vec();
    aad.extend_from_slice(media_id.as_bytes());
    aad
}

/// Decrypts one sealed file, a record at a time, from the first or from any
/// record a range asks to start at.
pub struct Opener {
    stream: StreamBE32<Aes256Gcm>,
    /// The record `open` decrypts next. STREAM binds each record to its
    /// position, so one moved within the file refuses to open.
    position: u32,
    /// Records in the file, the last of them included.
    total: u64,
    /// The size of the last record, which is the only short one.
    last: usize,
}

impl Opener {
    /// Check the header and unseal the file key. `len` is the size of the
    /// whole sealed file, which is what tells the reader where the last record
    /// starts.
    pub fn new(master: &Key, media_id: &str, header: &[u8], len: u64) -> Result<Self> {
        if header.len() != HEADER || &header[..MAGIC.len()] != MAGIC {
            return Err(anyhow!("not a sealed file"));
        }
        let body = len
            .checked_sub(HEADER as u64)
            .filter(|body| *body >= TAG as u64)
            .ok_or_else(|| anyhow!("sealed file is truncated"))?;
        let remaining = body.div_ceil(RECORD as u64);
        let last = (body - (remaining - 1) * RECORD as u64) as usize;
        if last < TAG {
            return Err(anyhow!("sealed file is truncated"));
        }

        let (wrap_nonce, rest) = header[MAGIC.len()..].split_at(WRAP_NONCE);
        let (sealed_key, stream_nonce) = rest.split_at(32 + TAG);
        let file_key = Zeroizing::new(
            Aes256Gcm::new(master.as_ref().into())
                .decrypt(
                    Nonce::from_slice(wrap_nonce),
                    Payload {
                        msg: sealed_key,
                        aad: &file_aad(media_id),
                    },
                )
                .map_err(|_| {
                    anyhow!("this file was not sealed under this vault, or not as {media_id}")
                })?,
        );
        let stream = StreamBE32::from_aead(
            Aes256Gcm::new(file_key.as_slice().into()),
            stream_nonce.into(),
        );
        Ok(Self {
            stream,
            position: 0,
            total: remaining,
            last,
        })
    }

    /// How many bytes the next record takes on disk; 0 once all are read.
    pub fn next_len(&self) -> usize {
        match self.total.saturating_sub(self.position as u64) {
            0 => 0,
            1 => self.last,
            _ => RECORD,
        }
    }

    /// Start at record `index` instead of the first — at byte
    /// `HEADER + index * RECORD` of the file — for a read that wants only the
    /// end of it.
    pub fn skip_to(&mut self, index: u64) -> Result<()> {
        if index >= self.total {
            return Err(anyhow!("past the end of the sealed file"));
        }
        self.position = u32::try_from(index).map_err(|_| anyhow!("sealed file is too large"))?;
        Ok(())
    }

    /// Decrypt the record `next_len` asked for.
    pub fn open(&mut self, record: &[u8]) -> Result<Vec<u8>> {
        let left = self.total.saturating_sub(self.position as u64);
        if left == 0 {
            return Err(anyhow!("sealed file already read"));
        }
        // Only the record that really is the last may be opened as last, so
        // a file cut short at a record boundary still refuses to open.
        let plain = self
            .stream
            .decrypt(self.position, left == 1, record)
            .map_err(|_| anyhow!("sealed file is damaged or was tampered with"))?;
        self.position += 1;
        Ok(plain)
    }
}

/// A sealed file read as its plaintext, for code that wants `std::io::Read`:
/// the export's zip writer, re-sealing.
pub struct Reader<R> {
    inner: R,
    opener: Opener,
    buffer: Vec<u8>,
    at: usize,
}

impl Reader<std::fs::File> {
    pub fn open(master: &Key, media_id: &str, path: &std::path::Path) -> Result<Self> {
        let mut file = std::fs::File::open(path)
            .with_context(|| format!("could not read {}", path.display()))?;
        let len = file.metadata()?.len();
        let mut header = [0u8; HEADER];
        file.read_exact(&mut header)
            .context("sealed file is truncated")?;
        let opener = Opener::new(master, media_id, &header, len)?;
        Ok(Self {
            inner: file,
            opener,
            buffer: Vec::new(),
            at: 0,
        })
    }
}

impl<R: Read> Read for Reader<R> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        while self.at == self.buffer.len() {
            let len = self.opener.next_len();
            if len == 0 {
                return Ok(0);
            }
            let mut record = vec![0u8; len];
            self.inner.read_exact(&mut record)?;
            self.buffer = self.opener.open(&record).map_err(std::io::Error::other)?;
            self.at = 0;
        }
        let n = out.len().min(self.buffer.len() - self.at);
        out[..n].copy_from_slice(&self.buffer[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

/// Seal `source` into `target` under `media_id`, on the calling thread.
/// Returns the plaintext size.
pub fn seal_file(
    master: &Key,
    media_id: &str,
    source: &mut impl Read,
    target: &std::path::Path,
) -> Result<u64> {
    use std::io::Write;
    let mut out = std::io::BufWriter::new(std::fs::File::create(target)?);
    let (mut sealer, header) = Sealer::new(master, media_id)?;
    out.write_all(&header)?;
    let mut buffer = vec![0u8; CHUNK];
    let mut size = 0u64;
    loop {
        let n = source.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        out.write_all(&sealer.push(&buffer[..n])?)?;
    }
    out.write_all(&sealer.finish()?)?;
    out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
    Ok(size)
}

/// A vault password on its way to Argon2, wiped from memory once it is done.
pub type Password = Zeroizing<String>;

fn humantime(d: Duration) -> String {
    match d.as_secs() {
        s if s % 60 == 0 => format!("{}m", s / 60),
        s => format!("{s}s"),
    }
}

/// The vault's state: whether it exists, and its key while it is open.
pub struct Vault {
    db: SqlitePool,
    idle: Duration,
    open: Mutex<Option<Unlocked>>,
}

struct Unlocked {
    key: Arc<Key>,
    touched: Instant,
}

#[derive(serde::Serialize)]
pub struct Status {
    /// Whether a vault password has been set at all.
    pub configured: bool,
    pub unlocked: bool,
    /// Seconds until it locks itself if nothing uses it.
    pub locks_in: Option<u64>,
    /// How many files are sealed: what an export needs the vault open for.
    pub sealed: i64,
}

/// What a password check came to.
pub enum Unlock {
    Opened,
    WrongPassword,
    NotConfigured,
}

impl Vault {
    pub fn new(db: SqlitePool, idle: Duration) -> Arc<Self> {
        Arc::new(Self {
            db,
            idle,
            open: Mutex::new(None),
        })
    }

    fn slot(&self) -> std::sync::MutexGuard<'_, Option<Unlocked>> {
        self.open.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The master key, if the vault is open — and keep it open a while longer.
    /// A vault left alone past the idle time closes on the next look.
    pub fn key(&self) -> Option<Arc<Key>> {
        let mut slot = self.slot();
        match slot.as_mut() {
            Some(open) if open.touched.elapsed() < self.idle => {
                open.touched = Instant::now();
                Some(open.key.clone())
            }
            Some(_) => {
                *slot = None;
                None
            }
            None => None,
        }
    }

    /// Whether the vault is open, without counting as a use that keeps it so:
    /// for a check that should not hold it open by being asked often.
    pub fn is_open(&self) -> bool {
        self.slot()
            .as_ref()
            .is_some_and(|open| open.touched.elapsed() < self.idle)
    }

    pub fn lock(&self) {
        *self.slot() = None;
    }

    /// Open with a key as it is, skipping Argon2, for tests of what an open
    /// vault allows.
    #[cfg(test)]
    pub fn open_with(&self, key: Key) {
        *self.slot() = Some(Unlocked {
            key: Arc::new(key),
            touched: Instant::now(),
        });
    }

    /// Drop the key once it has gone unused past the idle time. Without this
    /// a vault nobody looks at would only notice it should be shut on the
    /// next look, with the key sitting in memory all the while.
    fn expire(&self) {
        let mut slot = self.slot();
        if slot
            .as_ref()
            .is_some_and(|open| open.touched.elapsed() >= self.idle)
        {
            *slot = None;
            tracing::info!("vault locked after {} idle", humantime(self.idle));
        }
    }

    /// Check for an idle vault every few seconds, for as long as the vault
    /// itself is around.
    pub fn spawn_idle_lock(self: &Arc<Self>) {
        let vault = Arc::downgrade(self);
        let every = (self.idle / 4).clamp(Duration::from_secs(1), Duration::from_secs(30));
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(every);
            loop {
                ticker.tick().await;
                let Some(vault) = vault.upgrade() else { return };
                vault.expire();
            }
        });
    }

    pub async fn status(&self) -> Result<Status> {
        let configured = self.row().await?.is_some();
        let sealed: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM media WHERE encrypted")
            .fetch_one(&self.db)
            .await?;
        self.expire();
        let slot = self.slot();
        let remaining = slot
            .as_ref()
            .map(|open| self.idle.saturating_sub(open.touched.elapsed()))
            .filter(|left| !left.is_zero());
        Ok(Status {
            configured,
            unlocked: remaining.is_some(),
            locks_in: remaining.map(|left| left.as_secs()),
            sealed,
        })
    }

    async fn row(&self) -> Result<Option<sqlx::sqlite::SqliteRow>> {
        Ok(sqlx::query(
            "SELECT salt, m_cost, t_cost, p_cost, nonce, sealed FROM vault WHERE id = 1",
        )
        .fetch_optional(&self.db)
        .await?)
    }

    /// Make the vault, and leave it open. Refuses when there already is one:
    /// a second setup would orphan every file sealed under the first.
    pub async fn setup(&self, password: Password) -> Result<bool> {
        if self.row().await?.is_some() {
            return Ok(false);
        }
        let salt = random::<16>();
        let master: Key = Zeroizing::new(random::<32>());
        let (nonce, sealed, master) = tokio::task::spawn_blocking(move || -> Result<_> {
            let kek = derive(&password, &salt, M_COST, T_COST, P_COST)?;
            let (nonce, sealed) = seal_master(&kek, &master)?;
            Ok((nonce, sealed, master))
        })
        .await??;

        let inserted = sqlx::query(
            "INSERT OR IGNORE INTO vault (id, salt, m_cost, t_cost, p_cost, nonce, sealed, created_at)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .bind(salt.as_slice())
        .bind(M_COST as i64)
        .bind(T_COST as i64)
        .bind(P_COST as i64)
        .bind(nonce.as_slice())
        .bind(sealed)
        .bind(crate::now())
        .execute(&self.db)
        .await?
        .rows_affected();
        if inserted == 0 {
            return Ok(false);
        }

        *self.slot() = Some(Unlocked {
            key: Arc::new(master),
            touched: Instant::now(),
        });
        Ok(true)
    }

    /// The master key the password stands for, if it is the right one.
    async fn check(&self, password: Password) -> Result<Option<Option<Key>>> {
        let Some(row) = self.row().await? else {
            return Ok(None);
        };
        let salt: Vec<u8> = row.get("salt");
        let (m, t, p): (i64, i64, i64) = (row.get("m_cost"), row.get("t_cost"), row.get("p_cost"));
        let nonce: Vec<u8> = row.get("nonce");
        let sealed: Vec<u8> = row.get("sealed");
        let key = tokio::task::spawn_blocking(move || -> Result<Option<Key>> {
            let kek = derive(&password, &salt, m as u32, t as u32, p as u32)?;
            Ok(open_master(&kek, &nonce, &sealed))
        })
        .await??;
        Ok(Some(key))
    }

    pub async fn unlock(&self, password: Password) -> Result<Unlock> {
        Ok(match self.check(password).await? {
            None => Unlock::NotConfigured,
            Some(None) => Unlock::WrongPassword,
            Some(Some(key)) => {
                *self.slot() = Some(Unlocked {
                    key: Arc::new(key),
                    touched: Instant::now(),
                });
                Unlock::Opened
            }
        })
    }

    /// Re-seal the master key under a new password. No file changes: they are
    /// sealed under the master key, which stays what it was.
    pub async fn change_password(&self, current: Password, new: Password) -> Result<Unlock> {
        let master = match self.check(current).await? {
            None => return Ok(Unlock::NotConfigured),
            Some(None) => return Ok(Unlock::WrongPassword),
            Some(Some(key)) => key,
        };
        let salt = random::<16>();
        let (nonce, sealed, master) = tokio::task::spawn_blocking(move || -> Result<_> {
            let kek = derive(&new, &salt, M_COST, T_COST, P_COST)?;
            let (nonce, sealed) = seal_master(&kek, &master)?;
            Ok((nonce, sealed, master))
        })
        .await??;
        sqlx::query(
            "UPDATE vault SET salt = ?1, m_cost = ?2, t_cost = ?3, p_cost = ?4, nonce = ?5, sealed = ?6
             WHERE id = 1",
        )
        .bind(salt.as_slice())
        .bind(M_COST as i64)
        .bind(T_COST as i64)
        .bind(P_COST as i64)
        .bind(nonce.as_slice())
        .bind(sealed)
        .execute(&self.db)
        .await?;
        *self.slot() = Some(Unlocked {
            key: Arc::new(master),
            touched: Instant::now(),
        });
        Ok(Unlock::Opened)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use zeroize::Zeroizing;

    use super::{derive, open_master, seal_master, Opener, Reader, Sealer, CHUNK, HEADER, TAG};

    fn key(byte: u8) -> super::Key {
        Zeroizing::new([byte; 32])
    }

    fn seal(master: &super::Key, id: &str, plain: &[u8], step: usize) -> Vec<u8> {
        let (mut sealer, mut out) = Sealer::new(master, id).expect("sealer");
        for piece in plain.chunks(step.max(1)) {
            out.extend(sealer.push(piece).expect("push"));
        }
        out.extend(sealer.finish().expect("finish"));
        out
    }

    fn open(master: &super::Key, id: &str, sealed: &[u8]) -> anyhow::Result<Vec<u8>> {
        let mut opener = Opener::new(master, id, &sealed[..HEADER], sealed.len() as u64)?;
        let mut at = HEADER;
        let mut out = Vec::new();
        loop {
            let len = opener.next_len();
            if len == 0 {
                break;
            }
            out.extend(opener.open(&sealed[at..at + len])?);
            at += len;
        }
        Ok(out)
    }

    /// Idle means the key is gone from memory, not only that the next look
    /// will refuse it.
    #[tokio::test]
    async fn an_idle_vault_lets_go_of_its_key() {
        let db = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let vault = super::Vault::new(db, std::time::Duration::from_millis(20));
        *vault.slot() = Some(super::Unlocked {
            key: std::sync::Arc::new(key(1)),
            touched: std::time::Instant::now(),
        });
        vault.expire();
        assert!(vault.slot().is_some(), "still within the idle time");
        std::thread::sleep(std::time::Duration::from_millis(30));
        vault.expire();
        assert!(vault.slot().is_none());
    }

    /// Reading from record `k` on gives exactly the plaintext from
    /// `k * CHUNK` on, and the last record still has to be the last.
    #[test]
    fn a_sealed_file_opens_from_any_record() {
        let master = key(3);
        let plain: Vec<u8> = (0..3 * CHUNK + 17).map(|i| (i % 249) as u8).collect();
        let sealed = seal(&master, "id", &plain, CHUNK);
        for first in 0..4u64 {
            let mut opener =
                Opener::new(&master, "id", &sealed[..HEADER], sealed.len() as u64).unwrap();
            opener.skip_to(first).unwrap();
            let mut at = HEADER + first as usize * super::RECORD;
            let mut out = Vec::new();
            while opener.next_len() > 0 {
                let len = opener.next_len();
                out.extend(opener.open(&sealed[at..at + len]).unwrap());
                at += len;
            }
            assert_eq!(out, plain[first as usize * CHUNK..], "from record {first}");
        }
        let mut opener =
            Opener::new(&master, "id", &sealed[..HEADER], sealed.len() as u64).unwrap();
        assert!(opener.skip_to(4).is_err());
        // A middle record presented where the last one should be does not open.
        opener.skip_to(3).unwrap();
        let middle = &sealed[HEADER + super::RECORD..HEADER + 2 * super::RECORD];
        assert!(opener.open(&middle[..opener.next_len()]).is_err());
    }

    #[test]
    fn files_of_every_awkward_size_round_trip() {
        let master = key(7);
        for size in [0, 1, CHUNK - 1, CHUNK, CHUNK + 1, 3 * CHUNK, 3 * CHUNK + 17] {
            let plain: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
            for step in [1000, CHUNK, 7 * CHUNK] {
                let sealed = seal(&master, "id", &plain, step);
                let records = size.div_ceil(CHUNK).max(1);
                assert_eq!(sealed.len(), HEADER + size + records * TAG, "size {size}");
                assert_eq!(
                    open(&master, "id", &sealed).expect("opens"),
                    plain,
                    "size {size} step {step}"
                );
            }
        }
    }

    #[test]
    fn the_std_reader_sees_the_plaintext() {
        let master = key(1);
        let plain: Vec<u8> = (0..(2 * CHUNK + 5)).map(|i| i as u8).collect();
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("x");
        std::fs::write(&path, seal(&master, "x", &plain, CHUNK)).expect("write");
        let mut out = Vec::new();
        Reader::open(&master, "x", &path)
            .expect("reader")
            .read_to_end(&mut out)
            .expect("read");
        assert_eq!(out, plain);
    }

    #[test]
    fn the_wrong_key_another_id_or_a_cut_are_all_refused() {
        let master = key(2);
        let plain = vec![9u8; CHUNK * 2];
        let sealed = seal(&master, "a", &plain, CHUNK);

        assert!(open(&key(3), "a", &sealed).is_err(), "another vault");
        assert!(
            open(&master, "b", &sealed).is_err(),
            "renamed to another id"
        );

        // Cut at a record boundary: what is left still parses as records, and
        // only the missing "last" mark gives it away.
        let cut = &sealed[..HEADER + CHUNK + TAG];
        assert!(
            open(&master, "a", cut).is_err(),
            "truncated at a record boundary"
        );

        let mut flipped = sealed.clone();
        flipped[HEADER + 10] ^= 1;
        assert!(open(&master, "a", &flipped).is_err(), "a flipped bit");
    }

    #[test]
    fn only_the_right_password_opens_the_master_key() {
        // Small costs: this is about the sealing, not about Argon2's speed.
        let kek = derive("correct horse", b"saltsaltsaltsalt", 256, 1, 1).expect("derive");
        let master = key(5);
        let (nonce, sealed) = seal_master(&kek, &master).expect("seal");
        assert_eq!(
            open_master(&kek, &nonce, &sealed).expect("opens").as_ref(),
            master.as_ref()
        );
        let wrong = derive("correct horse!", b"saltsaltsaltsalt", 256, 1, 1).expect("derive");
        assert!(open_master(&wrong, &nonce, &sealed).is_none());
    }
}
