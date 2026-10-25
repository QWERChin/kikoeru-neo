//! Copies a whole library database between SQLite and PostgreSQL, in
//! either direction (book/src/data-model.md, "在两种数据库之间复制").
//!
//! The source is read in one snapshot and never written. The target is a
//! new SQLite file or an empty PostgreSQL database. It is migrated to the
//! same version as the source and filled in one transaction, so a failed
//! copy leaves no rows behind.

use std::path::{Path, PathBuf};

use futures_util::TryStreamExt;
use sqlx::{
    ConnectOptions, PgConnection, PgPool, Postgres, QueryBuilder, Row, Sqlite, SqlitePool,
    Transaction,
    migrate::MigrateError,
    postgres::{PgConnectOptions, PgPoolOptions},
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};

use super::{MIGRATOR, PG_MIGRATOR};

/// How long to keep trying a PostgreSQL server that does not answer. The
/// pool's default of 30 s is long for a command someone is waiting on.
const PG_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Rows per INSERT. 500 rows of the widest table bind 7000 values, under
/// SQLite's limit of 32766 and PostgreSQL's of 65535.
const BATCH: usize = 500;

/// One end of a copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    Sqlite(PathBuf),
    /// A PostgreSQL URL. A password can also come from `PGPASSWORD` or
    /// `~/.pgpass`, which keeps it off the command line.
    Postgres(String),
}

impl Endpoint {
    /// A `postgres://` or `postgresql://` URL, otherwise a SQLite file.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        if text.starts_with("postgres://") || text.starts_with("postgresql://") {
            Self::Postgres(text.to_owned())
        } else {
            Self::Sqlite(PathBuf::from(text))
        }
    }

    /// For messages. A PostgreSQL URL is shown without its password.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Sqlite(path) => path.display().to_string(),
            Self::Postgres(url) => redact_password(url),
        }
    }
}

/// Hides a password given as `user:secret@` or as `?password=secret`,
/// the two places sqlx reads one from.
fn redact_password(url: &str) -> String {
    let (base, query) = match url.split_once('?') {
        Some((base, query)) => (base, Some(query)),
        None => (url, None),
    };
    let mut out = match base.find("://").map(|i| i + 3) {
        Some(start) => {
            let rest = &base[start..];
            // The last `@` rather than the first `/`: an unescaped `/` in
            // the password must not let it through.
            match rest.rfind('@') {
                Some(at) => match rest[..at].find(':') {
                    Some(colon) => {
                        format!("{}{}:***{}", &base[..start], &rest[..colon], &rest[at..])
                    }
                    None => base.to_owned(),
                },
                None => base.to_owned(),
            }
        }
        None => base.to_owned(),
    };
    if let Some(query) = query {
        let pairs: Vec<&str> = query
            .split('&')
            .map(|pair| match pair.split_once('=') {
                // sqlx decodes the key, so `%70assword` is a password too.
                Some((key, _)) if percent_decode(key).eq_ignore_ascii_case("password") => {
                    "password=***"
                }
                _ => pair,
            })
            .collect();
        out.push('?');
        out.push_str(&pairs.join("&"));
    }
    out
}

/// Decodes `%XX` escapes. A malformed escape is kept as it is.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[derive(Debug, thiserror::Error)]
pub enum CopyError {
    #[error("cannot open {what}: {source}")]
    Open { what: String, source: sqlx::Error },
    #[error("{0} does not exist")]
    SourceMissing(String),
    #[error("{0} already exists; the copy only writes a new file")]
    TargetExists(String),
    #[error("cannot create {what}: {source}")]
    Create {
        what: String,
        source: std::io::Error,
    },
    #[error("{0} is not empty; the copy only writes into an empty database")]
    TargetNotEmpty(String),
    #[error("another copy is writing into {0}")]
    TargetBusy(String),
    #[error(
        "{what} is at schema version {found:?} but this program is at {expected}; \
         start the matching kikoeru-neo once to migrate it first"
    )]
    Version {
        what: String,
        found: Option<i64>,
        expected: i64,
    },
    #[error("{table} has {source_rows} rows in the source but {target_rows} in the copy")]
    Count {
        table: &'static str,
        source_rows: u64,
        target_rows: i64,
    },
    #[error("database migration failed: {0}")]
    Migrate(#[from] MigrateError),
    #[error(transparent)]
    Sql(#[from] sqlx::Error),
}

/// Rows copied per table, in copy order.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub tables: Vec<(&'static str, u64)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Int,
    Bool,
    Text,
    Real,
    Blob,
}

