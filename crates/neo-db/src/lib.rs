//! The library database, whichever backend holds it. Each function here
//! runs the same query from `queries/` on the backend `Db` was opened with
//! (book/src/data-model.md, "存储层的代码结构"). The row and argument types
//! come from `neo_core::db` and are re-exported module by module, so
//! callers import everything from here.

use std::path::{Path, PathBuf};

#[cfg(feature = "testing")]
pub mod testing;

pub use neo_core::db::{MIGRATOR, OpenError, is_busy, now_ms, retry_busy};
#[cfg(feature = "postgres")]
pub use neo_core::db::{PG_MIGRATOR, copy};

/// Where the library database is.
#[derive(Clone, PartialEq, Eq)]
pub enum Location {
    /// A SQLite file, normally `neo.sqlite3` in the data directory.
    Sqlite(PathBuf),
    /// A `postgres://` URL. The password can come from `PGPASSWORD` or a
    /// pgpass file instead of the URL.
    #[cfg(feature = "postgres")]
    Postgres(String),
}

/// The URL may hold a password, so `Debug` shows it the way error
/// messages do, without one.
impl std::fmt::Debug for Location {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite(path) => f.debug_tuple("Sqlite").field(path).finish(),
            #[cfg(feature = "postgres")]
            Self::Postgres(url) => f
                .debug_tuple("Postgres")
                .field(&copy::Endpoint::Postgres(url.clone()).describe())
                .finish(),
        }
    }
}

/// An open library database.
#[derive(Debug, Clone)]
pub enum Db {
    Sqlite(neo_db_sqlite::Db),
    #[cfg(feature = "postgres")]
    Postgres(neo_db_postgres::Db),
}

/// A write transaction, begun with [`Db::begin`]. Dropping it without
/// [`Tx::commit`] rolls it back.
#[derive(Debug)]
pub enum Tx<'c> {
    Sqlite(neo_db_sqlite::Tx<'c>),
    #[cfg(feature = "postgres")]
    Postgres(neo_db_postgres::Tx<'c>),
}

impl Db {
    /// Opens the database and applies every pending migration. A SQLite
    /// file is created if needed.
    ///
    /// # Errors
    /// The database cannot be opened or reached, or a migration fails.
    pub async fn open(location: &Location) -> Result<Self, OpenError> {
        match location {
            Location::Sqlite(path) => Self::open_sqlite(path).await,
            #[cfg(feature = "postgres")]
            Location::Postgres(url) => {
                let describe = copy::Endpoint::Postgres(url.clone()).describe();
                Ok(Self::Postgres(
                    neo_db_postgres::Db::open(url, &describe).await?,
                ))
            }
        }
    }

    /// Opens (creating if needed) the SQLite database at `path` and
    /// applies every pending migration.
    ///
    /// # Errors
    /// The file cannot be opened or created, or a migration fails.
    pub async fn open_sqlite(path: &Path) -> Result<Self, OpenError> {
        Ok(Self::Sqlite(neo_db_sqlite::Db::open(path).await?))
    }

    pub async fn close(&self) {
        match self {
            Self::Sqlite(db) => db.close().await,
            #[cfg(feature = "postgres")]
            Self::Postgres(db) => db.close().await,
        }
    }

    /// Begins a transaction on the write connection.
    ///
    /// # Errors
    /// Any database error.
    pub async fn begin(&self) -> sqlx::Result<Tx<'static>> {
        Ok(match self {
            Self::Sqlite(db) => Tx::Sqlite(db.write.begin().await?),
            #[cfg(feature = "postgres")]
            Self::Postgres(db) => Tx::Postgres(db.write.begin().await?),
        })
    }

    /// Begins a transaction that no other writer, in this process or
    /// another, can commit into between its reads and its writes. On
    /// SQLite it takes the write lock before the first read. On PostgreSQL
    /// it is serializable, as every write transaction is. A conflict fails
    /// with an error [`is_busy`] accepts, so the caller can run the whole
    /// transaction again.
    ///
    /// # Errors
    /// Any database error.
    pub async fn begin_exclusive(&self) -> sqlx::Result<Tx<'static>> {
        Ok(match self {
            Self::Sqlite(db) => Tx::Sqlite(db.write.begin_with("BEGIN IMMEDIATE").await?),
            #[cfg(feature = "postgres")]
            Self::Postgres(db) => Tx::Postgres(db.write.begin().await?),
        })
    }

    /// Rows in `table`, which must be one of the schema's table names.
    ///
    /// # Errors
    /// Any database error.
    pub async fn count_rows(&self, table: &'static str) -> sqlx::Result<i64> {
        let sql = sqlx::AssertSqlSafe(format!("SELECT COUNT(*) FROM {table}"));
        match self {
            Self::Sqlite(db) => sqlx::query_scalar(sql).fetch_one(&db.read).await,
            #[cfg(feature = "postgres")]
            Self::Postgres(db) => sqlx::query_scalar(sql).fetch_one(&db.read).await,
        }
    }

    /// The SQLite pools, for the few places that run SQL only SQLite has,
    /// such as `VACUUM INTO` and `PRAGMA integrity_check`. `None` on
    /// PostgreSQL.
    #[must_use]
    pub fn sqlite(&self) -> Option<&neo_db_sqlite::Db> {
        match self {
            Self::Sqlite(db) => Some(db),
            #[cfg(feature = "postgres")]
            Self::Postgres(_) => None,
        }
    }

    /// The PostgreSQL pools, for tests that set up rows with SQL of their
    /// own. `None` on SQLite.
    #[cfg(feature = "postgres")]
    #[must_use]
    pub fn postgres(&self) -> Option<&neo_db_postgres::Db> {
        match self {
            Self::Postgres(db) => Some(db),
            Self::Sqlite(_) => None,
        }
    }
}

