//! Reads the old kikoeru database. The old schema is not part of the SQLx
//! offline metadata, so these queries are checked by the import tests on a
//! synthetic database built with the old DDL, not at compile time.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::Path,
};

use anyhow::{Context, Result};

use super::error::{ErrorCode, refuse};
use serde_json::{Map, Value};
use sqlx::{
    FromRow, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

#[derive(Debug, Clone, FromRow)]
pub struct Work {
    pub id: i64,
    pub root_folder: String,
    pub dir: String,
    pub title: String,
    pub circle_id: i64,
    pub nsfw: Option<bool>,
    pub release: Option<String>,
    pub dl_count: Option<i64>,
    pub price: Option<i64>,
    pub rate_count: Option<i64>,
    pub rate_average: Option<f64>,
    pub rate_count_detail: Option<String>,
    pub rank: Option<String>,
    pub memo: Option<String>,
    pub is_custom_meta: Option<i64>,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
}

#[derive(Debug, Clone, FromRow)]
pub struct User {
    pub name: String,
    pub group: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct Review {
    pub user_name: String,
    pub work_id: i64,
    pub rating: Option<i64>,
    pub review_text: Option<String>,
    pub progress: Option<String>,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
}

#[derive(Debug, Clone, FromRow)]
pub struct History {
    pub user_name: String,
    pub work_id: i64,
    pub state: String,
    pub updated_at: Option<i64>,
}

/// A work's track durations: path relative to the work directory to
/// seconds, the shape of the old memo's `duration` object.
pub type Durations = Map<String, Value>;

/// Everything the import reads from the old database, loaded at once. The
/// largest table has about 13 000 rows, so memory is not a concern.
#[derive(Debug, Clone, Default)]
pub struct Legacy {
    pub last_migration: Option<String>,
    pub circles: Vec<(i64, String)>,
    pub vas: Vec<(String, String)>,
    pub tags: Vec<(i64, String)>,
    pub works: Vec<Work>,
    pub work_tags: Vec<(i64, i64)>,
    pub work_vas: Vec<(i64, String)>,
    pub users: Vec<User>,
    pub reviews: Vec<Review>,
    pub histories: Vec<History>,
    /// Durations from `t_track`, by old work ID.
    pub track_durations: BTreeMap<i64, Durations>,
    pub translate_tasks: i64,
}

impl Legacy {
    /// Opens the database read-only, so that even a copy is never changed.
    pub async fn load(path: &Path) -> Result<Self> {
        Self::load_as(path, path).await
    }

    /// Like `load`, for a private copy at `path` of the user's `source`.
    /// Errors name `source`, because the copy is gone by the time the user
    /// reads them.
    pub async fn load_as(path: &Path, source: &Path) -> Result<Self> {
        if !path.is_file() {
            refuse!(
                ErrorCode::SourceNotFound,
                "{} does not exist",
                source.display()
            );
        }
        // immutable makes SQLite ignore a -wal or hot -journal file, so
        // committed changes in either would be silently left out.
        for suffix in ["-wal", "-journal"] {
            let mut side = path.as_os_str().to_owned();
            side.push(suffix);
            if std::fs::metadata(&side).is_ok_and(|m| m.len() > 0) {
                refuse!(
                    ErrorCode::UnmergedJournal,
                    "{} exists, so the copy may hold changes that are not in the main file; \
                     stop the old server, let it close the database, and copy again",
                    Path::new(&side).display()
                );
            }
        }
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            // immutable keeps SQLite from creating -wal or -shm files next to
            // the copy, which read_only alone allows for a WAL database.
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(path)
                    .read_only(true)
                    .immutable(true),
            )
            .await
            .with_context(|| format!("cannot open {}", source.display()))?;
        let result = read_all(&pool)
            .await
            .with_context(|| format!("{} does not look like a kikoeru database", source.display()));
        pool.close().await;
        result
    }
}

/// The tables and columns the old database has. The import asks the
/// database instead of trusting `knex_migrations`, which also lists
/// migrations that failed.
#[derive(Debug, Clone, Default)]
struct Columns(HashMap<String, BTreeSet<String>>);

impl Columns {
    async fn read(pool: &SqlitePool) -> Result<Self> {
        let tables: Vec<String> =
            sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table'")
                .fetch_all(pool)
                .await
                .context("listing tables")?;
        let mut map = HashMap::new();
        for table in tables {
            let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info(?)")
                .bind(&table)
                .fetch_all(pool)
                .await
                .with_context(|| format!("reading the columns of {table}"))?;
            map.insert(table, columns.into_iter().collect());
        }
        Ok(Self(map))
    }

    fn has_table(&self, table: &str) -> bool {
        self.0.contains_key(table)
    }

    fn has(&self, table: &str, column: &str) -> bool {
        self.0.get(table).is_some_and(|c| c.contains(column))
    }

    /// Fails with every required column the database lacks, so a database
    /// from another program is named as such.
    fn require(&self, required: &[(&str, &[&str])]) -> Result<()> {
        let mut missing = Vec::new();
        for (table, columns) in required {
            if !self.has_table(table) {
                missing.push((*table).to_owned());
                continue;
            }
            missing.extend(
                columns
                    .iter()
                    .filter(|c| !self.has(table, c))
                    .map(|c| format!("{table}.{c}")),
            );
        }
        if missing.is_empty() {
            Ok(())
        } else {
            refuse!(ErrorCode::NotKikoeru, "missing {}", missing.join(", "))
        }
    }
}

/// The schema of kikoeru 0.6.14, the oldest version the import supports.
/// `is_custom_meta` came with 0.6.14, so its absence means an older one.
const REQUIRED: &[(&str, &[&str])] = &[
    ("knex_migrations", &["id", "name"]),
    ("t_circle", &["id", "name"]),
    ("t_va", &["id", "name"]),
    ("t_tag", &["id", "name"]),
    (
        "t_work",
        &[
            "id",
            "root_folder",
            "dir",
            "title",
            "circle_id",
            "nsfw",
            "release",
            "dl_count",
            "price",
            "rate_count",
            "rate_average_2dp",
            "rate_count_detail",
            "rank",
            "memo",
            "is_custom_meta",
            "created_at",
            "updated_at",
        ],
    ),
    ("r_tag_work", &["tag_id", "work_id"]),
    ("r_va_work", &["va_id", "work_id"]),
    ("t_user", &["name", "group"]),
    (
        "t_review",
        &[
            "user_name",
            "work_id",
            "rating",
            "review_text",
            "progress",
            "created_at",
            "updated_at",
        ],
    ),
    (
        "t_play_histroy",
        &["user_name", "work_id", "state", "updated_at"],
    ),
    ("t_translate_task", &["id"]),
];

/// Old timestamps are `CURRENT_TIMESTAMP` text, which is UTC. SQLite parses
/// them, and anything it cannot parse becomes NULL and is reported.
macro_rules! ms {
    ($col:literal) => {
        concat!("CAST(strftime('%s', ", $col, ") AS INTEGER) * 1000")
    };
}

async fn read_all(pool: &SqlitePool) -> Result<Legacy> {
    let cols = Columns::read(pool).await?;
    if cols.has_table("t_work") && !cols.has("t_work", "is_custom_meta") {
        refuse!(
            ErrorCode::TooOld,
            "it comes from a kikoeru version older than 0.6.14; upgrade the old server to \
             0.6.14 or later, let it start once, and import again"
        );
    }
    cols.require(REQUIRED)?;
    // Work IDs are cast because a fresh install declares t_review.work_id
    // as a string column, which stores the number as text.
    let works = sqlx::query_as::<_, Work>(concat!(
        "SELECT id, root_folder, dir, title, circle_id, nsfw, release, dl_count, price,
                rate_count, rate_average_2dp AS rate_average, rate_count_detail, rank, memo,
                is_custom_meta, ",
        ms!("created_at"),
        " AS created_at, ",
        ms!("updated_at"),
        " AS updated_at FROM t_work ORDER BY id"
    ))
    .fetch_all(pool)
    .await
    .context("reading t_work")?;
    let reviews = sqlx::query_as::<_, Review>(concat!(
        "SELECT user_name, CAST(work_id AS INTEGER) AS work_id, rating, review_text, progress, ",
        ms!("created_at"),
        " AS created_at, ",
        ms!("updated_at"),
        " AS updated_at FROM t_review ORDER BY user_name, work_id"
    ))
    .fetch_all(pool)
    .await
    .context("reading t_review")?;
    let histories = sqlx::query_as::<_, History>(concat!(
        "SELECT user_name, CAST(work_id AS INTEGER) AS work_id, CAST(state AS TEXT) AS state, ",
        ms!("updated_at"),
        " AS updated_at FROM t_play_histroy ORDER BY user_name, work_id"
    ))
    .fetch_all(pool)
    .await
    .context("reading t_play_histroy")?;
    Ok(Legacy {
        last_migration: sqlx::query_scalar("SELECT name FROM knex_migrations ORDER BY id DESC")
            .fetch_optional(pool)
            .await
            .context("reading knex_migrations")?,
        circles: sqlx::query_as("SELECT id, name FROM t_circle ORDER BY id")
            .fetch_all(pool)
            .await
            .context("reading t_circle")?,
        vas: sqlx::query_as("SELECT id, name FROM t_va ORDER BY id")
            .fetch_all(pool)
            .await
            .context("reading t_va")?,
        tags: sqlx::query_as("SELECT id, name FROM t_tag ORDER BY id")
            .fetch_all(pool)
            .await
            .context("reading t_tag")?,
        works,
        work_tags: sqlx::query_as(
            "SELECT CAST(work_id AS INTEGER), tag_id FROM r_tag_work ORDER BY 1, 2",
        )
        .fetch_all(pool)
        .await
        .context("reading r_tag_work")?,
        work_vas: sqlx::query_as(
            "SELECT CAST(work_id AS INTEGER), va_id FROM r_va_work ORDER BY 1, 2",
        )
        .fetch_all(pool)
        .await
        .context("reading r_va_work")?,
        users: sqlx::query_as(r#"SELECT name, "group" FROM t_user ORDER BY name"#)
            .fetch_all(pool)
            .await
            .context("reading t_user")?,
        reviews,
        histories,
        track_durations: track_durations(pool, &cols).await?,
        translate_tasks: sqlx::query_scalar("SELECT COUNT(*) FROM t_translate_task")
            .fetch_one(pool)
            .await
            .context("reading t_translate_task")?,
    })
}

/// Track durations from `t_track`, by work. kikoeru 0.6.17 moved them there
/// from the work memo, and a rescan in 0.6.17 drops them from the memo.
async fn track_durations(pool: &SqlitePool, cols: &Columns) -> Result<BTreeMap<i64, Durations>> {
    let mut by_work = BTreeMap::<i64, Durations>::new();
    if !["work_id", "rel_path", "duration"]
        .iter()
        .all(|c| cols.has("t_track", c))
    {
        return Ok(by_work);
    }
    let rows: Vec<(i64, String, f64)> = sqlx::query_as(
        "SELECT CAST(work_id AS INTEGER), rel_path, duration FROM t_track
         WHERE duration IS NOT NULL ORDER BY 1, 2",
    )
    .fetch_all(pool)
    .await
    .context("reading t_track")?;
    for (work_id, rel_path, seconds) in rows {
        if let Some(n) = serde_json::Number::from_f64(seconds) {
            by_work
                .entry(work_id)
                .or_default()
                .insert(rel_path, Value::Number(n));
        }
    }
    Ok(by_work)
}