/// A table and its columns in schema order. `copy_test` checks this list
/// against the migrated SQLite schema, so a new column cannot be missed.
struct Table {
    name: &'static str,
    key: &'static [&'static str],
    columns: &'static [(&'static str, Class)],
    /// Its `id` is an identity on PostgreSQL, whose sequence must move past
    /// the copied IDs.
    identity: bool,
    /// A column that refers to a row of the same table that may come later.
    /// It is written as NULL first and filled in after all rows exist.
    self_ref: Option<&'static str>,
}

use Class::{Blob, Bool, Int, Real, Text};

/// Parents before children, so every foreign key finds its row.
#[rustfmt::skip]
const TABLES: &[Table] = &[
    table("roots", &["id"], &[
        ("id", Text), ("display_name", Text), ("last_scan_at", Int), ("status", Text),
        ("last_scan_end_at", Int), ("source_version", Int),
    ]),
    table("circles", &["id"], &[("id", Int), ("name", Text)]),
    table("vas", &["id"], &[("id", Text), ("name", Text)]),
    table("tags", &["id"], &[("id", Int), ("name", Text)]),
    table("tag_aliases", &["alias"], &[("alias", Text), ("tag_id", Int)]),
    table("tag_merges", &["from_id"], &[("from_id", Int), ("into_id", Int), ("merged_at", Int)]),
    identity("works", &[
        ("id", Int), ("code", Text), ("title", Text), ("circle_id", Int), ("release_date", Text),
        ("nsfw", Bool), ("dl_count", Int), ("price", Int), ("rate_average", Real),
        ("rate_count", Int), ("metadata_src", Text), ("cover_src", Text), ("cover_version", Int),
        ("extra", Text), ("missing_at", Int), ("created_at", Int), ("updated_at", Int),
    ]),
    identity("work_locations", &[
        ("id", Int), ("work_id", Int), ("root_id", Text), ("rel_path", Blob),
        ("display_path", Text), ("listing_hash", Text), ("listed_at", Int), ("missing_at", Int),
    ]),
    table("work_vas", &["work_id", "va_id"], &[("work_id", Int), ("va_id", Text)]),
    table("work_tags", &["work_id", "tag_id"], &[("work_id", Int), ("tag_id", Int)]),
    Table {
        self_ref: Some("duplicate_of"),
        ..identity("tracks", &[
            ("id", Int), ("work_id", Int), ("location_id", Int), ("rel_path", Blob),
            ("display_path", Text), ("sort_key", Text), ("kind", Text), ("size", Int),
            ("mtime", Int), ("duration_ms", Int), ("duration_src", Text), ("probe", Text),
            ("duplicate_of", Int), ("missing_at", Int),
        ])
    },
    table("track_analysis", &["track_id"], &[
        ("track_id", Int), ("size", Int), ("mtime", Int), ("integrated_lufs", Real),
        ("true_peak_dbtp", Real), ("analyzed_at", Int),
    ]),
    identity("users", &[
        ("id", Int), ("name", Text), ("password_hash", Text), ("is_admin", Bool),
        ("token_version", Int), ("created_at", Int),
    ]),
    table("user_settings", &["user_id"], &[
        ("user_id", Int), ("normalize_loudness", Bool), ("target_lufs", Real),
        ("updated_at", Int), ("accent", Text), ("seek_back_seconds", Int),
        ("seek_forward_seconds", Int),
    ]),
    table("favorites", &["user_id", "work_id"], &[
        ("user_id", Int), ("work_id", Int), ("created_at", Int),
    ]),
    table("play_progress", &["user_id", "work_id"], &[
        ("user_id", Int), ("work_id", Int), ("track_id", Int), ("position_ms", Int),
        ("group_path", Text), ("finished_at", Int), ("replaying", Bool), ("updated_at", Int),
    ]),
    table("progress_report_cursors", &["user_id", "work_id", "session_id"], &[
        ("user_id", Int), ("work_id", Int), ("session_id", Text), ("last_sequence", Int),
    ]),
    table("legacy_reviews", &["user_id", "work_id"], &[
        ("user_id", Int), ("work_id", Int), ("rating", Int), ("review_text", Text),
        ("progress", Text),
    ]),
    table("reviews", &["user_id", "work_id"], &[
        ("user_id", Int), ("work_id", Int), ("rating", Int), ("text", Text), ("updated_at", Int),
    ]),
    table("import_pending", &["user_id", "work_id"], &[
        ("user_id", Int), ("work_id", Int), ("source", Text), ("durations", Text),
        ("state", Text),
    ]),
    table("legacy_user_state", &["user_id", "code"], &[
        ("user_id", Int), ("code", Text), ("favorite_at", Int), ("has_review", Bool),
        ("rating", Int), ("review_text", Text), ("review_progress", Text), ("progress_at", Int),
        ("position_ms", Int), ("finished_at", Int), ("pending_source", Text),
        ("pending_durations", Text), ("pending_state", Text), ("review_at", Int),
    ]),
    identity("tasks", &[
        ("id", Int), ("kind", Text), ("dedupe_key", Text), ("state", Text), ("payload", Text),
        ("result", Text), ("progress", Real), ("message", Text), ("created_at", Int),
        ("updated_at", Int), ("queue_rank", Int),
    ]),
    table("root_settings", &["root_id"], &[
        ("root_id", Text), ("kind", Text), ("settings", Text), ("created_at", Int),
        ("updated_at", Int),
    ]),
    table("server_settings", &["id"], &[("id", Int), ("metadata_proxy", Text), ("updated_at", Int)]),
];

