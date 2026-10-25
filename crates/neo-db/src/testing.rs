//! A library database for tests, on the backend `KIKOERU_TEST_BACKEND`
//! names: `sqlite` (the default) in a temporary directory, or `postgres`
//! in a new database on the server `KIKOERU_TEST_PG` names, as a URL whose
//! user may create databases. CI runs the same tests once on each.

use std::sync::atomic::{AtomicU32, Ordering};

use crate::{Db, Location};

/// An open test database. The SQLite file goes away with it. PostgreSQL
/// databases stay until the throwaway server does.
pub struct TestDb {
    pub db: Db,
    pub location: Location,
    _dir: tempfile::TempDir,
}

/// Whether the tests run on PostgreSQL.
#[must_use]
pub fn on_postgres() -> bool {
    std::env::var("KIKOERU_TEST_BACKEND").is_ok_and(|b| b == "postgres")
}

/// Opens a new, migrated, empty library database.
///
/// # Panics
/// It cannot be created, or `KIKOERU_TEST_BACKEND` asks for PostgreSQL and
/// `KIKOERU_TEST_PG` is not set.
pub async fn open() -> TestDb {
    let dir = tempfile::tempdir().expect("cannot create a temporary directory");
    let location = if on_postgres() {
        postgres_location().await
    } else {
        Location::Sqlite(dir.path().join("neo.sqlite3"))
    };
    let db = Db::open(&location)
        .await
        .expect("cannot open the test database");
    TestDb {
        db,
        location,
        _dir: dir,
    }
}

#[cfg(feature = "postgres")]
async fn postgres_location() -> Location {
    use std::str::FromStr;

    use sqlx::{
        ConnectOptions, Connection,
        postgres::{PgConnectOptions, PgConnection},
    };

    static NEXT: AtomicU32 = AtomicU32::new(0);
    let url = std::env::var("KIKOERU_TEST_PG")
        .expect("KIKOERU_TEST_BACKEND=postgres needs KIKOERU_TEST_PG");
    let admin = PgConnectOptions::from_str(&url).expect("KIKOERU_TEST_PG is not a valid URL");
    let name = format!(
        "kneo_t_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let mut conn = PgConnection::connect_with(&admin)
        .await
        .expect("cannot reach KIKOERU_TEST_PG");
    // Real servers usually default to a locale that folds and orders more
    // than ASCII, unlike the test server's --no-locale. A database whose
    // default collation does that catches SQL that leans on the default
    // instead of the columns' COLLATE "C". The builtin provider needs no
    // OS locale (PostgreSQL 17 and later).
    let create = format!(
        "CREATE DATABASE {name} TEMPLATE template0 \
         LOCALE_PROVIDER builtin BUILTIN_LOCALE 'C.UTF-8'"
    );
    sqlx::query(sqlx::AssertSqlSafe(create))
        .execute(&mut conn)
        .await
        .expect("cannot create a test database");
    conn.close().await.ok();
    Location::Postgres(admin.database(&name).to_url_lossy().to_string())
}

#[cfg(not(feature = "postgres"))]
async fn postgres_location() -> Location {
    let _ = AtomicU32::new(0).fetch_add(0, Ordering::Relaxed);
    panic!("KIKOERU_TEST_BACKEND=postgres needs neo-db's postgres feature")
}

/// Runs `$body` with `$pool` bound to the `read` or `write` pool of
/// whichever backend `$db` is open on. Each backend's arm is checked on its
/// own, so `sqlx::query(...)` with `$1` placeholders works on both. For
/// tests that set up or inspect rows with SQL of their own.
#[cfg(feature = "postgres")]
#[macro_export]
macro_rules! with_pool {
    ($db:expr, $which:ident, $pool:ident => $body:expr) => {
        match $db {
            $crate::Db::Sqlite(db) => {
                let $pool = &db.$which;
                $body
            }
            $crate::Db::Postgres(db) => {
                let $pool = &db.$which;
                $body
            }
        }
    };
}

/// See the PostgreSQL build's version: without it, only SQLite.
#[cfg(not(feature = "postgres"))]
#[macro_export]
macro_rules! with_pool {
    ($db:expr, $which:ident, $pool:ident => $body:expr) => {
        match $db {
            $crate::Db::Sqlite(db) => {
                let $pool = &db.$which;
                $body
            }
        }
    };
}

/// A row type both backends can decode, such as a tuple of `i64`,
/// `String` and `bool`.
#[cfg(feature = "postgres")]
pub trait FromAnyRow:
    for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow>
    + for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow>
    + Send
    + Unpin
    + 'static
{
}

#[cfg(feature = "postgres")]
impl<T> FromAnyRow for T where
    T: for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow>
        + for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow>
        + Send
        + Unpin
        + 'static
{
}

/// See the PostgreSQL build's version.
#[cfg(not(feature = "postgres"))]
pub trait FromAnyRow:
    for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Unpin + 'static
{
}

#[cfg(not(feature = "postgres"))]
impl<T> FromAnyRow for T where
    T: for<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> + Send + Unpin + 'static
{
}

/// Runs a script of one or more statements on the write connection. The
/// SQL must suit both backends: `TRUE`/`FALSE` for flags, and bytes as
/// `CAST('...' AS BLOB)`, which becomes `BYTEA` on PostgreSQL. Rows with
/// explicit IDs in identity columns leave PostgreSQL's sequence behind, so
/// fixtures that the code under test adds to should let IDs be generated.
///
/// # Panics
/// The script fails.
pub async fn exec(db: &Db, sql: &str) {
    // The one spelling the fixtures need that differs: a text literal
    // stored as bytes.
    let sql = if matches!(db, Db::Sqlite(_)) {
        sql.to_owned()
    } else {
        sql.replace(" AS BLOB)", " AS BYTEA)")
    };
    let script = sqlx::AssertSqlSafe(sql.clone());
    crate::with_pool!(db, write, pool => sqlx::raw_sql(script).execute(pool).await.map(drop))
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
}

/// The first column of every row. Integers must be 64-bit on PostgreSQL:
/// cast a literal or a 32-bit expression with `CAST(... AS BIGINT)`.
///
/// # Panics
/// The query fails.
pub async fn column<T>(db: &Db, sql: &str) -> Vec<T>
where
    (T,): FromAnyRow,
{
    rows::<(T,)>(db, sql)
        .await
        .into_iter()
        .map(|(v,)| v)
        .collect()
}

/// The first column of the only row.
///
/// # Panics
/// The query fails or returns other than one row.
pub async fn value<T>(db: &Db, sql: &str) -> T
where
    (T,): FromAnyRow,
{
    let mut rows = column::<T>(db, sql).await;
    assert_eq!(rows.len(), 1, "{sql} returned {} rows", rows.len());
    rows.remove(0)
}

/// Every row as a tuple or another `FromRow` type.
///
/// # Panics
/// The query fails.
pub async fn rows<T: FromAnyRow>(db: &Db, sql: &str) -> Vec<T> {
    let query = sqlx::AssertSqlSafe(sql.to_owned());
    crate::with_pool!(db, write, pool => sqlx::query_as::<_, T>(query).fetch_all(pool).await)
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}
