#![cfg(feature = "postgres")]
//! `neo_db::copy` between SQLite files and PostgreSQL databases.

mod pg;

use std::path::{Path, PathBuf};

use neo_db::{
    Db,
    copy::{COPY_LOCK, CopyError, Endpoint, copy},
};

/// A row in every table, with NULLs, non-ASCII text, a blob that is not
/// UTF-8, the largest i64, flags both ways, and a track whose duplicate_of
/// points at a later row.
const FIXTURE: &str = r#"
INSERT INTO roots (id, display_name, last_scan_at, status, last_scan_end_at)
  VALUES ('lib', 'Library', 1700000000000, 'ok', NULL), ('cloud', '网盘 ☁', NULL, 'unknown', 5);
INSERT INTO circles (id, name) VALUES (1, 'Circle'), (9000000001, 'カスタム');
INSERT INTO vas (id, name) VALUES ('va-1', '声優 A'), ('va-2', 'O''Brien');
INSERT INTO tags (id, name) VALUES (1, 'tag'), (2, '标签');
INSERT INTO tag_aliases (alias, tag_id) VALUES ('Tag', 1), ('標籤', 2);
INSERT INTO tag_merges (from_id, into_id, merged_at) VALUES (3, 1, 42);
INSERT INTO works (id, code, title, circle_id, release_date, nsfw, dl_count, price, rate_average,
                   rate_count, metadata_src, cover_src, cover_version, extra, missing_at,
                   created_at, updated_at)
  VALUES (1, 'RJ000001', 'Title one', 1, '2024-01-02', 1, 100, 1320, 4.55, 7, 'dlsite', 'dlsite',
          2, '{"a":1}', NULL, 10, 11),
         (5, 'RJ01000005', 'タイトル
二', 9000000001, NULL, 0, NULL, NULL, NULL, NULL, 'manual', 'none', 0, '{}', 99, 12, 13);
INSERT INTO work_locations (id, work_id, root_id, rel_path, display_path, listing_hash,
                            listed_at, missing_at)
  VALUES (1, 1, 'lib', X'524AFFFE00', 'RJ�', 'abc', 5, NULL),
         (7, 5, 'cloud', CAST('RJ01000005' AS BLOB), 'RJ01000005', NULL, NULL, 3);
INSERT INTO work_vas (work_id, va_id) VALUES (1, 'va-1'), (5, 'va-2');
INSERT INTO work_tags (work_id, tag_id) VALUES (1, 1), (1, 2);
INSERT INTO tracks (id, work_id, location_id, rel_path, display_path, sort_key, kind, size, mtime,
                    duration_ms, duration_src, probe, duplicate_of, missing_at)
  VALUES (1, 1, 1, X'61', 'a.mp3', 'a', 'audio', 10, 20, 3000, 'probe', '{"format":{}}', 3, NULL),
         (2, 1, 1, X'62', 'mp3/b.mp3', 'b', 'audio', NULL, NULL, NULL, NULL, NULL, NULL, NULL),
         (3, 5, 7, X'63', 'c.wav', 'c', 'audio', 9223372036854775807, -1, 0, 'tag', NULL, NULL, 77);
INSERT INTO track_analysis (track_id, size, mtime, integrated_lufs, true_peak_dbtp, analyzed_at)
  VALUES (1, 10, 20, -14.25, -0.5, 30);
INSERT INTO users (id, name, password_hash, is_admin, token_version, created_at)
  VALUES (1, 'admin', '$argon2id$x', 1, 123456789, 1), (4, 'guest', NULL, 0, 0, 2);
INSERT INTO user_settings (user_id, normalize_loudness, target_lufs, updated_at, accent,
                           seek_back_seconds, seek_forward_seconds)
  VALUES (1, 1, -18.5, 3, 'pink', 10, 45), (4, 0, -16, 4, NULL, 15, 30);
INSERT INTO favorites (user_id, work_id, created_at) VALUES (1, 1, 5), (4, 5, 6);
INSERT INTO play_progress (user_id, work_id, track_id, position_ms, group_path, finished_at,
                           replaying, updated_at)
  VALUES (1, 1, 2, 1500, 'mp3', 7, 1, 8), (4, 5, NULL, 0, NULL, NULL, 0, 9);
INSERT INTO progress_report_cursors (user_id, work_id, session_id, last_sequence)
  VALUES (1, 1, 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 2),
         (1, 1, 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb', 7),
         (4, 5, 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 1);
INSERT INTO legacy_reviews (user_id, work_id, rating, review_text, progress)
  VALUES (1, 1, 5, 'good', 'listened'), (4, 5, NULL, NULL, NULL);
INSERT INTO reviews (user_id, work_id, rating, text, updated_at)
  VALUES (1, 1, 5, '好', 10), (4, 5, NULL, 'text only', NULL);
INSERT INTO import_pending (user_id, work_id, source, durations, state)
  VALUES (1, 5, 'legacy', '[1,2]', 'waiting');
INSERT INTO legacy_user_state (user_id, code, favorite_at, has_review, rating, review_text,
                               review_progress, progress_at, position_ms, finished_at,
                               pending_source, pending_durations, pending_state, review_at)
  VALUES (1, 'RJ999999', 3, 1, 4, 'r', 'p', 5, 6, 7, 's', 'd', 'st', 8);
INSERT INTO tasks (id, kind, dedupe_key, state, payload, result, progress, message, created_at,
                   updated_at)
  VALUES (3, 'scan', 'scan:lib', 'done', '{}', '{"ok":true}', 1.0, 'done', 1, 2),
         (8, 'probe', NULL, 'queued', '{"id":1}', NULL, NULL, NULL, 3, 4);
INSERT INTO root_settings (root_id, kind, settings, created_at, updated_at)
  VALUES ('cloud', 'openlist', '{"url":"http://x"}', 1, 2);
INSERT INTO server_settings (id, metadata_proxy, updated_at)
  VALUES (1, 'socks5h://127.0.0.1:1080', 3);
"#;

/// A library database at `path` with the fixture in it.
async fn library(path: &Path) {
    let db = Db::open_sqlite(path).await.unwrap();
    sqlx::raw_sql(FIXTURE)
        .execute(&db.sqlite().unwrap().write)
        .await
        .unwrap();
    db.close().await;
}

/// Every row of every table as SQL literals, in primary key order, so two
/// SQLite files compare equal exactly when they hold the same values.
async fn dump(path: &Path) -> Vec<(String, Vec<String>)> {
    let db = Db::open_sqlite(path).await.unwrap();
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table'
           AND name NOT LIKE 'sqlite%' AND name <> '_sqlx_migrations' ORDER BY name",
    )
    .fetch_all(&db.sqlite().unwrap().read)
    .await
    .unwrap();
    let mut out = Vec::new();
    for table in tables {
        let columns: Vec<(String, i64)> =
            sqlx::query_as("SELECT name, pk FROM pragma_table_info($1) ORDER BY cid")
                .bind(&table)
                .fetch_all(&db.sqlite().unwrap().read)
                .await
                .unwrap();
        let quoted: Vec<String> = columns.iter().map(|(c, _)| format!("quote({c})")).collect();
        let mut key: Vec<&(String, i64)> = columns.iter().filter(|(_, pk)| *pk > 0).collect();
        key.sort_by_key(|(_, pk)| *pk);
        let key: Vec<&str> = key.iter().map(|(c, _)| c.as_str()).collect();
        let rows: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT {} FROM {table} ORDER BY {}",
            quoted.join(" || '|' || "),
            key.join(", ")
        )))
        .fetch_all(&db.sqlite().unwrap().read)
        .await
        .unwrap();
        out.push((table, rows));
    }
    db.close().await;
    out
}

