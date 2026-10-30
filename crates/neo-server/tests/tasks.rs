mod common;

use std::time::Duration;

use axum::http::StatusCode;
use common::{Req, app};
use futures_util::FutureExt;
use neo_db::{Db, tasks::TaskRow};
use neo_server::tasks::{Job, JobError, Lane, Scheduler, SubmitError, TaskSpec};
use serde_json::json;
use tokio::sync::oneshot;

fn spec(kind: &'static str, key: Option<&str>) -> TaskSpec {
    TaskSpec {
        kind,
        dedupe_key: key.map(str::to_owned),
        payload: json!({}),
        lane: Lane::Heavy,
        timeout: None,
        queue_limit: None,
    }
}

fn returns(value: serde_json::Value) -> Job {
    Box::new(move |_| async move { Ok(Some(value)) }.boxed())
}

/// A job that runs until `release` fires or it is cancelled. It reports
/// when it has started.
fn blocking(started: oneshot::Sender<()>, release: oneshot::Receiver<()>) -> Job {
    Box::new(move |ctx| {
        async move {
            let _ = started.send(());
            tokio::select! {
                _ = release => Ok(None),
                () = ctx.cancel.cancelled() => Err(JobError("stopped".into())),
            }
        }
        .boxed()
    })
}

/// Polls until the task reaches `state`. The deadline only bounds a
/// failing test, so it is far above what a loaded machine needs.
async fn wait_for(scheduler: &Scheduler, id: i64, state: &str) -> TaskRow {
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        let row = scheduler.get(id).await.unwrap().unwrap();
        if row.state == state {
            return row;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!(
        "task {id} never reached {state}: {:?}",
        scheduler.get(id).await
    );
}

async fn db() -> (neo_db::testing::TestDb, Db) {
    let test = neo_db::testing::open().await;
    let db = test.db.clone();
    (test, db)
}

async fn sqlite_db() -> (tempfile::TempDir, Db) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open_sqlite(&dir.path().join("neo.sqlite3"))
        .await
        .unwrap();
    (dir, db)
}

#[tokio::test]
async fn a_job_runs_to_done_with_its_result_and_progress() {
    let (_dir, db) = db().await;
    let scheduler = Scheduler::new(db, 1, 4);
    let (release, wait) = oneshot::channel::<()>();
    let job: Job = Box::new(|ctx| {
        async move {
            ctx.progress().send(0.5).unwrap();
            let _ = wait.await;
            Ok(Some(
                json!({"url": "/api/tracks/1/playable", "mime_type": "audio/mp4"}),
            ))
        }
        .boxed()
    });
    let submitted = scheduler
        .submit(spec("transcode", None), job)
        .await
        .unwrap();
    assert!(submitted.created);
    wait_for(&scheduler, submitted.id, "running").await;
    // The job runs until released and progress is flushed once a second,
    // so the test waits for the flush instead of sleeping past it, which a
    // loaded machine can outlast.
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        let midway = scheduler.get(submitted.id).await.unwrap().unwrap();
        assert_eq!(midway.state, "running");
        if midway.progress == Some(0.5) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "progress is flushed while running: {:?}",
            midway.progress
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    release.send(()).unwrap();
    let done = wait_for(&scheduler, submitted.id, "done").await;
    assert_eq!(done.progress, Some(1.0));
    assert!(done.result.unwrap().contains("playable"));
}

#[tokio::test]
async fn one_active_task_per_dedupe_key() {
    let (_dir, db) = db().await;
    let scheduler = Scheduler::new(db, 1, 4);
    let (started, started_rx) = oneshot::channel();
    let (release, release_rx) = oneshot::channel();
    let first = scheduler
        .submit(spec("transcode", Some("k")), blocking(started, release_rx))
        .await
        .unwrap();
    started_rx.await.unwrap();
    let second = scheduler
        .submit(
            spec("transcode", Some("k")),
            Box::new(|_| async { panic!("must not run") }.boxed()),
        )
        .await
        .unwrap();
    assert_eq!(second.id, first.id);
    assert!(!second.created);
    release.send(()).unwrap();
    wait_for(&scheduler, first.id, "done").await;
    let third = scheduler
        .submit(spec("transcode", Some("k")), returns(json!(null)))
        .await
        .unwrap();
    assert_ne!(
        third.id, first.id,
        "a finished task no longer blocks its key"
    );
}