impl Tx<'_> {
    /// # Errors
    /// Any database error. The transaction is rolled back then.
    pub async fn commit(self) -> sqlx::Result<()> {
        match self {
            Self::Sqlite(tx) => tx.commit().await,
            #[cfg(feature = "postgres")]
            Self::Postgres(tx) => tx.commit().await,
        }
    }

    /// # Errors
    /// Any database error.
    pub async fn rollback(self) -> sqlx::Result<()> {
        match self {
            Self::Sqlite(tx) => tx.rollback().await,
            #[cfg(feature = "postgres")]
            Self::Postgres(tx) => tx.rollback().await,
        }
    }
}

/// Defines, for each listed query, a function of the same name that takes
/// the [`Db`] (or a [`Tx`]) in place of a pool and runs the query on the
/// open backend. `read` and `write` name the pool the query runs on, `tx`
/// a query that runs inside the caller's transaction.
macro_rules! dispatch {
    ($module:ident; $( $via:ident fn $name:ident ( $($arg:ident : $ty:ty),* $(,)? ) -> $ret:ty ; )*) => {
        $( dispatch!(@one $via $module $name ( $($arg : $ty),* ) -> $ret); )*
    };
    (@one read $module:ident $name:ident ( $($arg:ident : $ty:ty),* ) -> $ret:ty) => {
        /// See the query of the same name in `queries/`.
        ///
        /// # Errors
        /// As the query.
        pub async fn $name(db: &crate::Db, $($arg: $ty),*) -> $ret {
            match db {
                crate::Db::Sqlite(db) => neo_db_sqlite::$module::$name(&db.read, $($arg),*).await,
                #[cfg(feature = "postgres")]
                crate::Db::Postgres(db) => neo_db_postgres::$module::$name(&db.read, $($arg),*).await,
            }
        }
    };
    (@one write $module:ident $name:ident ( $($arg:ident : $ty:ty),* ) -> $ret:ty) => {
        /// See the query of the same name in `queries/`.
        ///
        /// # Errors
        /// As the query.
        pub async fn $name(db: &crate::Db, $($arg: $ty),*) -> $ret {
            match db {
                crate::Db::Sqlite(db) => neo_db_sqlite::$module::$name(&db.write, $($arg),*).await,
                #[cfg(feature = "postgres")]
                crate::Db::Postgres(db) => neo_db_postgres::$module::$name(&db.write, $($arg),*).await,
            }
        }
    };
    (@one tx $module:ident $name:ident ( $($arg:ident : $ty:ty),* ) -> $ret:ty) => {
        /// See the query of the same name in `queries/`.
        ///
        /// # Errors
        /// As the query.
        pub async fn $name(tx: &mut crate::Tx<'_>, $($arg: $ty),*) -> $ret {
            match tx {
                crate::Tx::Sqlite(tx) => neo_db_sqlite::$module::$name(tx, $($arg),*).await,
                #[cfg(feature = "postgres")]
                crate::Tx::Postgres(tx) => neo_db_postgres::$module::$name(tx, $($arg),*).await,
            }
        }
    };
}

