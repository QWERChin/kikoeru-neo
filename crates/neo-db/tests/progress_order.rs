use neo_db::{
    Db,
    progress::{self, ReportInput, ReportOrder},
    testing::{self, TestDb, exec, rows},
};

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

const FIXTURE: &str = "
    INSERT INTO users (id, name, created_at) VALUES (1, 'alice', 0), (2, 'bob', 0);
    INSERT INTO roots (id, display_name) VALUES ('local', 'Local');
    INSERT INTO works (id, code, title, metadata_src, created_at, updated_at) VALUES
        (1, 'RJ1', 'Work one', 'none', 0, 0), (2, 'RJ2', 'Work two', 'none', 0, 0);
    INSERT INTO work_locations (id, work_id, root_id, rel_path, display_path) VALUES
        (1, 1, 'local', CAST('RJ1' AS BLOB), 'RJ1'),
        (2, 2, 'local', CAST('RJ2' AS BLOB), 'RJ2');
    INSERT INTO tracks (id, work_id, location_id, rel_path, display_path, sort_key, kind) VALUES
        (11, 1, 1, CAST('one.mp3' AS BLOB), 'one.mp3', 'one.mp3', 'audio'),
        (12, 1, 1, CAST('two.mp3' AS BLOB), 'two.mp3', 'two.mp3', 'audio'),
        (21, 2, 2, CAST('one.mp3' AS BLOB), 'one.mp3', 'one.mp3', 'audio');";

async fn library() -> TestDb {
    let t = testing::open().await;
    exec(&t.db, FIXTURE).await;
    t
}

fn report(session_id: &str, sequence: i64, position_ms: i64) -> ReportInput<'_> {
    ReportInput {
        track_id: 11,
        position_ms,
        group_path: "",
        report_order: Some(ReportOrder {
            session_id,
            sequence,
        }),
    }
}

// Include the storage-only fields, not just the derived API flags.
type Progress = (
    i64,
    i64,
    Option<i64>,
    i64,
    Option<String>,
    Option<i64>,
    bool,
    i64,
);

async fn stored(db: &Db) -> Vec<Progress> {
    rows(
        db,
        "SELECT user_id, work_id, track_id, position_ms, group_path, finished_at,
                     replaying, updated_at FROM play_progress ORDER BY user_id, work_id",
    )
    .await
}

async fn cursors(db: &Db) -> Vec<(i64, i64, String, i64)> {
    rows(
        db,
        "SELECT user_id, work_id, session_id, last_sequence
              FROM progress_report_cursors ORDER BY user_id, work_id, session_id",
    )
    .await
}

#[tokio::test]
async fn stale_and_duplicate_reports_leave_the_whole_progress_unchanged() {
    let t = library().await;
    let db = &t.db;
    progress::report(db, 1, 1, &report(A, 2, 110_000), 20)
        .await
        .unwrap();
    let expected = vec![(
        1,
        1,
        Some(11),
        110_000,
        Some(String::new()),
        None,
        false,
        20,
    )];
    assert_eq!(stored(db).await, expected);
    for sequence in [1, 2] {
        let input = ReportInput {
            track_id: 12,
            group_path: "other",
            ..report(A, sequence, 100_000)
        };
        progress::report(db, 1, 1, &input, 30).await.unwrap();
        assert_eq!(stored(db).await, expected);
    }
}

#[tokio::test]
async fn later_sequences_accept_backwards_seeks_and_gaps() {
    let t = library().await;
    progress::report(&t.db, 1, 1, &report(A, 1, 110_000), 10)
        .await
        .unwrap();
    progress::report(&t.db, 1, 1, &report(A, 9_007_199_254_740_991, 90_000), 20)
        .await
        .unwrap();
    assert_eq!(
        stored(&t.db).await,
        [(1, 1, Some(11), 90_000, Some(String::new()), None, false, 20)]
    );
}