#[tokio::test]
async fn a_full_queue_refuses_new_tasks() {
    let (_dir, db) = db().await;
    let scheduler = Scheduler::new(db, 1, 4);
    let limited = |key| TaskSpec {
        queue_limit: Some(1),
        ..spec("transcode", Some(key))
    };
    let (started, started_rx) = oneshot::channel();
    let (_release, release_rx) = oneshot::channel::<()>();
    scheduler
        .submit(limited("a"), blocking(started, release_rx))
        .await
        .unwrap();
    started_rx.await.unwrap();
    scheduler
        .submit(limited("b"), returns(json!(null)))
        .await
        .unwrap();
    let error = scheduler
        .submit(limited("c"), returns(json!(null)))
        .await
        .unwrap_err();
    assert!(matches!(error, SubmitError::QueueFull), "{error:?}");
    // A duplicate of a waiting task is still answered, full queue or not.
    let duplicate = scheduler
        .submit(limited("b"), returns(json!(null)))
        .await
        .unwrap();
    assert!(!duplicate.created);
}

#[tokio::test]
async fn cancelling_a_queued_task_never_runs_it() {
    let (_dir, db) = db().await;
    let scheduler = Scheduler::new(db, 1, 4);
    let (started, started_rx) = oneshot::channel();
    let (release, release_rx) = oneshot::channel();
    let blocker = scheduler
        .submit(spec("transcode", None), blocking(started, release_rx))
        .await
        .unwrap();
    started_rx.await.unwrap();
    let queued = scheduler
        .submit(
            spec("transcode", None),
            Box::new(|_| async { panic!("must not run") }.boxed()),
        )
        .await
        .unwrap();
    assert!(scheduler.cancel(queued.id).await.unwrap());
    wait_for(&scheduler, queued.id, "cancelled").await;
    release.send(()).unwrap();
    wait_for(&scheduler, blocker.id, "done").await;
}

#[tokio::test]
async fn cancelling_a_running_task_lets_the_job_clean_up() {
    let (_dir, db) = db().await;
    let scheduler = Scheduler::new(db, 1, 4);
    let (started, started_rx) = oneshot::channel();
    let (_release, release_rx) = oneshot::channel::<()>();
    let task = scheduler
        .submit(spec("transcode", None), blocking(started, release_rx))
        .await
        .unwrap();
    started_rx.await.unwrap();
    scheduler.cancel(task.id).await.unwrap();
    let row = wait_for(&scheduler, task.id, "cancelled").await;
    assert_eq!(row.message, None);
}

#[tokio::test]
async fn a_timeout_fails_the_task() {
    let (_dir, db) = db().await;
    let scheduler = Scheduler::new(db, 1, 4);
    let (started, _started_rx) = oneshot::channel();
    let (_release, release_rx) = oneshot::channel::<()>();
    let timed = TaskSpec {
        timeout: Some(Duration::from_millis(100)),
        ..spec("transcode", None)
    };
    let task = scheduler
        .submit(timed, blocking(started, release_rx))
        .await
        .unwrap();
    let row = wait_for(&scheduler, task.id, "failed").await;
    assert_eq!(row.message.as_deref(), Some("任务超时"));
}

#[tokio::test]
async fn a_job_error_becomes_the_task_message() {
    let (_dir, db) = db().await;
    let scheduler = Scheduler::new(db, 1, 4);
    let job: Job = Box::new(|_| async { Err(JobError("ffmpeg exited with 1".into())) }.boxed());
    let task = scheduler
        .submit(spec("transcode", None), job)
        .await
        .unwrap();
    let row = wait_for(&scheduler, task.id, "failed").await;
    assert_eq!(row.message.as_deref(), Some("ffmpeg exited with 1"));
}

