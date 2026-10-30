//! The store's queries. Most tests run on whichever backend
//! `KIKOERU_TEST_BACKEND` names (see `neo_db::testing`). The ones about
//! SQLite's own files, settings and locking always open SQLite.

use neo_db::{
    Db, OpenError,
    testing::{self, TestDb, exec, on_postgres, value},
    users::{self, CreateError, DeleteOutcome},
};

async fn open() -> (TestDb, Db) {
    let t = testing::open().await;
    let db = t.db.clone();
    (t, db)
}

/// A SQLite library in a directory of its own, for tests about SQLite.
async fn open_sqlite() -> (tempfile::TempDir, Db) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open_sqlite(&dir.path().join("neo.sqlite3"))
        .await
        .unwrap();
    (dir, db)
}

#[tokio::test]
async fn migrates_empty_database_to_full_schema() {
    let (_dir, db) = open_sqlite().await;
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE '\\_%' ESCAPE '\\'
         AND name NOT LIKE 'sqlite%' ORDER BY name",
    )
    .fetch_all(&db.sqlite().unwrap().read)
    .await
    .unwrap();
    assert_eq!(
        tables,
        [
            "circles",
            "favorites",
            "import_pending",
            "legacy_reviews",
            "legacy_user_state",
            "play_progress",
            "progress_report_cursors",
            "reviews",
            "root_settings",
            "roots",
            "server_settings",
            "tag_aliases",
            "tag_merges",
            "tags",
            "tasks",
            "track_analysis",
            "tracks",
            "user_settings",
            "users",
            "vas",
            "work_locations",
            "work_tags",
            "work_vas",
            "works",
        ]
    );
    db.close().await;
}

#[tokio::test]
async fn reviews_are_backfilled_from_imported_legacy_reviews() {
    // A library imported before `reviews` existed: only legacy_reviews has
    // the old ratings and texts.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("neo.sqlite3");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
    neo_db::MIGRATOR.run_to(7, &pool).await.unwrap();
    sqlx::raw_sql(
        "INSERT INTO users (name, is_admin, created_at) VALUES ('u', 0, 0);
         INSERT INTO works (code, title, metadata_src, created_at, updated_at) VALUES
           ('RJ1', 'a', 'none', 0, 0), ('RJ2', 'b', 'none', 0, 0), ('RJ3', 'c', 'none', 0, 0),
           ('RJ4', 'd', 'none', 0, 0), ('RJ5', 'e', 'none', 0, 0);
         INSERT INTO legacy_reviews (user_id, work_id, rating, review_text, progress) VALUES
           (1, 1, 4, ' good ', 'listened'),
           (1, 2, NULL, NULL, 'marked'),
           (1, 3, 9, '   ', NULL),
           (1, 4, 0, 'words', NULL),
           (1, 5, 3, NULL, 'replay');",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let db = Db::open_sqlite(&path).await.unwrap();
    type Row = (i64, Option<i64>, Option<String>, Option<i64>);
    let rows: Vec<Row> =
        sqlx::query_as("SELECT work_id, rating, text, updated_at FROM reviews ORDER BY work_id")
            .fetch_all(&db.sqlite().unwrap().read)
            .await
            .unwrap();
    assert_eq!(
        rows,
        [
            (1, Some(4), Some("good".into()), None),
            (4, None, Some("words".into()), None),
            (5, Some(3), None, None),
        ],
        "only valid ratings and non-blank texts, without a time"
    );
    let archived: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM legacy_reviews")
        .fetch_one(&db.sqlite().unwrap().read)
        .await
        .unwrap();
    assert_eq!(archived, 5, "the archive is untouched");
    db.close().await;
}

#[tokio::test]
async fn the_backfill_trims_and_cuts_like_the_importer() {
    // Every Unicode White_Space character, which Rust's str::trim strips.
    let space: String = (0..=0x3000u32)
        .filter_map(char::from_u32)
        .filter(|c| c.is_whitespace())
        .collect();
    let texts = [
        space.clone(),
        format!("{space}x{space}"),
        format!(
            "{}{}",
            space.repeat(20),
            "a".repeat(neo_core::review::MAX_TEXT_CHARS + 5)
        ),
    ];
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("neo.sqlite3");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
    neo_db::MIGRATOR.run_to(7, &pool).await.unwrap();
    sqlx::raw_sql("INSERT INTO users (name, is_admin, created_at) VALUES ('u', 0, 0)")
        .execute(&pool)
        .await
        .unwrap();
    for (i, text) in texts.iter().enumerate() {
        let id = i64::try_from(i).unwrap() + 1;
        sqlx::query(
            "INSERT INTO works (id, code, title, metadata_src, created_at, updated_at)
             VALUES (?, ?, 't', 'none', 0, 0)",
        )
        .bind(id)
        .bind(format!("RJ{id}"))
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO legacy_reviews (user_id, work_id, review_text) VALUES (1, ?, ?)")
            .bind(id)
            .bind(text)
            .execute(&pool)
            .await
            .unwrap();
    }
    pool.close().await;

    let db = Db::open_sqlite(&path).await.unwrap();
    for (i, text) in texts.iter().enumerate() {
        let id = i64::try_from(i).unwrap() + 1;
        let stored: Option<Option<String>> =
            sqlx::query_scalar("SELECT text FROM reviews WHERE work_id = ?")
                .bind(id)
                .fetch_optional(&db.sqlite().unwrap().read)
                .await
                .unwrap();
        let imported = neo_core::review::from_legacy(None, Some(text))
            .review
            .map(|r| r.text);
        assert_eq!(stored, imported, "work {id}");
    }
    db.close().await;
}

#[tokio::test]
async fn reopening_is_a_no_op() {
    let (t, first) = open().await;
    first.close().await;
    let db = Db::open(&t.location).await.unwrap();
    let applied: i64 = value(&db, "SELECT count(*) FROM _sqlx_migrations").await;
    let migrator = if on_postgres() {
        &neo_db::PG_MIGRATOR
    } else {
        &neo_db::MIGRATOR
    };
    assert_eq!(applied, migrator.iter().count() as i64);
}

#[tokio::test]
async fn connections_use_book_settings() {
    let (_dir, db) = open_sqlite().await;
    for pool in [&db.sqlite().unwrap().read, &db.sqlite().unwrap().write] {
        let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(mode, "wal");
        let fk: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(fk, 1);
        let timeout: i64 = sqlx::query_scalar("PRAGMA busy_timeout")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(timeout, 5000);
    }
    let pools = db.sqlite().unwrap();
    assert_eq!(pools.write.options().get_max_connections(), 1);
    assert_eq!(pools.read.options().get_max_connections(), 4);
    // The read pool must not be able to write.
    assert!(
        sqlx::query("INSERT INTO circles (id, name) VALUES (1, 'x')")
            .execute(&db.sqlite().unwrap().read)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn foreign_keys_are_enforced() {
    let (_dir, db) = open().await;
    let err = neo_db::with_pool!(&db, write, pool => {
        sqlx::query("INSERT INTO favorites (user_id, work_id, created_at) VALUES (1, 1, 0)")
            .execute(pool)
            .await
            .map(drop)
    })
    .unwrap_err();
    let db_err = err.as_database_error().expect("a database error");
    assert!(db_err.is_foreign_key_violation(), "{err}");
}

#[tokio::test]
async fn only_one_active_task_per_dedupe_key() {
    let (_dir, db) = open().await;
    let insert = |state: &'static str| {
        let db = db.clone();
        async move {
            neo_db::with_pool!(&db, write, pool => {
                sqlx::query(
                    "INSERT INTO tasks (kind, dedupe_key, state, payload, created_at, updated_at)
                     VALUES ('scan', 'root:a', $1, '{}', 0, 0)",
                )
                .bind(state)
                .execute(pool)
                .await
                .map(drop)
            })
        }
    };
    insert("done").await.unwrap();
    insert("failed").await.unwrap();
    insert("queued").await.unwrap();
    assert!(insert("running").await.is_err());
    assert!(insert("queued").await.is_err());
}

#[tokio::test]
async fn corrupt_database_refuses_to_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("neo.sqlite3");
    std::fs::write(
        &path,
        b"this is not a sqlite database, just text padding it out",
    )
    .unwrap();
    assert!(Db::open_sqlite(&path).await.is_err());
}

#[tokio::test]
async fn unknown_applied_migration_refuses_to_open() {
    // A database migrated by a newer build must not be used by an older one.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("neo.sqlite3");
    let db = Db::open_sqlite(&path).await.unwrap();
    sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time)
         VALUES (99999999, 'from the future', 1, x'00', 0)",
    )
    .execute(&db.sqlite().unwrap().write)
    .await
    .unwrap();
    db.close().await;
    assert!(matches!(
        Db::open_sqlite(&path).await,
        Err(OpenError::Migrate(_))
    ));
}

