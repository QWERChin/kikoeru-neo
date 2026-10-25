//! What the database layer shares between its backends: the row and
//! argument types of every query, the migrations, retrying a busy
//! transaction and copying a whole library. The queries themselves and
//! the connection pools live in the neo-db crates (book/src/data-model.md,
//! "存储层的代码结构").

pub mod analysis;
#[cfg(feature = "postgres")]
pub mod copy;
pub mod detail;
pub mod edit;
pub mod import;
pub mod legacy_state;
pub mod library;
pub mod media;
pub mod metadata;
pub mod progress;
pub mod reviews;
pub mod roots;
pub mod scan;
pub mod settings;
pub mod tags;
pub mod tasks;
pub mod users;
pub mod works;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sqlx::migrate::{MigrateError, Migrator};

pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations/sqlite");

/// The same schema for PostgreSQL (book/src/data-model.md, "PostgreSQL").
/// It starts at version 9 with the SQLite schema of that version, and every
/// later migration keeps the same number on both backends.
#[cfg(feature = "postgres")]
pub static PG_MIGRATOR: Migrator = sqlx::migrate!("./migrations/postgres");

/// Retries after SQLITE_BUSY, on top of the first attempt.
const BUSY_RETRIES: u32 = 3;
/// The wait before the first retry. Each later one waits this much longer.
const BUSY_BACKOFF: Duration = Duration::from_millis(50);

#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    // The cause is part of the message and deliberately not a `source`:
    // sqlx's errors already repeat their own cause, and `{:#}` printed the
    // driver's message three times.
    #[error("cannot open database {path}: {error}")]
    Connect { path: String, error: sqlx::Error },
    #[error("database migration failed: {0}")]
    Migrate(#[from] MigrateError),
}

/// Current time as Unix milliseconds, the unit of every business timestamp.
pub fn now_ms() -> i64 {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
}

/// Runs `transaction` again, at most [`BUSY_RETRIES`] more times, when
/// the database reports another writer in the way ([`is_busy`],
/// book/src/data-model.md "SQLite 设置").
///
/// Each call of `transaction` must be one whole transaction, either a single
/// statement or a `begin` through `commit`, with no effect outside the
/// database. A busy error rolls the whole attempt back, so running it again
/// is safe. Busy can still happen despite `busy_timeout` and the single
/// write connection: another process, such as the `user` command, may hold
/// the lock past the timeout, and a transaction that read before another
/// process wrote fails at once with `SQLITE_BUSY_SNAPSHOT`.
///
/// # Errors
/// The last attempt's error, or any error that is not busy.
pub async fn retry_busy<T, F, Fut>(mut transaction: F) -> Result<T, sqlx::Error>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, sqlx::Error>>,
{
    let mut retries = 0;
    loop {
        match transaction().await {
            Err(error) if is_busy(&error) && retries < BUSY_RETRIES => {
                retries += 1;
                tokio::time::sleep(BUSY_BACKOFF * retries).await;
            }
            result => return result,
        }
    }
}

/// `SQLITE_BUSY` or one of its extended codes, such as `SQLITE_BUSY_SNAPSHOT`,
/// or PostgreSQL's serialization failure or deadlock. Each means another
/// writer got in the way and the whole transaction may run again.
#[must_use]
pub fn is_busy(error: &sqlx::Error) -> bool {
    const SQLITE_BUSY: i32 = 5;
    let sqlx::Error::Database(error) = error else {
        return false;
    };
    let Some(code) = error.code() else {
        return false;
    };
    // SQLSTATE codes are five characters, and 40001 would also parse as a
    // SQLite code number.
    if matches!(&*code, "40001" | "40P01") {
        return true;
    }
    code.parse::<i32>()
        .is_ok_and(|code| code & 0xff == SQLITE_BUSY)
}