#[tokio::test]
async fn another_session_does_not_erase_the_first_sessions_watermark() {
    let t = library().await;
    progress::report(&t.db, 1, 1, &report(A, 2, 110_000), 10)
        .await
        .unwrap();
    let b = ReportInput {
        track_id: 12,
        group_path: "b",
        ..report(B, 1, 90_000)
    };
    progress::report(&t.db, 1, 1, &b, 20).await.unwrap();
    let expected = stored(&t.db).await;
    for sequence in [1, 2] {
        progress::report(&t.db, 1, 1, &report(A, sequence, 100_000), 30)
            .await
            .unwrap();
        assert_eq!(stored(&t.db).await, expected);
    }
}

#[tokio::test]
async fn legacy_writes_do_not_erase_watermarks() {
    let t = library().await;
    progress::report(&t.db, 1, 1, &report(A, 2, 110_000), 10)
        .await
        .unwrap();
    let legacy = ReportInput {
        report_order: None,
        ..report(A, 1, 90_000)
    };
    progress::report(&t.db, 1, 1, &legacy, 20).await.unwrap();
    let expected = stored(&t.db).await;
    progress::report(&t.db, 1, 1, &report(A, 1, 100_000), 30)
        .await
        .unwrap();
    assert_eq!(stored(&t.db).await, expected);
    assert_eq!(cursors(&t.db).await, [(1, 1, A.into(), 2)]);
}

#[tokio::test]
async fn clear_preserves_watermarks_but_allows_a_new_report() {
    let t = library().await;
    progress::report(&t.db, 1, 1, &report(A, 2, 110_000), 10)
        .await
        .unwrap();
    progress::clear(&t.db, 1, 1).await.unwrap();
    for sequence in [1, 2] {
        progress::report(&t.db, 1, 1, &report(A, sequence, 100_000), 20)
            .await
            .unwrap();
        assert!(stored(&t.db).await.is_empty());
    }
    assert_eq!(cursors(&t.db).await, [(1, 1, A.into(), 2)]);
    progress::report(&t.db, 1, 1, &report(A, 3, 90_000), 30)
        .await
        .unwrap();
    assert_eq!(stored(&t.db).await[0].3, 90_000);
}

#[tokio::test]
async fn a_duplicate_after_finish_does_not_start_replaying() {
    let t = library().await;
    let input = report(A, 2, 110_000);
    progress::report(&t.db, 1, 1, &input, 10).await.unwrap();
    progress::finish(&t.db, 1, 1, 11, "", 20).await.unwrap();
    let expected = vec![(
        1,
        1,
        Some(11),
        110_000,
        Some(String::new()),
        Some(20),
        false,
        20,
    )];
    assert_eq!(stored(&t.db).await, expected);
    progress::report(&t.db, 1, 1, &input, 30).await.unwrap();
    assert_eq!(stored(&t.db).await, expected);
    progress::report(&t.db, 1, 1, &report(A, 3, 90_000), 40)
        .await
        .unwrap();
    let replaying = vec![(
        1,
        1,
        Some(11),
        90_000,
        Some(String::new()),
        Some(20),
        true,
        40,
    )];
    assert_eq!(stored(&t.db).await, replaying);
    progress::report(&t.db, 1, 1, &input, 50).await.unwrap();
    assert_eq!(stored(&t.db).await, replaying);
}