#[tokio::test]
async fn user_lifecycle() {
    let (_dir, db) = open().await;
    let w = &db;
    assert!(!users::admin_exists(w).await.unwrap());
    let alice = users::create(w, "alice", "h1", true, 0, 1).await.unwrap();
    assert!(users::admin_exists(w).await.unwrap());
    assert!(matches!(
        users::create(w, "alice", "h2", false, 0, 2).await,
        Err(CreateError::NameTaken(_))
    ));
    let bob = users::create(w, "bob", "h3", false, 0, 3).await.unwrap();

    let before = users::find_by_id(w, alice).await.unwrap().unwrap();
    assert_eq!(before.token_version, 0);
    // PostgreSQL's sequence also moves for the refused insert, so bob's ID
    // is only known to come later.
    assert_eq!(
        users::find_by_name(w, "bob").await.unwrap().unwrap().id,
        bob
    );
    assert!(bob > alice);
    assert!(before.is_admin);
    assert!(users::set_password(w, alice, "h4").await.unwrap());
    let after = users::find_by_name(w, "alice").await.unwrap().unwrap();
    assert_eq!(after.token_version, 1);
    assert_eq!(after.password_hash.as_deref(), Some("h4"));
    assert!(!users::set_password(w, 999, "h").await.unwrap());

    assert_eq!(
        users::delete(w, "alice").await.unwrap(),
        DeleteOutcome::LastAdmin
    );
    assert_eq!(
        users::delete(w, "nobody").await.unwrap(),
        DeleteOutcome::NotFound
    );
    assert_eq!(
        users::delete(w, "bob").await.unwrap(),
        DeleteOutcome::Deleted
    );
    users::create(w, "carol", "h5", true, 0, 4).await.unwrap();
    assert_eq!(
        users::delete(w, "alice").await.unwrap(),
        DeleteOutcome::Deleted
    );
    assert_eq!(
        users::delete(w, "carol").await.unwrap(),
        DeleteOutcome::LastAdmin
    );
}