#[tokio::test]
async fn lanes_limit_concurrency_separately() {
    let (_dir, db) = db().await;
    let scheduler = Scheduler::new(db, 1, 4);
    let (started, started_rx) = oneshot::channel();
    let (release, release_rx) = oneshot::channel();
    let heavy = scheduler
        .submit(spec("transcode", None), blocking(started, release_rx))
        .await
        .unwrap();
    started_rx.await.unwrap();
    let light = TaskSpec {
        lane: Lane::Light,
        ..spec("probe", None)
    };
    let probe = scheduler.submit(light, returns(json!(null))).await.unwrap();
    wait_for(&scheduler, probe.id, "done").await;
    let waiting = scheduler
        .submit(spec("transcode", None), returns(json!(null)))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        scheduler.get(waiting.id).await.unwrap().unwrap().state,
        "queued"
    );
    release.send(()).unwrap();
    wait_for(&scheduler, heavy.id, "done").await;
    wait_for(&scheduler, waiting.id, "done").await;
}

#[tokio::test]
async fn restart_interrupts_running_tasks_and_resumes_queued_ones() {
    let (_dir, db) = db().await;
    neo_db::testing::exec(
        &db,
        "INSERT INTO tasks (id, kind, dedupe_key, state, payload, created_at, updated_at) VALUES
           (1, 'transcode', 'a', 'running', '{}', 0, 0),
           (2, 'transcode', 'b', 'queued', '{\"track\": 7}', 0, 0),
           (3, 'metadata', 'c', 'queued', '{}', 0, 0),
           (4, 'transcode', 'd', 'done', '{}', 0, 9999999999999),
           (5, 'transcode', 'e', 'done', '{}', 0, 0)",
    )
    .await;
    let scheduler = Scheduler::new(db, 1, 4);
    scheduler
        .recover(|row| {
            (row.kind == "transcode").then(|| {
                let payload = row.payload.clone();
                (Lane::Heavy, None, returns(json!({"from": payload})))
            })
        })
        .await
        .unwrap();
    assert_eq!(
        wait_for(&scheduler, 1, "interrupted")
            .await
            .message
            .as_deref(),
        Some("服务重启，任务被中断")
    );
    let resumed = wait_for(&scheduler, 2, "done").await;
    assert!(resumed.result.unwrap().contains("track"));
    wait_for(&scheduler, 3, "interrupted").await;
    wait_for(&scheduler, 4, "done").await;
    assert!(
        scheduler.get(5).await.unwrap().is_none(),
        "a finished task past the retention is pruned at startup"
    );
}

#[tokio::test]
async fn shutdown_interrupts_running_tasks_and_keeps_queued_ones() {
    let (_dir, db) = db().await;
    let scheduler = Scheduler::new(db, 1, 4);
    let (started, started_rx) = oneshot::channel();
    let (_release, release_rx) = oneshot::channel::<()>();
    let running = scheduler
        .submit(spec("transcode", None), blocking(started, release_rx))
        .await
        .unwrap();
    started_rx.await.unwrap();
    let queued = scheduler
        .submit(spec("transcode", None), returns(json!(null)))
        .await
        .unwrap();
    scheduler.shutdown(Duration::from_secs(5)).await;
    assert_eq!(
        scheduler.get(running.id).await.unwrap().unwrap().state,
        "interrupted"
    );
    assert_eq!(
        scheduler.get(queued.id).await.unwrap().unwrap().state,
        "queued"
    );
}

/// A scan that records when it starts, so a test can read the order the
/// scan lane ran them in.
fn recording(order: &std::sync::Arc<std::sync::Mutex<Vec<i64>>>) -> Job {
    let order = std::sync::Arc::clone(order);
    Box::new(move |ctx| {
        async move {
            order.lock().unwrap().push(ctx.task_id);
            Ok(None)
        }
        .boxed()
    })
}