fn sqlite(dir: &Path, name: &str) -> (PathBuf, Endpoint) {
    let path = dir.join(name);
    (path.clone(), Endpoint::Sqlite(path))
}

#[tokio::test]
async fn sqlite_to_sqlite_keeps_every_row() {
    let dir = tempfile::tempdir().unwrap();
    let (from_path, from) = sqlite(dir.path(), "from.sqlite3");
    let (to_path, to) = sqlite(dir.path(), "to.sqlite3");
    library(&from_path).await;

    let report = copy(&from, &to).await.unwrap();

    // A single file the user can move: no WAL was left beside it.
    assert!(!dir.path().join("to.sqlite3-wal").exists());
    let tracks = report.tables.iter().find(|(t, _)| *t == "tracks").unwrap();
    assert_eq!(tracks.1, 3);
    assert_eq!(report.tables.len(), 24);
    let before = dump(&from_path).await;
    assert!(
        before.iter().all(|(_, rows)| !rows.is_empty()),
        "fixture misses a table"
    );
    assert_eq!(dump(&to_path).await, before);
    let db = Db::open_sqlite(&to_path).await.unwrap();
    copied_watermarks_reject_old_reports(&db).await;
    db.close().await;
}

#[tokio::test]
async fn refuses_to_overwrite_an_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let (from_path, from) = sqlite(dir.path(), "from.sqlite3");
    let (to_path, to) = sqlite(dir.path(), "to.sqlite3");
    library(&from_path).await;
    std::fs::write(&to_path, b"keep me").unwrap();

    let err = copy(&from, &to).await.unwrap_err();

    assert!(matches!(err, CopyError::TargetExists(_)), "{err}");
    assert_eq!(std::fs::read(&to_path).unwrap(), b"keep me");
}