#[tokio::test]
async fn ids_are_never_reused() {
    // Tokens, favorites, progress and URLs hold these IDs. A deleted row's
    // ID must never come back as a different row.
    let (_dir, db) = open().await;
    let w = &db;
    users::create(w, "admin", "h", true, 0, 0).await.unwrap();
    let bob = users::create(w, "bob", "h", false, 0, 0).await.unwrap();
    assert_eq!(
        users::delete(w, "bob").await.unwrap(),
        DeleteOutcome::Deleted
    );
    let carol = users::create(w, "carol", "h", false, 0, 0).await.unwrap();
    assert!(carol > bob);

    if let Some(sqlite) = db.sqlite() {
        for table in ["works", "work_locations", "tracks", "tasks"] {
            let sql: String = sqlx::query_scalar(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = $1",
            )
            .bind(table)
            .fetch_one(&sqlite.write)
            .await
            .unwrap();
            assert!(sql.contains("AUTOINCREMENT"), "{table}");
        }
    }
    exec(
        &db,
        "INSERT INTO works (code, title, metadata_src, created_at, updated_at)
         VALUES ('RJ1', 'a', 'none', 0, 0), ('RJ2', 'b', 'none', 0, 0);
         DELETE FROM works WHERE code = 'RJ2';
         INSERT INTO works (code, title, metadata_src, created_at, updated_at)
         VALUES ('RJ3', 'c', 'none', 0, 0);",
    )
    .await;
    let id: i64 = value(&db, "SELECT id FROM works WHERE code = 'RJ3'").await;
    assert_eq!(id, 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_deletions_from_independent_pools_keep_one_admin() {
    let (t, first) = open().await;
    let second = Db::open(&t.location).await.unwrap();
    users::create(&first, "alice", "hash", true, 1, 0)
        .await
        .unwrap();
    users::create(&first, "bob", "hash", true, 2, 0)
        .await
        .unwrap();
    let start = std::sync::Arc::new(tokio::sync::Barrier::new(3));
    let mut tasks = tokio::task::JoinSet::new();
    // A CLI process has its own pool, so one shared write connection would
    // hide a check-then-delete race between separate processes.
    for (db, name) in [(first.clone(), "alice"), (second.clone(), "bob")] {
        let start = start.clone();
        tasks.spawn(async move {
            start.wait().await;
            users::delete(&db, name).await.unwrap()
        });
    }
    start.wait().await;
    let mut outcomes = Vec::new();
    while let Some(outcome) =
        tokio::time::timeout(std::time::Duration::from_secs(10), tasks.join_next())
            .await
            .expect("admin deletion timed out")
    {
        outcomes.push(outcome.unwrap());
    }
    assert_eq!(outcomes.len(), 2);
    assert!(outcomes.contains(&DeleteOutcome::Deleted));
    assert!(outcomes.contains(&DeleteOutcome::LastAdmin));
    let alice = users::find_by_name(&first, "alice").await.unwrap();
    let bob = users::find_by_name(&second, "bob").await.unwrap();
    assert_ne!(alice.is_some(), bob.is_some());
    assert!(users::admin_exists(&first).await.unwrap());
    first.close().await;
    second.close().await;
}

#[tokio::test]
async fn roots_follow_the_config_without_dropping_old_ones() {
    let (_dir, db) = open().await;
    neo_db::roots::sync(&db, &[("a", "A"), ("b", "B")])
        .await
        .unwrap();
    neo_db::roots::sync(&db, &[("a", "A renamed")])
        .await
        .unwrap();
    let rows: Vec<(String, String)> =
        testing::rows(&db, "SELECT id, display_name FROM roots ORDER BY id").await;
    assert_eq!(
        rows,
        [("a".into(), "A renamed".into()), ("b".into(), "B".into())]
    );
}

#[tokio::test]
async fn queue_limits_count_only_the_same_kind() {
    use neo_db::tasks::{Enqueued, enqueue};
    let (_dir, db) = open().await;
    for i in 0..3 {
        let key = format!("probe:{i}");
        let queued = enqueue(&db, "probe", Some(&key), "{}", None, 0)
            .await
            .unwrap();
        assert!(matches!(queued, Enqueued::Created(_)));
    }
    // Three queued probes do not fill a transcode queue of two.
    let first = enqueue(&db, "transcode", None, "{}", Some(2), 0)
        .await
        .unwrap();
    assert!(matches!(first, Enqueued::Created(_)), "{first:?}");
    enqueue(&db, "transcode", None, "{}", Some(2), 0)
        .await
        .unwrap();
    let third = enqueue(&db, "transcode", None, "{}", Some(2), 0)
        .await
        .unwrap();
    assert!(matches!(third, Enqueued::QueueFull), "{third:?}");
}

#[tokio::test]
async fn tags_resolve_by_alias_then_genre_id_and_merge() {
    use neo_db::tags::{self, CUSTOM_TAG_BASE, MergeOutcome};
    let (_dir, db) = open().await;
    exec(
        &db,
        "INSERT INTO tags (id, name) VALUES (5, 'School'), (7, 'ASMR');
         INSERT INTO tag_aliases (alias, tag_id) VALUES ('School', 5), ('Academy', 5), ('ASMR', 7);",
    )
    .await;

    let mut tx = db.begin().await.unwrap();
    assert_eq!(tags::resolve(&mut tx, "Academy", None).await.unwrap(), 5);
    // A known genre under a new spelling keeps its name and learns the alias.
    assert_eq!(tags::resolve(&mut tx, "学校", Some(5)).await.unwrap(), 5);
    assert_eq!(tags::resolve(&mut tx, "学校", None).await.unwrap(), 5);
    assert_eq!(tags::resolve(&mut tx, "Whisper", Some(9)).await.unwrap(), 9);
    assert_eq!(tags::resolve(&mut tx, "Whisper", None).await.unwrap(), 9);
    let custom = tags::resolve(&mut tx, "hand made", None).await.unwrap();
    assert_eq!(custom, CUSTOM_TAG_BASE);
    assert_eq!(
        tags::resolve(&mut tx, "another", None).await.unwrap(),
        CUSTOM_TAG_BASE + 1
    );
    tx.commit().await.unwrap();
    let name: String = value(&db, "SELECT name FROM tags WHERE id = 5").await;
    assert_eq!(name, "School");

    exec(
        &db,
        "INSERT INTO works (code, title, metadata_src, created_at, updated_at)
           VALUES ('RJ000001', 't', 'none', 0, 0), ('RJ000002', 't', 'none', 0, 0);
         INSERT INTO work_tags VALUES (1, 9), (2, 9), (2, 7);",
    )
    .await;
    assert_eq!(
        tags::merge(&db, 9, 9, 1).await.unwrap(),
        MergeOutcome::SameTag
    );
    assert_eq!(
        tags::merge(&db, 9, 99, 1).await.unwrap(),
        MergeOutcome::NotFound
    );
    assert_eq!(
        tags::merge(&db, 9, 7, 1).await.unwrap(),
        MergeOutcome::Merged
    );
    let links: Vec<(i64, i64)> =
        testing::rows(&db, "SELECT work_id, tag_id FROM work_tags ORDER BY 1, 2").await;
    assert_eq!(links, [(1, 7), (2, 7)]);
    let mut tx = db.begin().await.unwrap();
    assert_eq!(tags::resolve(&mut tx, "Whisper", None).await.unwrap(), 7);
    tx.commit().await.unwrap();
}

#[tokio::test]
async fn merges_keep_other_tags_aliases_and_survive_resolve() {
    use neo_db::tags;
    let (_dir, db) = open().await;
    // 31 and 438 share a canonical name; 31 got the alias first.
    exec(
        &db,
        "INSERT INTO tags (id, name) VALUES (1, 'Other'), (31, 'Shared'), (438, 'Shared');
         INSERT INTO tag_aliases (alias, tag_id)
           VALUES ('Other', 1), ('Shared', 31), ('Only 438', 438);",
    )
    .await;
    tags::merge(&db, 438, 1, 1).await.unwrap();
    let owner: i64 = value(&db, "SELECT tag_id FROM tag_aliases WHERE alias = 'Shared'").await;
    assert_eq!(owner, 31, "a merge does not take a third tag's alias");

    // A scrape naming the merged genre by ID lands on the surviving tag.
    let mut tx = db.begin().await.unwrap();
    assert_eq!(
        tags::resolve(&mut tx, "New spelling", Some(438))
            .await
            .unwrap(),
        1
    );
    tx.commit().await.unwrap();
    let recreated: i64 = value(&db, "SELECT COUNT(*) FROM tags WHERE id = 438").await;
    assert_eq!(recreated, 0);
}

#[tokio::test]
async fn merge_records_stay_correct_over_time() {
    use neo_db::tags::{self, CUSTOM_TAG_BASE};
    let (_dir, db) = open().await;
    exec(
        &db,
        "INSERT INTO tags (id, name) VALUES (7, 'Seven'), (9, 'Nine'), (10, 'Ten');",
    )
    .await;
    let resolve = |name: &'static str, id: Option<i64>| {
        let db = db.clone();
        async move {
            let mut tx = db.begin().await.unwrap();
            let got = tags::resolve(&mut tx, name, id).await.unwrap();
            tx.commit().await.unwrap();
            got
        }
    };
    let custom = resolve("custom", None).await;
    assert_eq!(custom, CUSTOM_TAG_BASE);
    tags::merge(&db, 7, custom, 1).await.unwrap();
    tags::merge(&db, custom, 9, 2).await.unwrap();
    // A new custom tag does not reuse the merged-away custom ID.
    let next = resolve("another", None).await;
    assert_eq!(next, CUSTOM_TAG_BASE + 1);
    tags::merge(&db, next, 10, 3).await.unwrap();
    assert_eq!(resolve("unseen spelling", Some(7)).await, 9);

    // Long chains are followed to the end.
    let (_dir2, db2) = open().await;
    let rows: Vec<String> = (1..=40).map(|i| format!("({i}, 't{i}')")).collect();
    exec(
        &db2,
        &format!("INSERT INTO tags (id, name) VALUES {};", rows.join(", ")),
    )
    .await;
    for i in 1..40 {
        tags::merge(&db2, i, i + 1, i).await.unwrap();
    }
    let mut tx = db2.begin().await.unwrap();
    assert_eq!(tags::resolve(&mut tx, "fresh", Some(1)).await.unwrap(), 40);
    tx.commit().await.unwrap();
}

/// One attempt of a transaction that reads, lets another process write if
/// `interfere` says so, and then writes. The other process's commit makes
/// the write fail at once with SQLITE_BUSY_SNAPSHOT, which `busy_timeout`
/// does not wait out.
async fn read_then_write(
    db: &Db,
    other: &tokio::sync::Mutex<sqlx::SqliteConnection>,
    attempt: u32,
    interfere: bool,
) -> Result<(), sqlx::Error> {
    let mut tx = db.sqlite().unwrap().write.begin().await?;
    sqlx::query("SELECT count(*) FROM users")
        .fetch_one(&mut *tx)
        .await?;
    if interfere {
        sqlx::query("INSERT INTO users (name, created_at) VALUES (?, 0)")
            .bind(format!("other {attempt}"))
            .execute(&mut *other.lock().await)
            .await?;
    }
    sqlx::query("INSERT INTO users (name, created_at) VALUES ('mine', 0)")
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

async fn second_connection(dir: &tempfile::TempDir) -> tokio::sync::Mutex<sqlx::SqliteConnection> {
    use sqlx::Connection;
    let url = format!("sqlite:{}", dir.path().join("neo.sqlite3").display());
    tokio::sync::Mutex::new(sqlx::SqliteConnection::connect(&url).await.unwrap())
}

#[tokio::test]
async fn a_busy_transaction_is_run_again() {
    use std::sync::atomic::{AtomicU32, Ordering};
    let (dir, db) = open_sqlite().await;
    let other = second_connection(&dir).await;
    let attempts = AtomicU32::new(0);
    neo_db::retry_busy(|| {
        let attempt = attempts.fetch_add(1, Ordering::SeqCst) + 1;
        read_then_write(&db, &other, attempt, attempt == 1)
    })
    .await
    .unwrap();
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    let names: Vec<String> = testing::column(&db, "SELECT name FROM users ORDER BY name").await;
    assert_eq!(names, ["mine", "other 1"]);
}

#[tokio::test]
async fn review_writes_survive_another_process_writing() {
    use neo_core::review::Valid;
    use neo_db::reviews;
    let (dir, db) = open_sqlite().await;
    let other = second_connection(&dir).await;
    exec(
        &db,
        "INSERT INTO users (id, name, created_at) VALUES (1, 'u', 0);
         INSERT INTO works (id, code, title, metadata_src, created_at, updated_at)
           VALUES (1, 'RJ1', 't', 'none', 0, 0);",
    )
    .await;
    let write = &db.sqlite().unwrap().write;
    // The other process holds the write lock when the review reads and
    // writes. A read transaction that then writes gets SQLITE_BUSY at once,
    // without waiting for busy_timeout, so the first attempt always fails.
    async fn interfere(other: &tokio::sync::Mutex<sqlx::SqliteConnection>, name: &str) {
        let mut conn = other.lock().await;
        sqlx::raw_sql("BEGIN IMMEDIATE")
            .execute(&mut *conn)
            .await
            .unwrap();
        sqlx::query("INSERT INTO users (name, created_at) VALUES (?, 0)")
            .bind(name)
            .execute(&mut *conn)
            .await
            .unwrap();
        drop(conn);
    }
    // Commits once the first attempt has been turned away, and holds the
    // only write connection meanwhile. The next attempt cannot begin until
    // the lock is gone, so no timing decides whether the retries run out.
    async fn commit_after_first_attempt(
        write: &sqlx::SqlitePool,
        other: &tokio::sync::Mutex<sqlx::SqliteConnection>,
    ) {
        while write.num_idle() > 0 {
            tokio::task::yield_now().await;
        }
        let _held = write.acquire().await.unwrap();
        sqlx::raw_sql("COMMIT")
            .execute(&mut *other.lock().await)
            .await
            .unwrap();
    }
    // The connection the fixtures used goes back to the pool in a task of
    // its own. Until it is idle, taking it would look like the first attempt.
    async fn wait_until_idle(write: &sqlx::SqlitePool) {
        while write.num_idle() == 0 {
            tokio::task::yield_now().await;
        }
    }
    let valid = Valid {
        rating: Some(4),
        text: None,
    };
    wait_until_idle(write).await;
    interfere(&other, "a").await;
    let (saved, ()) = tokio::join!(
        reviews::put(&db, 1, 1, &valid, 7),
        commit_after_first_attempt(write, &other)
    );
    assert_eq!(saved.unwrap().unwrap().rating, Some(4));

    wait_until_idle(write).await;
    interfere(&other, "b").await;
    let (deleted, ()) = tokio::join!(
        reviews::delete(&db, 1, 1),
        commit_after_first_attempt(write, &other)
    );
    assert!(deleted.unwrap());
    assert_eq!(reviews::get(&db, 1, 1).await.unwrap(), None);
}

#[tokio::test]
async fn busy_is_retried_three_times_then_returned() {
    use std::sync::atomic::{AtomicU32, Ordering};
    let (dir, db) = open_sqlite().await;
    let other = second_connection(&dir).await;
    let attempts = AtomicU32::new(0);
    let error = neo_db::retry_busy(|| {
        let attempt = attempts.fetch_add(1, Ordering::SeqCst) + 1;
        read_then_write(&db, &other, attempt, true)
    })
    .await
    .unwrap_err();
    assert!(neo_db::is_busy(&error), "{error}");
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        4,
        "one attempt and three retries"
    );
}

#[tokio::test]
async fn other_errors_are_not_retried() {
    use std::sync::atomic::{AtomicU32, Ordering};
    let (_dir, db) = open().await;
    let attempts = AtomicU32::new(0);
    let insert = || {
        attempts.fetch_add(1, Ordering::SeqCst);
        let db = db.clone();
        async move {
            neo_db::with_pool!(&db, write, pool => {
                sqlx::query("INSERT INTO users (name, created_at) VALUES ('same', 0)")
                    .execute(pool)
                    .await
                    .map(drop)
            })
        }
    };
    neo_db::retry_busy(insert).await.unwrap();
    let error = neo_db::retry_busy(insert).await.unwrap_err();
    assert!(!neo_db::is_busy(&error));
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn pruning_drops_old_and_excess_finished_tasks_only() {
    use neo_db::tasks;
    let (_dir, db) = open().await;
    let insert = |state: &'static str, updated_at: i64| {
        let db = db.clone();
        async move {
            value::<i64>(
                &db,
                &format!(
                    "INSERT INTO tasks (kind, state, payload, created_at, updated_at)
                     VALUES ('transcode', '{state}', '{{}}', 0, {updated_at}) RETURNING id"
                ),
            )
            .await
        }
    };
    let old_done = insert("done", 100).await;
    let old_running = insert("running", 100).await;
    let old_queued = insert("queued", 100).await;
    let recent: Vec<i64> = [
        insert("failed", 1000).await,
        insert("cancelled", 1001).await,
        insert("interrupted", 1002).await,
        insert("done", 1003).await,
    ]
    .to_vec();

    // Older than 500 goes; of the rest, the newest two finished stay.
    let removed = tasks::prune(&db, 500, 2).await.unwrap();
    assert_eq!(removed, 3);
    let left: Vec<i64> = testing::column(&db, "SELECT id FROM tasks ORDER BY id").await;
    assert_eq!(left, [old_running, old_queued, recent[2], recent[3]]);
    assert!(!left.contains(&old_done));
}

#[tokio::test]
async fn a_retried_start_after_a_lost_answer_still_starts() {
    use neo_db::tasks;
    let (_dir, db) = open().await;
    let id = value::<i64>(
        &db,
        "INSERT INTO tasks (kind, state, payload, created_at, updated_at)
         VALUES ('scan', 'queued', '{}', 0, 0) RETURNING id",
    )
    .await;
    assert!(tasks::start(&db, id, 1).await.unwrap());
    // The first write committed, but its answer never reached the runner.
    assert!(tasks::start(&db, id, 2).await.unwrap());
    let ended = tasks::finish(&db, id, tasks::EndState::Cancelled, None, None, 3);
    assert!(ended.await.unwrap());
    assert!(
        !tasks::start(&db, id, 4).await.unwrap(),
        "an ended task stays ended"
    );
}

#[tokio::test]
async fn only_a_queued_task_is_cancelled_without_its_runner() {
    use neo_db::tasks;
    let (_dir, db) = open().await;
    let insert = |state: &'static str| {
        let db = db.clone();
        async move {
            value::<i64>(
                &db,
                &format!(
                    "INSERT INTO tasks (kind, state, payload, created_at, updated_at)
                     VALUES ('scan', '{state}', '{{}}', 0, 0) RETURNING id"
                ),
            )
            .await
        }
    };
    let queued = insert("queued").await;
    let running = insert("running").await;
    let done = insert("done").await;
    assert!(tasks::cancel_queued(&db, queued, 5).await.unwrap());
    assert!(!tasks::cancel_queued(&db, running, 5).await.unwrap());
    assert!(!tasks::cancel_queued(&db, done, 5).await.unwrap());
    let states: Vec<String> = testing::column(&db, "SELECT state FROM tasks ORDER BY id").await;
    assert_eq!(states, ["cancelled", "running", "done"]);
}