const fn table(
    name: &'static str,
    key: &'static [&'static str],
    columns: &'static [(&'static str, Class)],
) -> Table {
    Table {
        name,
        key,
        columns,
        identity: false,
        self_ref: None,
    }
}

const fn identity(name: &'static str, columns: &'static [(&'static str, Class)]) -> Table {
    Table {
        identity: true,
        ..table(name, &["id"], columns)
    }
}

/// One stored value. The class travels with NULL too, because PostgreSQL
/// rejects a NULL bound as the wrong type.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    Int(Option<i64>),
    Bool(Option<bool>),
    Text(Option<String>),
    Real(Option<f64>),
    Blob(Option<Vec<u8>>),
}

enum Tx {
    Sqlite(Transaction<'static, Sqlite>),
    Postgres(Transaction<'static, Postgres>),
}

/// Runs `$body` with `$t` bound to the transaction of either backend. Each
/// arm is type-checked on its own, so the body can use backend types.
macro_rules! with_tx {
    ($tx:expr, $t:ident => $body:expr) => {
        match $tx {
            Tx::Sqlite($t) => $body,
            Tx::Postgres($t) => $body,
        }
    };
}

/// Copies every table from `from` into `to` and checks the row counts.
///
/// # Errors
/// The source or target cannot be opened, they are at different schema
/// versions, the target is not new and empty, or any database error. On
/// error the target has no rows, and a new SQLite file is removed.
pub async fn copy(from: &Endpoint, to: &Endpoint) -> Result<Report, CopyError> {
    let expected = latest_version();
    let (mut source, source_pools) = open_source(from).await?;
    let found = with_tx!(&mut source, t => {
        // A file that is not a library database has no such table.
        match sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success")
            .fetch_one(&mut **t)
            .await
        {
            Err(sqlx::Error::Database(_)) => Ok(None),
            other => other,
        }
    })?;
    if found != Some(expected) {
        return Err(CopyError::Version {
            what: from.describe(),
            found,
            expected,
        });
    }
    let target_pools = open_target(to).await?;
    let result = async {
        let mut target = begin_target(&target_pools, to).await?;
        let report = copy_tables(&mut source, &mut target).await?;
        with_tx!(target, t => t.commit().await)?;
        Ok(report)
    }
    .await;
    with_tx!(source, t => t.rollback().await).ok();
    source_pools.close().await;
    target_pools.close().await;
    if result.is_err()
        && let Endpoint::Sqlite(path) = to
    {
        remove_sqlite(path);
    }
    result
}

fn latest_version() -> i64 {
    MIGRATOR.iter().map(|m| m.version).max().unwrap_or(0)
}

enum Pools {
    Sqlite(SqlitePool),
    Postgres(PgPool),
}

impl Pools {
    async fn close(&self) {
        match self {
            Self::Sqlite(p) => p.close().await,
            Self::Postgres(p) => p.close().await,
        }
    }
}

/// One connection each, so that every statement runs in the snapshot the
/// transaction took.
async fn open_source(from: &Endpoint) -> Result<(Tx, Pools), CopyError> {
    let open_err = |source| CopyError::Open {
        what: from.describe(),
        source,
    };
    match from {
        Endpoint::Sqlite(path) => {
            if !path.is_file() {
                return Err(CopyError::SourceMissing(from.describe()));
            }
            // Read-only, so a mistaken path can never be changed. WAL
            // readers keep one snapshot for the whole transaction while the
            // server goes on writing.
            // Statement logging is off because reading a whole table is
            // expected to pass sqlx's one-second slow-query warning.
            let options = SqliteConnectOptions::new()
                .filename(path)
                .read_only(true)
                .busy_timeout(std::time::Duration::from_secs(30))
                .disable_statement_logging();
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(options)
                .await
                .map_err(open_err)?;
            let tx = pool.begin().await?;
            Ok((Tx::Sqlite(tx), Pools::Sqlite(pool)))
        }
        Endpoint::Postgres(url) => {
            let options = url
                .parse::<PgConnectOptions>()
                .map_err(open_err)?
                .disable_statement_logging();
            let pool = PgPoolOptions::new()
                .max_connections(1)
                .acquire_timeout(PG_CONNECT_TIMEOUT)
                .connect_with(options)
                .await
                .map_err(open_err)?;
            let tx = pool
                .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
                .await?;
            Ok((Tx::Postgres(tx), Pools::Postgres(pool)))
        }
    }
}

/// Creates or checks the target and migrates it. Its rows are written later
/// in one transaction.
async fn open_target(to: &Endpoint) -> Result<Pools, CopyError> {
    let open_err = |source| CopyError::Open {
        what: to.describe(),
        source,
    };
    match to {
        Endpoint::Sqlite(path) => {
            create_private(path).map_err(|e| match e.kind() {
                std::io::ErrorKind::AlreadyExists => CopyError::TargetExists(to.describe()),
                _ => CopyError::Create {
                    what: to.describe(),
                    source: e,
                },
            })?;
            // From here on the file is this copy's own, so a failure may
            // remove it. A single file without -wal, so the copy can be
            // moved or backed up as it is.
            let options = SqliteConnectOptions::new()
                .filename(path)
                .foreign_keys(true)
                .journal_mode(SqliteJournalMode::Delete)
                .disable_statement_logging();
            let opened = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(options)
                .await;
            let pool = match opened {
                Ok(pool) => pool,
                Err(e) => {
                    remove_sqlite(path);
                    return Err(open_err(e));
                }
            };
            if let Err(e) = MIGRATOR.run(&pool).await {
                pool.close().await;
                remove_sqlite(path);
                return Err(e.into());
            }
            Ok(Pools::Sqlite(pool))
        }
        Endpoint::Postgres(url) => {
            let options = url
                .parse::<PgConnectOptions>()
                .map_err(open_err)?
                .disable_statement_logging();
            let pool = PgPoolOptions::new()
                .max_connections(1)
                .acquire_timeout(PG_CONNECT_TIMEOUT)
                .connect_with(options)
                .await
                .map_err(open_err)?;
            // Checked before migrating too, so a database that holds
            // something else is never given this schema.
            let result = async {
                refuse_unless_empty(&mut *pool.acquire().await?, to).await?;
                PG_MIGRATOR.run(&pool).await?;
                Ok(())
            }
            .await;
            if let Err(e) = result {
                pool.close().await;
                return Err(e);
            }
            Ok(Pools::Postgres(pool))
        }
    }
}

/// A database that has only this schema with no rows, or nothing at all.
/// Anything else would be mixed with the copy or migrated by mistake.
async fn refuse_unless_empty(conn: &mut PgConnection, to: &Endpoint) -> Result<(), CopyError> {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables
         WHERE table_schema = current_schema()",
    )
    .fetch_all(&mut *conn)
    .await?;
    if tables.is_empty() {
        return Ok(());
    }
    let ours = |t: &String| t == "_sqlx_migrations" || TABLES.iter().any(|s| s.name == t);
    if !tables.iter().all(ours) {
        return Err(CopyError::TargetNotEmpty(to.describe()));
    }
    for t in TABLES {
        if !tables.iter().any(|name| name == t.name) {
            continue;
        }
        let any: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT EXISTS (SELECT 1 FROM {})",
            t.name
        )))
        .fetch_one(&mut *conn)
        .await?;
        if any {
            return Err(CopyError::TargetNotEmpty(to.describe()));
        }
    }
    Ok(())
}