#[tokio::test]
async fn refuses_a_source_at_another_schema_version() {
    let dir = tempfile::tempdir().unwrap();
    let (from_path, from) = sqlite(dir.path(), "from.sqlite3");
    let (to_path, to) = sqlite(dir.path(), "to.sqlite3");
    library(&from_path).await;
    let db = Db::open_sqlite(&from_path).await.unwrap();
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version = 14")
        .execute(&db.sqlite().unwrap().write)
        .await
        .unwrap();
    db.close().await;

    let err = copy(&from, &to).await.unwrap_err();

    assert!(
        matches!(
            err,
            CopyError::Version {
                found: Some(13),
                expected: 14,
                ..
            }
        ),
        "{err}"
    );
    assert!(!to_path.exists(), "the target was created before the check");
}

#[tokio::test]
async fn refuses_a_file_that_is_not_a_library() {
    let dir = tempfile::tempdir().unwrap();
    let (from_path, from) = sqlite(dir.path(), "other.sqlite3");
    let (_, to) = sqlite(dir.path(), "to.sqlite3");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&from_path)
        .create_if_missing(true);
    let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
    sqlx::query("CREATE TABLE t (x)")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let err = copy(&from, &to).await.unwrap_err();

    assert!(
        matches!(err, CopyError::Version { found: None, .. }),
        "{err}"
    );
}

#[test]
fn endpoint_tells_urls_from_paths() {
    assert_eq!(
        Endpoint::parse("postgres:///kikoeru?host=/run/postgresql"),
        Endpoint::Postgres("postgres:///kikoeru?host=/run/postgresql".to_owned())
    );
    assert_eq!(
        Endpoint::parse("postgresql://u@h/db"),
        Endpoint::Postgres("postgresql://u@h/db".to_owned())
    );
    assert_eq!(
        Endpoint::parse("data/neo.sqlite3"),
        Endpoint::Sqlite(PathBuf::from("data/neo.sqlite3"))
    );
}

#[tokio::test]
async fn errors_never_show_a_postgres_password() {
    let dir = tempfile::tempdir().unwrap();
    let (_, to) = sqlite(dir.path(), "to.sqlite3");
    let from = Endpoint::Postgres("postgres://kneo:hunter2@127.0.0.1:1/kneo".to_owned());

    let err = copy(&from, &to).await.unwrap_err().to_string();

    assert!(!err.contains("hunter2"), "{err}");
    assert!(err.contains("kneo:***@127.0.0.1"), "{err}");
}