#[tokio::test]
async fn a_restart_keeps_the_order_of_scans_moved_to_the_front() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("neo.sqlite3");
    let scan = |key: &str| TaskSpec {
        lane: Lane::Scan,
        ..spec("scan", Some(key))
    };
    let scheduler = Scheduler::new(Db::open_sqlite(&path).await.unwrap(), 1, 4);
    let (started, started_rx) = oneshot::channel();
    let (_release, release_rx) = oneshot::channel::<()>();
    scheduler
        .submit(scan("scan:a"), blocking(started, release_rx))
        .await
        .unwrap();
    started_rx.await.unwrap();
    let mut queued = Vec::new();
    for key in ["scan:b", "scan:c", "scan:d"] {
        let job = returns(json!(null));
        queued.push(scheduler.submit(scan(key), job).await.unwrap().id);
    }
    let [b, c, d] = queued[..] else {
        unreachable!()
    };
    scheduler.move_to_front(d).await.unwrap();
    scheduler.move_to_front(c).await.unwrap();
    let positions = [c, d, b].map(|id| scheduler.queue_position(id));
    assert_eq!(positions, [Some(1), Some(2), Some(3)]);
    scheduler.shutdown(Duration::from_secs(5)).await;

    // A scan submitted after the moves goes behind them, as it would have
    // in the old process.
    let db = Db::open_sqlite(&path).await.unwrap();
    sqlx::raw_sql(
        "INSERT INTO tasks (kind, dedupe_key, state, payload, created_at, updated_at)
         VALUES ('scan', 'scan:e', 'queued', '{}', 0, 0)",
    )
    .execute(&db.sqlite().unwrap().write)
    .await
    .unwrap();
    let order = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let restarted = Scheduler::new(db, 1, 4);
    restarted
        .recover(|_| Some((Lane::Scan, None, recording(&order))))
        .await
        .unwrap();
    wait_for(&restarted, b, "done").await;
    let e = d + 1;
    wait_for(&restarted, e, "done").await;
    assert_eq!(*order.lock().unwrap(), [c, d, b, e]);
}