/// Held by the copy's transaction on a PostgreSQL target. Without it two
/// copies into one empty database could both pass the emptiness check and
/// commit a mixture of their sources.
pub const COPY_LOCK: i64 = 0x6b6e_656f_636f_7079;

async fn begin_target(pools: &Pools, to: &Endpoint) -> Result<Tx, CopyError> {
    Ok(match pools {
        Pools::Sqlite(p) => Tx::Sqlite(p.begin().await?),
        Pools::Postgres(p) => {
            let mut tx = p.begin().await?;
            let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock($1)")
                .bind(COPY_LOCK)
                .fetch_one(&mut *tx)
                .await?;
            if !locked {
                return Err(CopyError::TargetBusy(to.describe()));
            }
            // Again under the lock: a copy that held it may have committed
            // since the first check.
            refuse_unless_empty(&mut tx, to).await?;
            Tx::Postgres(tx)
        }
    })
}

async fn copy_tables(source: &mut Tx, target: &mut Tx) -> Result<Report, CopyError> {
    let mut report = Report::default();
    for table in TABLES {
        let rows = copy_table(source, target, table).await?;
        if let Some(column) = table.self_ref {
            fill_self_ref(source, target, table, column).await?;
        }
        let in_target: i64 = with_tx!(&mut *target, t => {
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT COUNT(*) FROM {}", table.name)))
                .fetch_one(&mut **t)
                .await
        })?;
        if u64::try_from(in_target).ok() != Some(rows) {
            return Err(CopyError::Count {
                table: table.name,
                source_rows: rows,
                target_rows: in_target,
            });
        }
        report.tables.push((table.name, rows));
    }
    for table in TABLES.iter().filter(|t| t.identity) {
        let used = highest_id(source, table).await?;
        keep_ids_used(target, table, used).await?;
    }
    Ok(report)
}