pub mod analysis {
    pub use neo_core::db::analysis::*;

    dispatch! { analysis;
        read fn loudness(track_id: i64, size: Option<i64>, mtime: Option<i64>, now: i64) -> Result<Option<StoredLoudness>, sqlx::Error>;
        write fn save(track: &crate::media::TrackFile, size: Option<i64>, mtime: Option<i64>, loudness: StoredLoudness, now: i64) -> Result<(), sqlx::Error>;
    }
}

pub mod detail {
    pub use neo_core::db::detail::*;

    dispatch! { detail;
        read fn get(user_id: i64, work_id: i64, with_locations: bool) -> Result<Option<neo_core::dto::WorkDetail>, DetailError>;
    }
}

pub mod edit {
    pub use neo_core::db::edit::*;

    dispatch! { edit;
        write fn set_metadata(work_id: i64, edit: &MetadataEdit, now: i64) -> sqlx::Result<bool>;
        tx fn va_id(name: &str) -> sqlx::Result<String>;
        read fn missing() -> Result<neo_core::dto::MissingReport, MissingError>;
        write fn cleanup(work_ids: &[i64], location_ids: &[i64]) -> sqlx::Result<Cleanup>;
    }
}

pub mod legacy_state {
    pub use neo_core::db::legacy_state::*;

    dispatch! { legacy_state;
        tx fn put(state: &State) -> Result<(), sqlx::Error>;
        tx fn claim(work_id: i64, code: &str) -> Result<Claimed, sqlx::Error>;
    }
}

pub mod library {
    pub use neo_core::db::library::*;

    dispatch! { library;
        write fn set_favorite(user_id: i64, work_id: i64, favorite: bool, now: i64) -> sqlx::Result<bool>;
        read fn circles() -> Result<Vec<neo_core::dto::CircleCount>, LibraryError>;
        read fn vas() -> Result<Vec<neo_core::dto::VaCount>, LibraryError>;
        read fn tags() -> Result<Vec<neo_core::dto::TagCount>, LibraryError>;
    }
}

pub mod media {
    pub use neo_core::db::media::*;

    dispatch! { media;
        read fn root_name(track_id: i64) -> Result<Option<String>, sqlx::Error>;
        read fn track_file(id: i64) -> Result<Option<TrackFile>, sqlx::Error>;
        read fn subtitle_for(track: &TrackFile, extensions: &[&str]) -> Result<Option<i64>, sqlx::Error>;
        read fn work_fonts(work_id: i64) -> Result<Vec<(i64, String)>, sqlx::Error>;
        write fn save_probe(track: &TrackFile, probe_json: &str, duration_ms: Option<i64>) -> Result<(), sqlx::Error>;
        read fn work_code(work_id: i64) -> Result<Option<String>, sqlx::Error>;
        read fn work_files(work_id: i64) -> Result<Vec<TrackFile>, sqlx::Error>;
        read fn track_version(id: i64) -> Result<Option<(i64, Option<i64>, Option<i64>)>, sqlx::Error>;
        read fn source_version(root_id: &str) -> Result<Option<i64>, sqlx::Error>;
        read fn cover_state(work_id: i64) -> Result<Option<CoverState>, sqlx::Error>;
    }
}

pub mod progress {
    pub use neo_core::db::progress::*;