#[tokio::test]
async fn a_move_finishes_even_when_its_caller_gives_up_during_the_write() {
    use sqlx::Connection;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("neo.sqlite3");
    let scheduler = Scheduler::new(Db::open_sqlite(&path).await.unwrap(), 1, 4);
    let scan = |key: &str| TaskSpec {
        lane: Lane::Scan,
        ..spec("scan", Some(key))
    };
    let (started, started_rx) = oneshot::channel();
    let (_release, release_rx) = oneshot::channel::<()>();
    scheduler
        .submit(scan("scan:a"), blocking(started, release_rx))
        .await
        .unwrap();
    started_rx.await.unwrap();
    let b = scheduler
        .submit(scan("scan:b"), returns(json!(null)))
        .await
        .unwrap()
        .id;
    let c = scheduler
        .submit(scan("scan:c"), returns(json!(null)))
        .await
        .unwrap()
        .id;

    // Another writer holds the database, so the move waits in its write.
    let mut other = sqlx::SqliteConnection::connect(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();
    sqlx::raw_sql("BEGIN IMMEDIATE")
        .execute(&mut other)
        .await
        .unwrap();
    let gave_up =
        tokio::time::timeout(Duration::from_millis(300), scheduler.move_to_front(c)).await;
    assert!(gave_up.is_err(), "the move waited for the writer");
    sqlx::raw_sql("COMMIT").execute(&mut other).await.unwrap();

    eventually(
        || async { scheduler.queue_position(c) == Some(1) },
        "the move to reach the queue in memory",
    )
    .await;
    assert_eq!(scheduler.queue_position(b), Some(2));
    scheduler.shutdown(Duration::from_secs(5)).await;
    let db = Db::open_sqlite(&path).await.unwrap();
    let recovered = neo_db::tasks::recover(&db, 0).await.unwrap();
    let order: Vec<i64> = recovered.queued.iter().map(|row| row.id).collect();
    assert_eq!(order, [c, b], "the stored order is the one in memory");
}

#[tokio::test]
async fn a_starting_scan_keeps_first_place_until_its_row_is_running() {
    use sqlx::Connection;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("neo.sqlite3");
    let scheduler = Scheduler::new(Db::open_sqlite(&path).await.unwrap(), 1, 4);
    let scan = |key: &str| TaskSpec {
        lane: Lane::Scan,
        ..spec("scan", Some(key))
    };
    // A cleanup holds the slot, so freeing it writes no task row.
    let cleanup = scheduler.try_scan_slot().unwrap();
    let (started, started_rx) = oneshot::channel();
    let (_release, release_rx) = oneshot::channel::<()>();
    let b = scheduler
        .submit(scan("scan:b"), blocking(started, release_rx))
        .await
        .unwrap()
        .id;
    let c = scheduler
        .submit(scan("scan:c"), returns(json!(null)))
        .await
        .unwrap()
        .id;

    // Another writer holds the database, so b's start write waits after b
    // has taken the slot.
    let mut other = sqlx::SqliteConnection::connect(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();
    sqlx::raw_sql("BEGIN IMMEDIATE")
        .execute(&mut other)
        .await
        .unwrap();
    drop(cleanup);
    for _ in 0..20 {
        let row = scheduler.get(b).await.unwrap().unwrap();
        assert_eq!(row.state, "queued");
        let positions = [b, c].map(|id| scheduler.queue_position(id));
        assert_eq!(positions, [Some(1), Some(2)], "a queued row has a place");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    sqlx::raw_sql("COMMIT").execute(&mut other).await.unwrap();

    started_rx.await.unwrap();
    assert_eq!(scheduler.get(b).await.unwrap().unwrap().state, "running");
    let positions = [b, c].map(|id| scheduler.queue_position(id));
    assert_eq!(positions, [None, Some(1)]);
}

#[tokio::test]
async fn task_snapshots_are_visible_to_admins_and_requesters_only() {
    let app = app().await;
    app.user("admin", true).await;
    let alice = app.user("alice", false).await;
    app.user("bob", false).await;
    let scheduler = app.state.scheduler().clone();
    let task = scheduler
        .submit(
            spec("transcode", None),
            returns(json!({"url": "/api/tracks/5/playable", "mime_type": "audio/mp4"})),
        )
        .await
        .unwrap();
    scheduler.add_requester(task.id, alice);
    wait_for(&scheduler, task.id, "done").await;
    let uri = format!("/api/tasks/{}", task.id);

    let (status, body) = app
        .send(Req::get(&uri).token(&app.login("alice").await))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "done");
    assert_eq!(body["progress"], 1);
    assert_eq!(body["result"]["url"], "/api/tracks/5/playable");

    let (status, _) = app
        .send(Req::get(&uri).token(&app.login("bob").await))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = app
        .send(Req::get(&uri).token(&app.login("admin").await))
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = app
        .send(Req::get("/api/tasks/999").token(&app.login("admin").await))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = app.send(Req::get(&uri)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn other_users_cannot_see_scans_even_when_they_know_the_id() {
    let app = app().await;
    app.user("admin", true).await;
    let alice = app.user("alice", false).await;
    let scheduler = app.state.scheduler().clone();
    let scan = TaskSpec {
        lane: Lane::Scan,
        ..spec("scan", Some("scan:local"))
    };
    let task = scheduler.submit(scan, returns(json!(null))).await.unwrap();
    scheduler.add_requester(task.id, alice);
    wait_for(&scheduler, task.id, "done").await;
    let (status, _) = app
        .send(Req::get(&format!("/api/tasks/{}", task.id)).token(&app.login("alice").await))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_malformed_task_id_gets_a_json_error() {
    let app = app().await;
    app.user("admin", true).await;
    let token = app.login("admin").await;
    for uri in ["/api/tasks/abc", "/api/tasks/99999999999999999999"] {
        let (status, body) = app.send(Req::get(uri).token(&token)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(body["error"]["code"], "invalid_request");
    }
}

#[tokio::test]
async fn shutdown_right_after_recovery_keeps_every_queued_task_queued() {
    let (_dir, db) = db().await;
    let mut values = Vec::new();
    for id in 1..=64 {
        values.push(format!("({id}, 'transcode', 'queued', '{{}}', 0, 0)"));
    }
    neo_db::testing::exec(
        &db,
        &format!(
            "INSERT INTO tasks (id, kind, state, payload, created_at, updated_at) VALUES {}",
            values.join(",")
        ),
    )
    .await;
    let scheduler = Scheduler::new(db, 4, 4);
    scheduler.shutdown(Duration::ZERO).await;
    scheduler
        .recover(|_| {
            let job: Job = Box::new(|ctx| {
                async move {
                    ctx.cancel.cancelled().await;
                    Err(JobError("stopped".into()))
                }
                .boxed()
            });
            Some((Lane::Heavy, None, job))
        })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    for id in 1..=64 {
        assert_eq!(
            scheduler.get(id).await.unwrap().unwrap().state,
            "queued",
            "task {id}"
        );
    }
}

#[tokio::test]
async fn a_job_holding_the_write_connection_still_times_out_promptly() {
    let (_dir, db) = db().await;
    let scheduler = Scheduler::new(db.clone(), 1, 4);
    let job: Job = Box::new(move |ctx| {
        async move {
            // Holds the only write connection while progress is pending.
            let _tx = db.begin().await.unwrap();
            ctx.progress().send(0.5).unwrap();
            tokio::select! {
                () = tokio::time::sleep(Duration::from_secs(300)) => {}
                () = ctx.cancel.cancelled() => {}
            }
            Err(JobError("stopped".into()))
        }
        .boxed()
    });
    // A timeout stuck behind the held connection would wait for the pool's
    // 30 s acquire timeout or the job's 300 s. The 20 s bound is below
    // both and far above the 1.5 s timeout, so a loaded machine does not
    // fail it.
    let timed = TaskSpec {
        timeout: Some(Duration::from_millis(1500)),
        ..spec("transcode", None)
    };
    let started = std::time::Instant::now();
    let task = scheduler.submit(timed, job).await.unwrap();
    let row = wait_for(&scheduler, task.id, "failed").await;
    assert_eq!(row.message.as_deref(), Some("任务超时"));
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn a_failed_state_write_is_retried_until_it_succeeds() {
    let (_dir, db) = sqlite_db().await;
    sqlx::raw_sql(
        "CREATE TRIGGER refuse_done BEFORE UPDATE ON tasks WHEN NEW.state = 'done'
         BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
    )
    .execute(&db.sqlite().unwrap().write)
    .await
    .unwrap();
    let scheduler = Scheduler::new(db.clone(), 1, 4);
    let task = scheduler
        .submit(spec("transcode", Some("k")), returns(json!(null)))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        scheduler.get(task.id).await.unwrap().unwrap().state,
        "running"
    );
    sqlx::raw_sql("DROP TRIGGER refuse_done")
        .execute(&db.sqlite().unwrap().write)
        .await
        .unwrap();
    wait_for(&scheduler, task.id, "done").await;
    let next = scheduler
        .submit(spec("transcode", Some("k")), returns(json!(null)))
        .await
        .unwrap();
    assert!(next.created, "the key is free again");
}

#[tokio::test]
async fn a_dropped_submit_still_runs_its_task() {
    let (_dir, db) = db().await;
    let scheduler = Scheduler::new(db, 1, 4);
    {
        let submit = scheduler.submit(spec("transcode", Some("k")), returns(json!(null)));
        tokio::pin!(submit);
        // Poll once so the work starts, then drop the caller.
        assert!(futures_util::poll!(submit.as_mut()).is_pending());
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let again = scheduler
        .submit(spec("transcode", Some("k")), returns(json!(null)))
        .await
        .unwrap();
    wait_for(&scheduler, again.id, "done").await;
    // The dropped submit created task 1, and it ran instead of being stranded.
    assert_eq!(scheduler.get(1).await.unwrap().unwrap().state, "done");
}

#[tokio::test]
async fn an_explicit_cancel_followed_by_shutdown_stays_cancelled() {
    let (_dir, db) = db().await;
    let scheduler = Scheduler::new(db, 1, 4);
    let (started, started_rx) = oneshot::channel();
    let (_release, release_rx) = oneshot::channel::<()>();
    let running = scheduler
        .submit(spec("transcode", None), blocking(started, release_rx))
        .await
        .unwrap();
    started_rx.await.unwrap();
    let queued = scheduler
        .submit(spec("transcode", None), returns(json!(null)))
        .await
        .unwrap();
    assert!(scheduler.cancel(queued.id).await.unwrap());
    assert!(scheduler.cancel(running.id).await.unwrap());
    scheduler.shutdown(Duration::from_secs(5)).await;
    assert_eq!(
        scheduler.get(queued.id).await.unwrap().unwrap().state,
        "cancelled"
    );
    assert_eq!(
        scheduler.get(running.id).await.unwrap().unwrap().state,
        "cancelled"
    );
}

#[tokio::test]
async fn progress_writes_stay_a_second_apart_after_a_busy_writer() {
    let (_dir, db) = sqlite_db().await;
    sqlx::raw_sql(
        "CREATE TABLE progress_log (at INTEGER NOT NULL);
         CREATE TRIGGER log_progress AFTER UPDATE OF progress ON tasks
         BEGIN
           INSERT INTO progress_log VALUES
             (CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER));
         END;",
    )
    .execute(&db.sqlite().unwrap().write)
    .await
    .unwrap();
    let scheduler = Scheduler::new(db.clone(), 1, 4);
    let writer = db.clone();
    let job: Job = Box::new(move |ctx| {
        async move {
            let conn = writer.sqlite().unwrap().write.acquire().await.unwrap();
            for step in 1..=22 {
                ctx.progress().send(f64::from(step) / 100.0).unwrap();
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            drop(conn);
            for step in 23..=40 {
                ctx.progress().send(f64::from(step) / 100.0).unwrap();
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Ok(None)
        }
        .boxed()
    });
    let task = scheduler
        .submit(spec("transcode", None), job)
        .await
        .unwrap();
    for _ in 0..100 {
        if scheduler.get(task.id).await.unwrap().unwrap().state == "done" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let times: Vec<i64> = sqlx::query_scalar("SELECT at FROM progress_log ORDER BY at")
        .fetch_all(&db.sqlite().unwrap().read)
        .await
        .unwrap();
    // The final write that marks the task done is not a progress tick.
    let ticks = &times[..times.len().saturating_sub(1)];
    for pair in ticks.windows(2) {
        assert!(
            pair[1] - pair[0] >= 500,
            "progress writes too close: {times:?}"
        );
    }
}

#[tokio::test]
async fn begin_shutdown_stops_tasks_without_waiting_for_them() {
    let (_dir, db) = db().await;
    let scheduler = Scheduler::new(db, 1, 4);
    let (started, started_rx) = oneshot::channel();
    let (_release, release_rx) = oneshot::channel::<()>();
    let running = scheduler
        .submit(spec("transcode", None), blocking(started, release_rx))
        .await
        .unwrap();
    started_rx.await.unwrap();
    let queued = scheduler
        .submit(
            spec("transcode", None),
            Box::new(|_| async { panic!("must not run") }.boxed()),
        )
        .await
        .unwrap();
    scheduler.begin_shutdown();
    wait_for(&scheduler, running.id, "interrupted").await;
    assert_eq!(
        scheduler.get(queued.id).await.unwrap().unwrap().state,
        "queued"
    );
}

/// Finished tasks are pruned so repeated failures cannot grow the task
/// table and the requester map without bound. Active tasks are never
/// touched.
#[tokio::test]
async fn finished_tasks_and_their_requesters_are_pruned() {
    use neo_server::tasks::Retention;
    let (_dir, db) = db().await;
    let scheduler = Scheduler::new(db, 2, 1).with_retention(Retention {
        requesters: Duration::ZERO,
        rows: Duration::from_secs(3600),
        keep_rows: 1,
        prune_every: Duration::ZERO,
    });
    let (started, release) = (oneshot::channel(), oneshot::channel::<()>());
    let active = scheduler
        .submit(
            spec("transcode", Some("active")),
            blocking(started.0, release.1),
        )
        .await
        .unwrap();
    started.1.await.unwrap();

    let mut finished = Vec::new();
    for key in ["a", "b", "c"] {
        // The job ends only after its requester is recorded. A job that
        // ended first would sweep its own entry at once (the grace period
        // is zero), and the late requester would then wait for an end
        // that already happened.
        let (started, release) = (oneshot::channel(), oneshot::channel::<()>());
        let task = scheduler
            .submit(spec("transcode", Some(key)), blocking(started.0, release.1))
            .await
            .unwrap();
        started.1.await.unwrap();
        scheduler.add_requester(task.id, 7);
        release.0.send(()).unwrap();
        wait_for(&scheduler, task.id, "done").await;
        finished.push(task.id);
    }
    // Pruning runs as a task ends, just after its state is written, so the
    // test waits for it rather than for a fixed time.
    let newest = *finished.last().unwrap();
    for &id in &finished[..2] {
        eventually(
            || async { scheduler.get(id).await.unwrap().is_none() },
            "task kept",
        )
        .await;
        eventually(
            || async { !scheduler.is_requester(id, 7) },
            "requester kept",
        )
        .await;
    }
    assert!(
        scheduler.get(newest).await.unwrap().is_some(),
        "the newest is kept"
    );
    assert_eq!(
        scheduler.get(active.id).await.unwrap().unwrap().state,
        "running"
    );
    let _ = release.0.send(());
}

/// A requester recorded after its task already ended, as `prepare` does
/// for a job that fails at once, is forgotten like any other.
#[tokio::test]
async fn a_requester_added_after_the_task_ended_is_forgotten_too() {
    use neo_server::tasks::Retention;
    let (_dir, db) = db().await;
    let scheduler = Scheduler::new(db, 1, 1).with_retention(Retention {
        requesters: Duration::from_millis(100),
        rows: Duration::from_secs(3600),
        keep_rows: 100,
        prune_every: Duration::ZERO,
    });
    let task = scheduler
        .submit(spec("transcode", Some("fast")), returns(json!(1)))
        .await
        .unwrap();
    wait_for(&scheduler, task.id, "done").await;
    // The job's own task may not have left `live` yet; either way the end
    // time is already recorded.
    scheduler.add_requester(task.id, 7);
    assert!(
        scheduler.is_requester(task.id, 7),
        "still readable right after"
    );
    tokio::time::sleep(Duration::from_millis(150)).await;
    // Another task ending sweeps expired entries.
    let other = scheduler
        .submit(spec("transcode", Some("other")), returns(json!(1)))
        .await
        .unwrap();
    wait_for(&scheduler, other.id, "done").await;
    eventually(
        || async { !scheduler.is_requester(task.id, 7) },
        "requester kept",
    )
    .await;
}

/// Polls `condition` until it holds, failing after 10 s. Work that runs just
/// after a task's state is written is awaited this way, not with a sleep
/// that a loaded machine can outlast.
async fn eventually<F, Fut>(condition: F, what: &str)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !condition().await {
        assert!(std::time::Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Requesters of a task that has not ended are never swept, however long
/// it waits in the queue.
#[tokio::test]
async fn requesters_of_active_tasks_are_kept() {
    use neo_server::tasks::Retention;
    let (_dir, db) = db().await;
    let scheduler = Scheduler::new(db, 1, 1).with_retention(Retention {
        requesters: Duration::ZERO,
        rows: Duration::from_secs(3600),
        keep_rows: 100,
        prune_every: Duration::ZERO,
    });
    let (started, release) = (oneshot::channel(), oneshot::channel::<()>());
    let running = scheduler
        .submit(
            spec("transcode", Some("running")),
            blocking(started.0, release.1),
        )
        .await
        .unwrap();
    started.1.await.unwrap();
    let queued = scheduler
        .submit(spec("transcode", Some("queued")), returns(json!(1)))
        .await
        .unwrap();
    scheduler.add_requester(running.id, 7);
    scheduler.add_requester(queued.id, 7);
    let mut light = spec("probe", Some("light"));
    light.lane = Lane::Light;
    let other = scheduler.submit(light, returns(json!(1))).await.unwrap();
    wait_for(&scheduler, other.id, "done").await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(scheduler.is_requester(running.id, 7));
    assert!(scheduler.is_requester(queued.id, 7));
    let _ = release.0.send(());
}