/// The highest ID the source ever gave out in `table`, deleted rows
/// included. IDs appear in URLs and tokens, so the copy must not hand out
/// an ID again that once meant another row.
async fn highest_id(source: &mut Tx, table: &Table) -> Result<i64, CopyError> {
    let used: Option<i64> = match source {
        Tx::Sqlite(s) => {
            // AUTOINCREMENT keeps the high-water mark in sqlite_sequence.
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT MAX(COALESCE((SELECT seq FROM sqlite_sequence WHERE name = '{0}'), 0),
                            COALESCE((SELECT MAX(id) FROM {0}), 0))",
                table.name
            )))
            .fetch_one(&mut **s)
            .await?
        }
        Tx::Postgres(s) => {
            let sequence: String = sqlx::query_scalar("SELECT pg_get_serial_sequence($1, 'id')")
                .bind(table.name)
                .fetch_one(&mut **s)
                .await?;
            // last_value is the next ID, not one given out, until is_called.
            // A copy leaves it that way, so it must be read here rather
            // than through pg_sequence_last_value, which is NULL then.
            let (last, called): (i64, bool) = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT last_value, is_called FROM {sequence}"
            )))
            .fetch_one(&mut **s)
            .await?;
            let rows: Option<i64> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT MAX(id) FROM {}",
                table.name
            )))
            .fetch_one(&mut **s)
            .await?;
            Some(rows.unwrap_or(0).max(if called { last } else { last - 1 }))
        }
    };
    Ok(used.unwrap_or(0))
}