#[tokio::test]
async fn failed_progress_writes_roll_back_new_and_existing_cursors() {
    let t = library().await;
    for sequence in [1, 2] {
        let input = ReportInput {
            track_id: 99,
            ..report(A, sequence, 90_000)
        };
        let before = stored(&t.db).await;
        let error = progress::report(&t.db, 1, 1, &input, 10).await.unwrap_err();
        assert!(
            error
                .as_database_error()
                .unwrap()
                .is_foreign_key_violation(),
            "{error}"
        );
        assert_eq!(stored(&t.db).await, before);
        let expected = if sequence == 1 {
            vec![]
        } else {
            vec![(1, 1, A.into(), 1)]
        };
        assert_eq!(cursors(&t.db).await, expected);
        exec(&t.db, "INSERT INTO tracks (id, work_id, location_id, rel_path, display_path, sort_key, kind)
                     VALUES (99, 1, 1, CAST('repaired.mp3' AS BLOB), 'repaired.mp3', 'repaired.mp3', 'audio')").await;
        progress::report(&t.db, 1, 1, &input, 10).await.unwrap();
        assert_eq!(stored(&t.db).await[0].2, Some(99));
        assert_eq!(cursors(&t.db).await, [(1, 1, A.into(), sequence)]);
        exec(&t.db, "UPDATE play_progress SET track_id = NULL WHERE track_id = 99; DELETE FROM tracks WHERE id = 99").await;
    }
}

#[tokio::test]
async fn cursors_are_scoped_to_user_and_work_and_cascade_only_with_their_parents() {
    let t = library().await;
    for (user, work, track, sequence) in [(1, 1, 11, 2), (2, 1, 11, 1), (1, 2, 21, 1)] {
        let input = ReportInput {
            track_id: track,
            ..report(A, sequence, 90_000)
        };
        progress::report(&t.db, user, work, &input, 10)
            .await
            .unwrap();
    }
    assert_eq!(stored(&t.db).await.len(), 3);
    assert_eq!(
        cursors(&t.db).await,
        [
            (1, 1, A.into(), 2),
            (1, 2, A.into(), 1),
            (2, 1, A.into(), 1)
        ]
    );
    exec(&t.db, "UPDATE play_progress SET track_id = NULL WHERE track_id = 11; DELETE FROM tracks WHERE id = 11").await;
    assert_eq!(cursors(&t.db).await.len(), 3);
    exec(&t.db, "DELETE FROM users WHERE id = 2").await;
    assert_eq!(
        cursors(&t.db).await,
        [(1, 1, A.into(), 2), (1, 2, A.into(), 1)]
    );
    exec(&t.db, "DELETE FROM works WHERE id = 1").await;
    assert_eq!(cursors(&t.db).await, [(1, 2, A.into(), 1)]);
}

#[tokio::test]
async fn reopening_keeps_the_watermark_even_after_clear() {
    let t = library().await;
    progress::report(&t.db, 1, 1, &report(A, 2, 110_000), 10)
        .await
        .unwrap();
    progress::clear(&t.db, 1, 1).await.unwrap();
    t.db.close().await;
    let reopened = Db::open(&t.location).await.unwrap();
    progress::report(&reopened, 1, 1, &report(A, 1, 100_000), 20)
        .await
        .unwrap();
    assert!(stored(&reopened).await.is_empty());
    assert_eq!(cursors(&reopened).await, [(1, 1, A.into(), 2)]);
    reopened.close().await;
}

mod pg;

#[tokio::test]
async fn migrating_from_13_keeps_progress_and_does_not_invent_cursors() {
    use neo_db::Location;
    let dir = tempfile::tempdir().unwrap();
    let (db, location) = if testing::on_postgres() {
        let (pool, url) = pg::fresh().await;
        neo_db::PG_MIGRATOR.run_to(13, &pool).await.unwrap();
        (
            Db::Postgres(neo_db_postgres::Db {
                read: pool.clone(),
                write: pool,
            }),
            Location::Postgres(url),
        )
    } else {
        let path = dir.path().join("old.sqlite3");
        let pool = sqlx::SqlitePool::connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true),
        )
        .await
        .unwrap();
        neo_db::MIGRATOR.run_to(13, &pool).await.unwrap();
        (
            Db::Sqlite(neo_db_sqlite::Db {
                read: pool.clone(),
                write: pool,
            }),
            Location::Sqlite(path),
        )
    };
    exec(&db, FIXTURE).await;
    exec(
        &db,
        "INSERT INTO play_progress (user_id, work_id, track_id, position_ms, group_path,
               finished_at, replaying, updated_at) VALUES (1, 1, 11, 90000, '', 5, TRUE, 10)",
    )
    .await;
    let before = stored(&db).await;
    db.close().await;
    let upgraded = Db::open(&location).await.unwrap();
    assert_eq!(stored(&upgraded).await, before);
    assert!(cursors(&upgraded).await.is_empty());
    progress::report(&upgraded, 1, 1, &report(A, 2, 110_000), 20)
        .await
        .unwrap();
    let accepted = stored(&upgraded).await;
    progress::report(&upgraded, 1, 1, &report(A, 1, 100_000), 30)
        .await
        .unwrap();
    assert_eq!(stored(&upgraded).await, accepted);
    upgraded.close().await;
}