#[tokio::test]
async fn finds_roots_with_works_that_nothing_defines() {
    let (_dir, db) = open().await;
    exec(
        &db,
        "INSERT INTO roots (id, display_name) VALUES ('kept', 'k'), ('gone', 'g'), ('empty', 'e');
         INSERT INTO works (code, title, metadata_src, created_at, updated_at)
           VALUES ('RJ1', 'a', 'none', 0, 0), ('RJ2', 'b', 'none', 0, 0), ('RJ3', 'c', 'none', 0, 0);
         INSERT INTO work_locations (work_id, root_id, rel_path, display_path, missing_at) VALUES
           (1, 'kept', 'a', 'a', NULL), (2, 'gone', 'b', 'b', NULL),
           (3, 'gone', 'c', 'c', NULL), (3, 'empty', 'c', 'c', 5);",
    )
    .await;
    let orphans = neo_db::roots::with_works_outside(&db, &["kept"])
        .await
        .unwrap();
    // A root whose only location is missing has no works to warn about.
    assert_eq!(orphans, [("gone".to_owned(), 2)]);
}

#[tokio::test]
async fn a_measurement_without_an_mtime_expires_after_a_week() {
    use neo_db::analysis::{self, StoredLoudness, UNKNOWN_MTIME_TTL_MS};
    let (_dir, db) = open().await;
    exec(
        &db,
        "INSERT INTO roots (id, display_name) VALUES ('r', 'R');
         INSERT INTO works (code, title, metadata_src, created_at, updated_at)
           VALUES ('RJ000001', 'W', 'scan', 0, 0);
         INSERT INTO work_locations (work_id, root_id, rel_path, display_path)
           VALUES (1, 'r', CAST('w' AS BLOB), 'w');
         INSERT INTO tracks (work_id, location_id, rel_path, display_path, sort_key, kind, size, mtime)
           VALUES (1, 1, CAST('a.mp3' AS BLOB), 'a.mp3', 'a', 'audio', 10, NULL),
                  (1, 1, CAST('b.mp3' AS BLOB), 'b.mp3', 'b', 'audio', 10, 5);",
    )
    .await;
    let measured = StoredLoudness {
        integrated_lufs: Some(-20.0),
        true_peak_dbtp: Some(-3.0),
    };
    let t0 = 1_000_000_000_000;
    let first = neo_db::media::track_file(&db, 1).await.unwrap().unwrap();
    let second = neo_db::media::track_file(&db, 2).await.unwrap().unwrap();
    analysis::save(&db, &first, Some(10), None, measured, t0)
        .await
        .unwrap();
    analysis::save(&db, &second, Some(10), Some(5), measured, t0)
        .await
        .unwrap();
    let later = t0 + UNKNOWN_MTIME_TTL_MS + 1;
    let get = |id, mtime, now| analysis::loudness(&db, id, Some(10), mtime, now);
    assert_eq!(get(1, None, t0 + 1).await.unwrap(), Some(measured));
    assert_eq!(get(1, None, later).await.unwrap(), None);
    assert_eq!(get(2, Some(5), later).await.unwrap(), Some(measured));
    assert_eq!(get(2, Some(6), t0).await.unwrap(), None, "a changed mtime");
}