/// Makes the target give out IDs after `used`, whatever rows it has.
async fn keep_ids_used(target: &mut Tx, table: &Table, used: i64) -> Result<(), CopyError> {
    match target {
        Tx::Sqlite(t) => {
            // Inserting with explicit IDs already raised it to MAX(id).
            let name = table.name;
            sqlx::query("UPDATE sqlite_sequence SET seq = MAX(seq, $2) WHERE name = $1")
                .bind(name)
                .bind(used)
                .execute(&mut **t)
                .await?;
            sqlx::query(
                "INSERT INTO sqlite_sequence (name, seq)
                 SELECT $1, $2 WHERE NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = $1)",
            )
            .bind(name)
            .bind(used)
            .execute(&mut **t)
            .await?;
        }
        Tx::Postgres(t) => {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "SELECT setval(pg_get_serial_sequence('{}', 'id'), $1, false)",
                table.name
            )))
            .bind(used + 1)
            .execute(&mut **t)
            .await?;
        }
    }
    Ok(())
}

fn select_sql(table: &Table, columns: &[&str]) -> String {
    format!(
        "SELECT {} FROM {} ORDER BY {}",
        columns.join(", "),
        table.name,
        table.key.join(", ")
    )
}

/// Streams the table in key order and writes it in batches.
async fn copy_table(source: &mut Tx, target: &mut Tx, table: &Table) -> Result<u64, CopyError> {
    let names: Vec<&str> = table.columns.iter().map(|(n, _)| *n).collect();
    let sql = select_sql(table, &names);
    let mut batch: Vec<Vec<Value>> = Vec::with_capacity(BATCH);
    let mut rows = 0;
    with_tx!(source, s => {
        let mut stream = sqlx::query(sqlx::AssertSqlSafe(sql)).fetch(&mut **s);
        while let Some(row) = stream.try_next().await? {
            let mut values = Vec::with_capacity(table.columns.len());
            for (i, (name, class)) in table.columns.iter().enumerate() {
                // Filled in by fill_self_ref once every row exists.
                let value = if table.self_ref == Some(*name) {
                    Value::Int(None)
                } else {
                    match class {
                        Int => Value::Int(row.try_get(i)?),
                        Bool => Value::Bool(row.try_get(i)?),
                        Text => Value::Text(row.try_get(i)?),
                        Real => Value::Real(row.try_get(i)?),
                        Blob => Value::Blob(row.try_get(i)?),
                    }
                };
                values.push(value);
            }
            batch.push(values);
            rows += 1;
            if batch.len() == BATCH {
                insert(target, table, &batch).await?;
                batch.clear();
            }
        }
    });
    if !batch.is_empty() {
        insert(target, table, &batch).await?;
    }
    Ok(rows)
}

async fn insert(target: &mut Tx, table: &Table, rows: &[Vec<Value>]) -> Result<(), CopyError> {
    let names: Vec<&str> = table.columns.iter().map(|(n, _)| *n).collect();
    let head = format!("INSERT INTO {} ({}) ", table.name, names.join(", "));
    with_tx!(target, t => {
        let mut sql = QueryBuilder::new(head);
        sql.push_values(rows, |mut b, row| {
            for value in row {
                match value {
                    Value::Int(v) => b.push_bind(*v),
                    Value::Bool(v) => b.push_bind(*v),
                    Value::Text(v) => b.push_bind(v.clone()),
                    Value::Real(v) => b.push_bind(*v),
                    Value::Blob(v) => b.push_bind(v.clone()),
                };
            }
        });
        sql.build().execute(&mut **t).await?;
    });
    Ok(())
}