    dispatch! { progress;
        read fn track_place(track_id: i64) -> Result<Option<TrackPlace>, sqlx::Error>;
        read fn ends_group(work_id: i64, group_path: &str, track_id: i64) -> Result<bool, sqlx::Error>;
        write fn report(user_id: i64, work_id: i64, input: &ReportInput<'_>, now: i64) -> Result<(), sqlx::Error>;
        write fn finish(user_id: i64, work_id: i64, track_id: i64, group_path: &str, now: i64) -> Result<(), sqlx::Error>;
        write fn clear(user_id: i64, work_id: i64) -> Result<(), sqlx::Error>;
    }
}

pub mod reviews {
    pub use neo_core::db::reviews::*;

    dispatch! { reviews;
        read fn get(user_id: i64, work_id: i64) -> Result<Option<neo_core::dto::Review>, ReviewError>;
        write fn put(user_id: i64, work_id: i64, review: &neo_core::review::Valid, now: i64) -> Result<Option<neo_core::dto::Review>, ReviewError>;
        write fn delete(user_id: i64, work_id: i64) -> sqlx::Result<bool>;
        tx fn insert_legacy(user_id: i64, work_id: i64, review: &neo_core::review::Valid, updated_at: Option<i64>) -> sqlx::Result<u64>;
    }
}

pub mod roots {
    pub use neo_core::db::roots::*;

    dispatch! { roots;
        write fn sync(roots: &[(&str, &str)]) -> sqlx::Result<()>;
        read fn with_works_outside(known: &[&str]) -> sqlx::Result<Vec<(String, i64)>>;
        read fn list_stored() -> sqlx::Result<Vec<StoredRoot>>;
        write fn list_stored_for_cleanup() -> sqlx::Result<Vec<StoredRoot>>;
        write fn insert_stored(root: &StoredRoot, now: i64) -> sqlx::Result<bool>;
        write fn update_stored(root: &StoredRoot, location_changed: bool, now: i64) -> sqlx::Result<bool>;
        tx fn remove_stored(id: &str, now: i64) -> sqlx::Result<Option<Vec<i64>>>;
        read fn last_scan_end_at(id: &str) -> sqlx::Result<Option<i64>>;
        read fn status(id: &str) -> sqlx::Result<Option<RootRow>>;
        write fn record_scan_end(id: &str, now: i64) -> sqlx::Result<()>;
    }
}

pub mod server_settings {
    dispatch! { server_settings;
        read fn metadata_proxy() -> Result<Option<String>, sqlx::Error>;
        write fn set_metadata_proxy(proxy: Option<&str>, now: i64) -> Result<(), sqlx::Error>;
    }
}

pub mod settings {
    pub use neo_core::db::settings::*;

    dispatch! { settings;
        read fn get(user_id: i64) -> Result<Settings, sqlx::Error>;
        write fn put(user_id: i64, settings: Settings, now: i64) -> Result<(), sqlx::Error>;
    }
}

pub mod tags {
    pub use neo_core::db::tags::*;

    dispatch! { tags;
        tx fn resolve(name: &str, dlsite_id: Option<i64>) -> sqlx::Result<i64>;
        write fn merge(from: i64, into: i64, now: i64) -> sqlx::Result<MergeOutcome>;
    }
}

pub mod tasks {
    pub use neo_core::db::tasks::*;
    #[cfg(all(feature = "testing", feature = "postgres"))]
    pub use neo_db_postgres::tasks::ENQUEUE_COMMIT_OUTCOME;