#[test]
fn passwords_in_the_query_string_are_hidden_too() {
    for (url, shown) in [
        (
            "postgres://127.0.0.1/kneo?user=kneo&password=hunter2&sslmode=disable",
            "postgres://127.0.0.1/kneo?user=kneo&password=***&sslmode=disable",
        ),
        // sqlx decodes the key before it looks for a password.
        (
            "postgres://127.0.0.1/kneo?user=kneo&%70assword=hunter2",
            "postgres://127.0.0.1/kneo?user=kneo&password=***",
        ),
        (
            "postgres:///kneo?host=/run/postgresql",
            "postgres:///kneo?host=/run/postgresql",
        ),
        // Not a valid URL, but the parse error would still name it.
        (
            "postgres://u:secret/part@host/kneo",
            "postgres://u:***@host/kneo",
        ),
    ] {
        assert_eq!(Endpoint::Postgres(url.to_owned()).describe(), shown);
    }
}

/// Inserts a row without an ID and returns the one it got.
async fn next_id(path: &Path, sql: &str) -> i64 {
    let db = Db::open_sqlite(path).await.unwrap();
    let id = sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned()))
        .fetch_one(&db.sqlite().unwrap().write)
        .await
        .unwrap();
    db.close().await;
    id
}

const NEW_USER: &str =
    "INSERT INTO users (name, token_version, created_at) VALUES ('new', 0, 0) RETURNING id";
const NEW_TASK: &str = "INSERT INTO tasks (kind, state, payload, created_at, updated_at)
                        VALUES ('scan', 'queued', '{}', 0, 0) RETURNING id";

/// The fixture after its newest user and task were deleted, so the
/// highest IDs ever given out are above every row left.
async fn library_with_deleted_rows(path: &Path) {
    library(path).await;
    let db = Db::open_sqlite(path).await.unwrap();
    sqlx::raw_sql("DELETE FROM users WHERE id = 4; DELETE FROM tasks WHERE id = 8;")
        .execute(&db.sqlite().unwrap().write)
        .await
        .unwrap();
    db.close().await;
}

#[tokio::test]
async fn deleted_ids_are_not_given_out_again() {
    let dir = tempfile::tempdir().unwrap();
    let (from_path, from) = sqlite(dir.path(), "from.sqlite3");
    let (to_path, to) = sqlite(dir.path(), "to.sqlite3");
    library_with_deleted_rows(&from_path).await;

    copy(&from, &to).await.unwrap();

    assert_eq!(next_id(&to_path, NEW_USER).await, 5);
    assert_eq!(next_id(&to_path, NEW_TASK).await, 9);
}