/// Writes the self-referencing column now that every row of the table is
/// there. Only rows where it is set are touched.
async fn fill_self_ref(
    source: &mut Tx,
    target: &mut Tx,
    table: &Table,
    column: &str,
) -> Result<(), CopyError> {
    let sql = format!(
        "SELECT id, {column} FROM {} WHERE {column} IS NOT NULL ORDER BY id",
        table.name
    );
    let pairs: Vec<(i64, i64)> = with_tx!(source, s => {
        sqlx::query_as(sqlx::AssertSqlSafe(sql)).fetch_all(&mut **s).await
    })?;
    let update = format!("UPDATE {} SET {column} = $1 WHERE id = $2", table.name);
    with_tx!(target, t => {
        for (id, refers) in pairs {
            sqlx::query(sqlx::AssertSqlSafe(update.clone()))
                .bind(refers)
                .bind(id)
                .execute(&mut **t)
                .await?;
        }
    });
    Ok(())
}

/// Creates the empty target file, failing if anything is already there.
/// The library holds password hashes, so only the owner may read it, like
/// the data directory's own database. SQLite gives its journal the same
/// mode.
fn create_private(path: &Path) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path).map(drop)
}

/// Removes a SQLite target that the copy created and could not finish,
/// with any side files.
fn remove_sqlite(path: &Path) {
    for suffix in ["", "-journal", "-wal", "-shm"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(name));
    }
}

#[cfg(test)]
mod copy_test {
    use super::*;

    /// Without `user=` or PGUSER, sqlx must ask for the OS user, as libpq
    /// does, or peer authentication on a local socket always fails.
    #[test]
    fn a_url_without_a_user_connects_as_the_os_user() {
        if std::env::var_os("PGUSER").is_some() {
            return;
        }
        let options: sqlx::postgres::PgConnectOptions =
            "postgres:///kikoeru?host=/run/postgresql".parse().unwrap();
        assert_ne!(options.get_username(), "anonymous");
        assert!(!options.get_username().is_empty());
    }

    /// TABLES must name every table of the migrated schema, with every
    /// column in order and the class its declared type implies.
    #[tokio::test]
    async fn table_list_matches_the_schema() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(SqliteConnectOptions::new().in_memory(true))
            .await
            .unwrap();
        MIGRATOR.run(&pool).await.unwrap();
        let mut names: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type = 'table'
               AND name NOT LIKE 'sqlite%' AND name <> '_sqlx_migrations' ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        let mut listed: Vec<String> = TABLES.iter().map(|t| t.name.to_owned()).collect();
        names.sort();
        listed.sort();
        assert_eq!(listed, names);
        for table in TABLES {
            let columns: Vec<(String, String, i64)> =
                sqlx::query_as("SELECT name, type, pk FROM pragma_table_info($1) ORDER BY cid")
                    .bind(table.name)
                    .fetch_all(&pool)
                    .await
                    .unwrap();
            let want: Vec<(&str, Class)> = columns
                .iter()
                .map(|(name, ty, _)| {
                    let listed = table.columns.iter().find(|(n, _)| n == name);
                    let class = match (ty.as_str(), listed.map(|(_, c)| *c)) {
                        ("INTEGER", Some(Bool)) => Bool,
                        ("INTEGER", _) => Int,
                        ("TEXT", _) => Text,
                        ("REAL", _) => Real,
                        ("BLOB", _) => Blob,
                        (other, _) => panic!("{}.{name} has type {other}", table.name),
                    };
                    (name.as_str(), class)
                })
                .collect();
            assert_eq!(table.columns, want.as_slice(), "columns of {}", table.name);
            let mut key: Vec<(i64, &str)> = columns
                .iter()
                .filter(|(_, _, pk)| *pk > 0)
                .map(|(n, _, pk)| (*pk, n.as_str()))
                .collect();
            key.sort_unstable();
            let key: Vec<&str> = key.into_iter().map(|(_, n)| n).collect();
            assert_eq!(table.key, key.as_slice(), "primary key of {}", table.name);
        }
        pool.close().await;
    }
}