    dispatch! { tasks;
        write fn enqueue(kind: &str, dedupe_key: Option<&str>, payload: &str, queue_limit: Option<i64>, now: i64) -> Result<Enqueued, sqlx::Error>;
        read fn active_id(dedupe_key: &str) -> Result<Option<i64>, sqlx::Error>;
        read fn active_state(dedupe_key: &str) -> Result<Option<String>, sqlx::Error>;
        read fn get(id: i64) -> Result<Option<TaskRow>, sqlx::Error>;
        read fn list(state: Option<&str>, exclude_kind: Option<&str>, limit: u32, offset: u64) -> Result<Vec<TaskRow>, sqlx::Error>;
        read fn count(state: Option<&str>, exclude_kind: Option<&str>) -> Result<i64, sqlx::Error>;
        write fn set_message(id: i64, message: &str, now: i64) -> Result<(), sqlx::Error>;
        write fn start(id: i64, now: i64) -> Result<bool, sqlx::Error>;
        write fn cancel_queued(id: i64, now: i64) -> Result<bool, sqlx::Error>;
        write fn unstart(id: i64, now: i64) -> Result<(), sqlx::Error>;
        write fn move_to_front(id: i64) -> Result<bool, sqlx::Error>;
        write fn set_progress(id: i64, progress: f64, now: i64) -> Result<(), sqlx::Error>;
        write fn finish(id: i64, state: EndState, result: Option<&str>, message: Option<&str>, now: i64) -> Result<bool, sqlx::Error>;
        write fn recover(now: i64) -> Result<Recovered, sqlx::Error>;
        write fn prune(older_than: i64, keep: i64) -> Result<u64, sqlx::Error>;
    }
}

pub mod users {
    pub use neo_core::db::users::*;

    dispatch! { users;
        read fn find_by_name(name: &str) -> sqlx::Result<Option<UserRecord>>;
        read fn find_by_id(id: i64) -> sqlx::Result<Option<UserRecord>>;
        read fn admin_exists() -> sqlx::Result<bool>;
        read fn without_password() -> sqlx::Result<Vec<(String, bool)>>;
        write fn create(name: &str, password_hash: &str, is_admin: bool, token_version: i64, now_ms: i64) -> Result<i64, CreateError>;
        write fn set_password(id: i64, password_hash: &str) -> sqlx::Result<bool>;
        write fn revoke_tokens(id: i64) -> sqlx::Result<bool>;
        write fn set_password_if_version(id: i64, expected_version: i64, password_hash: &str) -> sqlx::Result<bool>;
        write fn delete(name: &str) -> sqlx::Result<DeleteOutcome>;
    }
}

pub mod works {
    pub use neo_core::db::works::*;

    dispatch! { works;
        read fn count(query: &ListQuery) -> sqlx::Result<i64>;
        read fn summary(user_id: i64, work_id: i64) -> Result<Option<neo_core::dto::WorkSummary>, ListError>;
        read fn list(query: &ListQuery) -> Result<Vec<neo_core::dto::WorkSummary>, ListError>;
        read fn exists(work_id: i64) -> sqlx::Result<bool>;
    }
}

pub mod scan {
    pub use neo_core::db::scan::*;