#[test]
fn location_debug_hides_the_password() {
    let location = neo_db::Location::Postgres("postgres://k:hunter2@db/kikoeru".into());
    let shown = format!("{location:?}");
    assert!(!shown.contains("hunter2"), "{shown}");
    assert!(shown.contains("db/kikoeru"), "{shown}");
}

#[tokio::test]
async fn a_read_transaction_sees_one_snapshot() {
    let (_t, db) = open().await;
    exec(
        &db,
        "INSERT INTO users (name, is_admin, created_at) VALUES ('a', FALSE, 0)",
    )
    .await;
    neo_db::with_pool!(&db, read, pool => {
        let mut tx = pool.begin().await.unwrap();
        let count = "SELECT COUNT(*) FROM users";
        let before: i64 = sqlx::query_scalar(count).fetch_one(&mut *tx).await.unwrap();
        exec(
            &db,
            "INSERT INTO users (name, is_admin, created_at) VALUES ('b', FALSE, 0)",
        )
        .await;
        let after: i64 = sqlx::query_scalar(count).fetch_one(&mut *tx).await.unwrap();
        assert_eq!((before, after), (1, 1), "a commit in between was seen");
    });
}

/// `{:#}` prints the whole source chain, so the cause must appear in it
/// once, not once per level.
#[tokio::test]
async fn an_open_error_names_its_cause_once() {
    let dir = tempfile::tempdir().unwrap();
    let err = Db::open_sqlite(&dir.path().join("missing/neo.sqlite3"))
        .await
        .unwrap_err();
    let mut chain = err.to_string();
    let mut source = std::error::Error::source(&err);
    while let Some(e) = source {
        chain = format!("{chain}: {e}");
        source = e.source();
    }
    assert_eq!(
        chain.matches("unable to open database file").count(),
        1,
        "{chain}"
    );
}