#[tokio::test]
async fn a_conflicting_report_rechecks_the_cursor_on_retry() {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicU32, Ordering},
        },
        time::Duration,
    };
    use tokio::sync::Barrier;

    let t = library().await;
    progress::report(&t.db, 1, 1, &report(A, 1, 50_000), 10)
        .await
        .unwrap();
    let attempts = Arc::new(AtomicU32::new(0));
    let retry = Arc::new(Barrier::new(2));
    // Gate the second checkout, after the first transaction has failed.
    // This controls the retry without adding a hook to the production query.
    macro_rules! gated_pool {
        ($builder:expr, $options:expr) => {{
            let attempts = attempts.clone();
            let retry = retry.clone();
            $builder
                .max_connections(1)
                .min_connections(1)
                .before_acquire(move |_, _| {
                    let attempt = attempts.fetch_add(1, Ordering::SeqCst);
                    let retry = retry.clone();
                    Box::pin(async move {
                        if attempt == 1 {
                            retry.wait().await;
                            retry.wait().await;
                        }
                        Ok(true)
                    })
                })
                .connect_with($options)
                .await
                .unwrap()
        }};
    }
    let old = match &t.db {
        Db::Sqlite(db) => Db::Sqlite(neo_db_sqlite::Db {
            read: db.read.clone(),
            write: gated_pool!(sqlx::sqlite::SqlitePoolOptions::new(),
                db.write.connect_options().as_ref().clone().busy_timeout(Duration::ZERO)),
        }),
        Db::Postgres(db) => Db::Postgres(neo_db_postgres::Db {
            read: db.read.clone(),
            write: gated_pool!(sqlx::postgres::PgPoolOptions::new().after_connect(|conn, _| {
                Box::pin(async move {
                    sqlx::query("SET SESSION CHARACTERISTICS AS TRANSACTION ISOLATION LEVEL SERIALIZABLE")
                        .execute(conn).await?;
                    Ok(())
                })
            }), db.write.connect_options().as_ref().clone().application_name("progress-order-retry")),
        }),
    };
    // Pool startup also checks out the connection; count only report attempts.
    attempts.store(0, Ordering::SeqCst);
    let mut newer = t.db.begin().await.unwrap();
    let update = "UPDATE progress_report_cursors SET last_sequence = 3;
                  UPDATE play_progress SET track_id = 12, position_ms = 110000,
                    group_path = 'new', updated_at = 30";
    match &mut newer {
        neo_db::Tx::Sqlite(tx) => {
            sqlx::raw_sql(update).execute(&mut **tx).await.unwrap();
        }
        neo_db::Tx::Postgres(tx) => {
            sqlx::raw_sql(update).execute(&mut **tx).await.unwrap();
        }
    };
    let release_newer = async {
        if let Db::Postgres(db) = &t.db {
            // Wait until the old UPSERT has a snapshot and is blocked on
            // the newer transaction's row lock. Committing causes 40001.
            loop {
                let blocked: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM pg_stat_activity
                     WHERE datname = current_database() AND application_name = 'progress-order-retry'
                       AND wait_event_type = 'Lock')",
                ).fetch_one(&db.read).await.unwrap();
                if blocked {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            newer.commit().await.unwrap();
            retry.wait().await;
        } else {
            // SQLite's first attempt gets BUSY while the writer holds the
            // lock. Commit only once the retry is waiting at the barrier.
            retry.wait().await;
            newer.commit().await.unwrap();
        }
        retry.wait().await;
    };
    let input = report(A, 2, 100_000);
    let (result, ()) = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::join!(progress::report(&old, 1, 1, &input, 99), release_newer)
    })
    .await
    .expect("the conflict and retry must reach both barriers");
    result.unwrap();
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        2,
        "one failed attempt and one retry"
    );
    assert_eq!(
        stored(&t.db).await,
        [(1, 1, Some(12), 110_000, Some("new".into()), None, false, 30)]
    );
    assert_eq!(cursors(&t.db).await, [(1, 1, A.into(), 3)]);
    old.close().await;
    t.db.close().await;
}