    dispatch! { scan;
        read fn root_exists(root_id: &str) -> sqlx::Result<bool>;
        write fn set_root_status(root_id: &str, status: &str, scanned_at: Option<i64>) -> sqlx::Result<()>;
        read fn root_locations(root_id: &str) -> sqlx::Result<Vec<LocationRow>>;
        tx fn restore_location(location_id: i64) -> sqlx::Result<()>;
        tx fn work_by_code(code: &str) -> sqlx::Result<Option<i64>>;
        tx fn insert_work(code: &str, title: &str, now: i64) -> sqlx::Result<i64>;
        tx fn insert_location(work_id: i64, root_id: &str, rel_path: &[u8], display: &str) -> sqlx::Result<i64>;
        tx fn mark_location_missing(location_id: i64, now: i64) -> sqlx::Result<()>;
        tx fn work_state(work_id: i64) -> sqlx::Result<WorkState>;
        tx fn set_work_missing(work_id: i64, missing_at: Option<i64>) -> sqlx::Result<()>;
        read fn stored_digests(root_id: &str) -> sqlx::Result<Vec<(i64, String)>>;
        read fn location_tracks(location_id: i64) -> sqlx::Result<Vec<TrackRow>>;
        read fn work_extra(work_id: i64) -> sqlx::Result<String>;
        tx fn update_changed_track(track_id: i64, file: ListedFile<'_>) -> sqlx::Result<()>;
        tx fn restore_track(track_id: i64) -> sqlx::Result<()>;
        tx fn insert_track(work_id: i64, location_id: i64, rel_path: &[u8], file: ListedFile<'_>, imported_duration_ms: Option<i64>) -> sqlx::Result<i64>;
        tx fn mark_track_missing(track_id: i64, now: i64) -> sqlx::Result<()>;
        tx fn set_listed(location_id: i64, now: i64, digest: &str) -> sqlx::Result<()>;
        tx fn work_copies(work_id: i64) -> sqlx::Result<Vec<CopyRow>>;
        tx fn set_duplicate_of(track_id: i64, kept: Option<i64>) -> sqlx::Result<()>;
        tx fn move_progress(from: i64, to: i64) -> sqlx::Result<()>;
        tx fn pending_imports(work_id: i64) -> sqlx::Result<Vec<PendingRow>>;
        tx fn unlisted_locations(work_id: i64) -> sqlx::Result<i64>;
        tx fn progress_track(user_id: i64, work_id: i64) -> sqlx::Result<Option<Option<i64>>>;
        tx fn tracks_at(work_id: i64, display_path: &str) -> sqlx::Result<Vec<i64>>;
        tx fn set_progress_track(user_id: i64, work_id: i64, track_id: i64, group_path: &str) -> sqlx::Result<()>;
        tx fn set_pending_state(user_id: i64, work_id: i64, state: &str) -> sqlx::Result<()>;
    }
}

pub mod metadata {
    pub use neo_core::db::metadata::*;

    dispatch! { metadata;
        read fn sources(work_id: i64) -> sqlx::Result<Option<Sources>>;
        read fn cover_version(work_id: i64) -> sqlx::Result<Option<i64>>;
        tx fn stored(work_id: i64) -> sqlx::Result<Option<Stored>>;
        tx fn bump_cover(work_id: i64, cover_src: &str, now: i64) -> sqlx::Result<()>;
        tx fn upsert_circle(circle_id: i64, name: &str, work_id: i64) -> sqlx::Result<()>;
        tx fn update_work(work_id: i64, fields: Fields<'_>, now: i64) -> sqlx::Result<()>;
        tx fn replace_credits(work_id: i64, vas: &[String], tag_names: &[(String, Option<i64>)]) -> sqlx::Result<()>;
        read fn works_without_metadata(root_id: &str) -> sqlx::Result<Vec<i64>>;
        read fn folder_images(root_id: &str) -> sqlx::Result<Vec<FolderImage>>;
    }
}

pub mod import {
    pub use neo_core::db::import::*;

    dispatch! { import;
        tx fn insert_root(id: &str, display_name: &str) -> sqlx::Result<()>;
        tx fn insert_circle(id: i64, name: &str) -> sqlx::Result<()>;
        tx fn insert_va(id: &str, name: &str) -> sqlx::Result<()>;
        tx fn insert_tag(id: i64, name: &str) -> sqlx::Result<()>;
        tx fn insert_work(work: ImportedWork<'_>) -> sqlx::Result<i64>;
        tx fn insert_work_tag(work_id: i64, tag_id: i64) -> sqlx::Result<()>;
        tx fn insert_work_va(work_id: i64, va_id: &str) -> sqlx::Result<()>;
        tx fn user_id(name: &str) -> sqlx::Result<Option<i64>>;
        tx fn insert_user(name: &str, is_admin: bool, token_version: i64, now: i64) -> sqlx::Result<i64>;
        tx fn insert_favorite(user_id: i64, work_id: i64, created_at: i64) -> sqlx::Result<()>;
        tx fn insert_progress(user_id: i64, work_id: i64, position_ms: i64, finished_at: Option<i64>, updated_at: i64) -> sqlx::Result<()>;
        tx fn insert_legacy_review(user_id: i64, work_id: i64, rating: Option<i64>, review_text: Option<&str>, progress: Option<&str>) -> sqlx::Result<()>;
        tx fn insert_pending(user_id: i64, work_id: i64, source: &str, durations: Option<&str>, state: &str) -> sqlx::Result<()>;
        tx fn waiting_codes() -> sqlx::Result<Vec<String>>;
        tx fn waiting_count() -> sqlx::Result<i64>;
        read fn imported_covers() -> sqlx::Result<i64>;
    }
}