/// Search folds ASCII case and nothing else, as SQLite's lower() does. On
/// a PostgreSQL database whose default collation folds more, the search
/// text must still be folded like the C-collated columns, or a title
/// searched exactly as written is missed.
#[tokio::test]
async fn search_folds_only_ascii_case_on_either_backend() {
    use neo_db::works::{self, Filter, ListQuery, Order, Scope, Sort};
    let (_t, db) = open().await;
    exec(
        &db,
        "INSERT INTO works (code, title, metadata_src, created_at, updated_at)
           VALUES ('RJ01', 'ＡＳＭＲ Voice', 'none', 0, 0)",
    )
    .await;
    let search = |q: &str| {
        let query = ListQuery {
            user_id: 1,
            scope: Scope::Library,
            sort: Sort::Added,
            order: Order::Desc,
            filter: Filter {
                q: Some(q.to_owned()),
                ..Filter::default()
            },
            limit: 10,
            offset: 0,
        };
        let db = db.clone();
        async move {
            works::list(&db, &query)
                .await
                .unwrap()
                .into_iter()
                .map(|w| w.code)
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(search("ＡＳＭＲ").await, ["RJ01"], "the title as written");
    assert_eq!(search("VOICE").await, ["RJ01"]);
    assert_eq!(search("rj01").await, ["RJ01"]);
    assert!(
        search("ａｓｍｒ").await.is_empty(),
        "only ASCII case is folded"
    );
}

#[tokio::test]
async fn detail_keeps_resume_progress_with_its_tracks_across_deduplication() {
    use neo_core::dto::{Id, SafeInteger, TrackKind};
    use neo_db::{detail, progress, scan};
    use std::{sync::Arc, time::Duration};
    use tokio::sync::Barrier;

    let (_t, db) = open().await;
    exec(
        &db,
        "INSERT INTO users (id, name, is_admin, created_at) VALUES (1, 'u', FALSE, 0);
         INSERT INTO roots (id, display_name) VALUES ('remote', 'Remote'), ('local', 'Local');
         INSERT INTO works (id, code, title, metadata_src, created_at, updated_at)
           VALUES (1, 'RJ1', 'Synthetic work', 'none', 0, 0);
         INSERT INTO work_locations (id, work_id, root_id, rel_path, display_path)
           VALUES (1, 1, 'remote', CAST('RJ1' AS BLOB), 'RJ1'),
                  (2, 1, 'local', CAST('RJ1' AS BLOB), 'RJ1');
         INSERT INTO tracks (id, work_id, location_id, rel_path, display_path, sort_key, kind, size)
           VALUES (10, 1, 1, CAST('audio/01.mp3' AS BLOB), 'audio/01.mp3', 'audio/01.mp3', 'audio', 1000),
                  (20, 1, 2, CAST('audio/01.mp3' AS BLOB), 'audio/01.mp3', 'audio/01.mp3', 'audio', 1000);",
    )
    .await;
    progress::report(
        &db,
        1,
        1,
        &progress::ReportInput {
            track_id: 10,
            position_ms: 600_000,
            group_path: "audio",
            report_order: None,
        },
        1,
    )
    .await
    .unwrap();

    let barrier = Arc::new(Barrier::new(2));
    let hook = match &db {
        Db::Sqlite(_) => &neo_db_sqlite::detail::AFTER_SUMMARY,
        Db::Postgres(_) => &neo_db_postgres::detail::AFTER_SUMMARY,
    };
    let read = hook.scope(barrier.clone(), detail::get(&db, 1, 1, true));
    let switch_copy = async {
        barrier.wait().await;
        // Use the scanner's writes and commit while detail holds the old summary.
        let mut tx = db.begin().await.unwrap();
        scan::set_duplicate_of(&mut tx, 10, Some(20)).await.unwrap();
        scan::move_progress(&mut tx, 10, 20).await.unwrap();
        tx.commit().await.unwrap();
        barrier.wait().await;
    };
    let (detail, ()) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(read, switch_copy)
    })
    .await
    .expect("the scan must commit while detail is paused");
    let detail = detail.unwrap().unwrap();
    let progress = detail.summary.progress.as_ref().unwrap();
    assert!(progress.resumable);
    assert_eq!(progress.track_id, Some(Id::try_from(10).unwrap()));
    assert_eq!(
        progress.position_ms,
        SafeInteger::try_from(600_000).unwrap()
    );
    assert_eq!(progress.group_path.as_deref(), Some("audio"));
    assert!(
        detail.tracks.iter().any(|track| {
            Some(track.id) == progress.track_id && !track.missing && track.kind == TrackKind::Audio
        }),
        "resumable progress must name a playable track in the same detail: {detail:?}"
    );

    let after = detail::get(&db, 1, 1, true).await.unwrap().unwrap();
    assert_eq!(after.tracks.len(), 1);
    assert_eq!(after.tracks[0].id, Id::try_from(20).unwrap());
    let progress = after.summary.progress.unwrap();
    assert!(progress.resumable);
    assert_eq!(progress.track_id, Some(after.tracks[0].id));
    assert_eq!(
        progress.position_ms,
        SafeInteger::try_from(600_000).unwrap()
    );
    db.close().await;
}