#[cfg(unix)]
#[tokio::test]
async fn a_new_sqlite_copy_is_readable_by_its_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let (from_path, from) = sqlite(dir.path(), "from.sqlite3");
    let (to_path, to) = sqlite(dir.path(), "to.sqlite3");
    library(&from_path).await;

    copy(&from, &to).await.unwrap();

    let mode = std::fs::metadata(&to_path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server in KIKOERU_TEST_PG"]
async fn sqlite_to_postgres_and_back_keeps_every_row() {
    let dir = tempfile::tempdir().unwrap();
    let (from_path, from) = sqlite(dir.path(), "from.sqlite3");
    let (back_path, back) = sqlite(dir.path(), "back.sqlite3");
    library_with_deleted_rows(&from_path).await;
    let (pool, url) = pg::fresh().await;
    let middle = Endpoint::Postgres(url.clone());

    copy(&from, &middle).await.unwrap();
    let db = Db::open(&neo_db::Location::Postgres(url)).await.unwrap();
    copied_watermarks_reject_old_reports(&db).await;
    db.close().await;
    copy(&middle, &back).await.unwrap();
    let db = Db::open_sqlite(&back_path).await.unwrap();
    copied_watermarks_reject_old_reports(&db).await;
    db.close().await;

    assert_eq!(dump(&back_path).await, dump(&from_path).await);
    // Neither copy gives out an ID the source once used, though the rows
    // that had them are gone.
    assert_eq!(next_id(&back_path, NEW_USER).await, 5);
    let user: i64 = sqlx::query_scalar(NEW_USER).fetch_one(&pool).await.unwrap();
    assert_eq!(user, 5);
    let task: i64 = sqlx::query_scalar(NEW_TASK).fetch_one(&pool).await.unwrap();
    assert_eq!(task, 9);
    pool.close().await;
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server in KIKOERU_TEST_PG"]
async fn refuses_a_postgres_database_that_has_rows_or_other_tables() {
    let dir = tempfile::tempdir().unwrap();
    let (from_path, from) = sqlite(dir.path(), "from.sqlite3");
    library(&from_path).await;

    let (other, url) = pg::fresh().await;
    sqlx::query("CREATE TABLE unrelated (x int)")
        .execute(&other)
        .await
        .unwrap();
    let err = copy(&from, &Endpoint::Postgres(url)).await.unwrap_err();
    assert!(matches!(err, CopyError::TargetNotEmpty(_)), "{err}");
    other.close().await;

    let (filled, url) = pg::fresh().await;
    let target = Endpoint::Postgres(url);
    copy(&from, &target).await.unwrap();
    let err = copy(&from, &target).await.unwrap_err();
    assert!(matches!(err, CopyError::TargetNotEmpty(_)), "{err}");
    let works: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM works")
        .fetch_one(&filled)
        .await
        .unwrap();
    assert_eq!(works, 2);
    filled.close().await;
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server in KIKOERU_TEST_PG"]
async fn an_empty_but_migrated_postgres_database_is_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let (from_path, from) = sqlite(dir.path(), "from.sqlite3");
    library(&from_path).await;
    let (pool, url) = pg::fresh().await;
    neo_db::PG_MIGRATOR.run(&pool).await.unwrap();

    copy(&from, &Endpoint::Postgres(url)).await.unwrap();
    pool.close().await;
}

#[tokio::test]
#[ignore = "needs a PostgreSQL server in KIKOERU_TEST_PG"]
async fn a_second_copy_into_the_same_postgres_database_waits_its_turn() {
    let dir = tempfile::tempdir().unwrap();
    let (from_path, from) = sqlite(dir.path(), "from.sqlite3");
    library(&from_path).await;
    let (pool, url) = pg::fresh().await;
    // Another copy that is still writing holds the lock in its transaction.
    let mut other = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(COPY_LOCK)
        .execute(&mut *other)
        .await
        .unwrap();

    let err = copy(&from, &Endpoint::Postgres(url)).await.unwrap_err();

    assert!(matches!(err, CopyError::TargetBusy(_)), "{err}");
    other.rollback().await.unwrap();
    let works: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM works")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(works, 0);
    pool.close().await;
}

async fn copied_watermarks_reject_old_reports(db: &Db) {
    use neo_db::{progress, testing};
    let before = testing::rows::<(Option<i64>, i64, Option<String>, Option<i64>, bool, i64)>(
        db,
        "SELECT track_id, position_ms, group_path, finished_at, replaying, updated_at
             FROM play_progress WHERE user_id = 1 AND work_id = 1",
    )
    .await;
    for sequence in [1, 2] {
        let input = progress::ReportInput {
            track_id: 1,
            position_ms: 1,
            group_path: "stale",
            report_order: Some(progress::ReportOrder {
                session_id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                sequence,
            }),
        };
        progress::report(db, 1, 1, &input, 100).await.unwrap();
        assert_eq!(
            testing::rows::<(Option<i64>, i64, Option<String>, Option<i64>, bool, i64)>(
                db,
                "SELECT track_id, position_ms, group_path, finished_at, replaying, updated_at
                 FROM play_progress WHERE user_id = 1 AND work_id = 1",
            )
            .await,
            before
        );
    }
}
