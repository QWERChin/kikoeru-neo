//! The library database on SQLite: its connection pools and every query,
//! compiled from the shared source in `crates/neo-db/queries`.

use std::{path::Path, time::Duration};

use neo_core::db::{MIGRATOR, OpenError};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

pub type Database = sqlx::Sqlite;
pub type Pool = sqlx::SqlitePool;
pub type Tx<'c> = sqlx::Transaction<'c, Database>;

#[path = "../../neo-db/queries/mod.rs"]
mod queries;
pub use queries::*;

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const READ_CONNECTIONS: u32 = 4;

/// The two pools every caller goes through. All writes share one connection,
/// so write transactions are serialized inside the process and never wait on
/// each other for SQLite's lock.
#[derive(Debug, Clone)]
pub struct Db {
    pub read: Pool,
    pub write: Pool,
}

impl Db {
    /// Opens (creating if needed) the database at `path` and applies every
    /// pending migration. Any migration error is returned, never logged and
    /// skipped, so a caller cannot run on a half-upgraded schema.
    ///
    /// # Errors
    /// The file cannot be opened or created, or a migration fails.
    pub async fn open(path: &Path) -> Result<Self, OpenError> {
        let connect_err = |error| OpenError::Connect {
            path: path.display().to_string(),
            error,
        };
        let base = SqliteConnectOptions::new()
            .filename(path)
            .foreign_keys(true)
            .busy_timeout(BUSY_TIMEOUT);
        let write = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                base.clone()
                    .create_if_missing(true)
                    .journal_mode(SqliteJournalMode::Wal)
                    .synchronous(SqliteSynchronous::Normal),
            )
            .await
            .map_err(connect_err)?;
        MIGRATOR.run(&write).await?;
        // The read pool opens after migrating, because a read-only connection
        // cannot create the file or switch it to WAL.
        let read = SqlitePoolOptions::new()
            .max_connections(READ_CONNECTIONS)
            .connect_with(base.read_only(true))
            .await
            .map_err(connect_err)?;
        Ok(Self { read, write })
    }

    pub async fn close(&self) {
        self.read.close().await;
        self.write.close().await;
    }
}