/// Progress reported on a copy a scan has hidden is stored on the copy it
/// kept, as the scan itself would move it, so a player holding the old
/// queue cannot point the progress at a track the detail page leaves out.
#[tokio::test]
async fn progress_on_a_hidden_duplicate_is_stored_on_the_kept_copy() {
    use neo_db::progress;
    let (_t, db) = open().await;
    exec(
        &db,
        "INSERT INTO users (id, name, is_admin, created_at) VALUES (1, 'u', FALSE, 0);
         INSERT INTO roots (id, display_name) VALUES ('a', 'A'), ('b', 'B');
         INSERT INTO works (id, code, title, metadata_src, created_at, updated_at)
           VALUES (1, 'RJ1', 't', 'none', 0, 0);
         INSERT INTO work_locations (id, work_id, root_id, rel_path, display_path)
           VALUES (1, 1, 'a', CAST('RJ1' AS BLOB), 'RJ1'), (2, 1, 'b', CAST('RJ1' AS BLOB), 'RJ1');
         INSERT INTO tracks (id, work_id, location_id, rel_path, display_path, sort_key, kind, size, duplicate_of)
           VALUES (10, 1, 1, CAST('x.mp3' AS BLOB), 'x.mp3', 'x.mp3', 'audio', 1, NULL),
                  (20, 1, 2, CAST('x.mp3' AS BLOB), 'x.mp3', 'x.mp3', 'audio', 1, 10);",
    )
    .await;
    let stored = |db: Db| async move {
        testing::rows::<(i64, i64, bool)>(
            &db,
            "SELECT track_id, position_ms, finished_at IS NOT NULL FROM play_progress",
        )
        .await
    };
    progress::report(
        &db,
        1,
        1,
        &progress::ReportInput {
            track_id: 20,
            position_ms: 5000,
            group_path: "",
            report_order: None,
        },
        7,
    )
    .await
    .unwrap();
    assert_eq!(stored(db.clone()).await, [(10, 5000, false)]);
    for track in [10, 20] {
        assert!(
            progress::ends_group(&db, 1, "", track).await.unwrap(),
            "{track}"
        );
    }
    assert!(!progress::ends_group(&db, 1, "sub", 20).await.unwrap());
    progress::finish(&db, 1, 1, 20, "", 8).await.unwrap();
    assert_eq!(
        stored(db.clone()).await,
        [(10, 5000, true)],
        "finishing on the same track keeps the position"
    );

    // During a scan the link can be stale: the kept file changed size, or
    // went missing, and the duplicates are not worked out again yet. The
    // copy the player used is then no longer the same file, so it keeps the
    // progress, and so does a finish.
    for change in [
        "UPDATE tracks SET size = 2 WHERE id = 10",
        "UPDATE tracks SET size = 1, missing_at = 1 WHERE id = 10",
    ] {
        exec(&db, change).await;
        assert!(
            !progress::ends_group(&db, 1, "", 20).await.unwrap(),
            "{change}"
        );
        progress::report(
            &db,
            1,
            1,
            &progress::ReportInput {
                track_id: 20,
                position_ms: 6000,
                group_path: "",
                report_order: None,
            },
            9,
        )
        .await
        .unwrap();
        assert_eq!(stored(db.clone()).await, [(20, 6000, true)], "{change}");
        progress::finish(&db, 1, 1, 20, "", 10).await.unwrap();
        assert_eq!(stored(db.clone()).await, [(20, 6000, true)], "{change}");
        exec(&db, "UPDATE play_progress SET track_id = 10").await;
    }
}
