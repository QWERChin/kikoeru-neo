//! The library database on PostgreSQL: its connection pools and every
//! query, compiled from the shared source in `crates/neo-db/queries`
//! (book/src/data-model.md, "PostgreSQL").

use std::time::Duration;

use neo_core::db::{OpenError, PG_MIGRATOR};
use sqlx::{
    ConnectOptions, Executor,
    postgres::{PgConnectOptions, PgPoolOptions},
};

pub type Database = sqlx::Postgres;
pub type Pool = sqlx::PgPool;
pub type Tx<'c> = sqlx::Transaction<'c, Database>;

#[path = "../../neo-db/queries/mod.rs"]
mod queries;
pub use queries::*;

const READ_CONNECTIONS: u32 = 4;
/// How long to keep trying a server that does not answer before startup
/// fails. The pool's default is 30 s.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The same split as on SQLite: one write connection, so this process's
/// write transactions never conflict with each other, and a pool of
/// readers.
#[derive(Debug, Clone)]
pub struct Db {
    pub read: Pool,
    pub write: Pool,
}

impl Db {
    /// Connects to the database at `url` and applies every pending
    /// migration.
    ///
    /// # Errors
    /// The URL is invalid, the server cannot be reached, or a migration
    /// fails.
    pub async fn open(url: &str, describe: &str) -> Result<Self, OpenError> {
        let connect_err = |error| OpenError::Connect {
            path: describe.to_owned(),
            error,
        };
        let options: PgConnectOptions = url.parse().map_err(connect_err)?;
        let options = options.disable_statement_logging();
        let write = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(CONNECT_TIMEOUT)
            // Another process, such as the `user` command, may write at
            // the same time. Serializable makes a transaction that read
            // before the other committed fail with 40001, which
            // `retry_busy` retries, as SQLite's BUSY_SNAPSHOT does.
            .after_connect(|conn, _| {
                Box::pin(async move {
                    conn.execute(
                        "SET SESSION CHARACTERISTICS AS TRANSACTION ISOLATION LEVEL SERIALIZABLE",
                    )
                    .await?;
                    Ok(())
                })
            })
            .connect_with(options.clone())
            .await
            .map_err(connect_err)?;
        PG_MIGRATOR.run(&write).await?;
        let read = PgPoolOptions::new()
            .max_connections(READ_CONNECTIONS)
            .acquire_timeout(CONNECT_TIMEOUT)
            // Readers that run several statements in one transaction rely
            // on one snapshot as SQLite gives them. READ ONLY matches SQLite's read-only reader pool.
            .after_connect(|conn, _| {
                Box::pin(async move {
                    conn.execute(
                        "SET SESSION CHARACTERISTICS AS TRANSACTION \
                         ISOLATION LEVEL REPEATABLE READ, READ ONLY",
                    )
                    .await?;
                    Ok(())
                })
            })
            .connect_with(options)
            .await
            .map_err(connect_err)?;
        Ok(Self { read, write })
    }

    pub async fn close(&self) {
        self.read.close().await;
        self.write.close().await;
    }
}

// This namespace/key pair is reserved for stored-root edits and reclamation.
// A backend can finish COMMIT after its application lost the secrets file lock.
async fn lock_stored_roots(tx: &mut Tx<'_>) -> sqlx::Result<()> {
    sqlx::query("SELECT pg_advisory_xact_lock(1802396973, 1)")
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn prepare_root_cleanup(tx: &mut Tx<'_>) -> sqlx::Result<()> {
    // SERIALIZABLE would keep the snapshot taken by the lock statement,
    // missing an edit that commits while that statement waits for the lock.
    sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
        .execute(&mut **tx)
        .await?;
    Ok(())
}
