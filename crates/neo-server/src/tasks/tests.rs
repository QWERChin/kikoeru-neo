//! Orderings the public API cannot produce on demand, such as a cancel
//! that lands after a task's row commits but before its runner registers.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use futures_util::FutureExt;
use neo_db::{
    Db,
    tasks::{self, Enqueued},
};

use super::{About, Job, Lane, Scheduler};

#[cfg(feature = "postgres")]
#[tokio::test]
#[ignore = "needs a disposable PostgreSQL server with KIKOERU_TEST_BACKEND=postgres"]
async fn a_lost_enqueue_commit_ack_does_not_strand_the_task() {
    lost_enqueue_ack(Some("scan:synthetic")).await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
#[ignore = "needs a disposable PostgreSQL server with KIKOERU_TEST_BACKEND=postgres"]
async fn a_lost_enqueue_commit_ack_without_a_dedupe_key_still_runs() {
    lost_enqueue_ack(None).await;
}

#[cfg(feature = "postgres")]
fn enqueue_spec(key: Option<&str>) -> super::TaskSpec {
    super::TaskSpec {
        kind: "scan",
        dedupe_key: key.map(str::to_owned),
        payload: serde_json::json!({"root": "synthetic"}),
        lane: Lane::Scan,
        timeout: None,
        queue_limit: None,
    }
}

#[cfg(feature = "postgres")]
async fn lost_enqueue_ack(key: Option<&str>) {
    assert!(neo_db::testing::on_postgres());
    let test = neo_db::testing::open().await;
    let scheduler = Scheduler::new(test.db.clone(), 1, 4);
    let cleanup = scheduler.try_scan_slot().unwrap();
    let (ran, started) = tokio::sync::oneshot::channel();
    let job: Job = Box::new(|_| {
        async move {
            ran.send(()).unwrap();
            Ok(Some(serde_json::json!({"original": true})))
        }
        .boxed()
    });
    // Scope the fault inside submit's owned operation, since task-local
    // hooks do not propagate into the task that submit spawns.
    let error = tasks::ENQUEUE_COMMIT_OUTCOME
        .scope(true, scheduler.enqueue_and_spawn(enqueue_spec(key), job))
        .await
        .unwrap_err();
    assert!(matches!(error, super::SubmitError::Db(sqlx::Error::Io(_))));
    let rows = tasks::list(&test.db, None, None, 10, 0).await.unwrap();
    assert_eq!(rows.len(), 1);
    let id = rows[0].id;
    assert_eq!(rows[0].state, "queued");
    if key.is_some() {
        let duplicate = scheduler
            .submit(
                enqueue_spec(key),
                Box::new(|_| async { panic!("the duplicate job must not run") }.boxed()),
            )
            .await
            .unwrap();
        assert_eq!(duplicate.id, id);
        assert!(!duplicate.created);
    }
    drop(cleanup);
    tokio::time::timeout(Duration::from_secs(60), started)
        .await
        .expect("the committed task must run without a restart")
        .expect("the original job must not be dropped");
    scheduler.0.tracker.close();
    tokio::time::timeout(Duration::from_secs(60), scheduler.0.tracker.wait())
        .await
        .unwrap();
    let done = scheduler.get(id).await.unwrap().unwrap();
    assert_eq!(done.state, "done");
    assert_eq!(done.result.as_deref(), Some("{\"original\":true}"));
    assert_eq!(tasks::count(&test.db, None, None).await.unwrap(), 1);
}

#[cfg(feature = "postgres")]
#[tokio::test]
#[ignore = "needs a disposable PostgreSQL server with KIKOERU_TEST_BACKEND=postgres"]
async fn an_uncommitted_enqueue_never_runs_its_job() {
    assert!(neo_db::testing::on_postgres());
    let test = neo_db::testing::open().await;
    let scheduler = Scheduler::new(test.db.clone(), 1, 4);
    let (ran, started) = tokio::sync::oneshot::channel();
    let job: Job = Box::new(|_| {
        async move {
            ran.send(()).unwrap();
            Ok(None)
        }
        .boxed()
    });
    let error = tasks::ENQUEUE_COMMIT_OUTCOME
        .scope(false, scheduler.enqueue_and_spawn(enqueue_spec(None), job))
        .await
        .unwrap_err();
    assert!(matches!(error, super::SubmitError::Db(sqlx::Error::Io(_))));
    scheduler.0.tracker.close();
    tokio::time::timeout(Duration::from_secs(60), scheduler.0.tracker.wait())
        .await
        .unwrap();
    assert!(started.await.is_err(), "a rolled-back task must not run");
    assert_eq!(tasks::count(&test.db, None, None).await.unwrap(), 0);
}

async fn eventually(condition: impl Fn() -> bool, what: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while !condition() {
        assert!(std::time::Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn a_runner_that_registers_after_a_cancel_starts_out_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open_sqlite(&dir.path().join("neo.sqlite3"))
        .await
        .unwrap();
    let scheduler = Scheduler::new(db.clone(), 1, 4);
    // A cleanup holds the scan slot, so a runner that missed the cancel
    // would wait in the queue for it.
    let cleanup = scheduler.try_scan_slot().unwrap();
    // What submit does first: the row commits, and the runner is not yet
    // registered.
    let Enqueued::Created(id) = tasks::enqueue(&db, "scan", Some("scan:a"), "{}", None, 0)
        .await
        .unwrap()
    else {
        panic!("a new task");
    };
    assert!(scheduler.cancel(id).await.unwrap());
    assert_eq!(scheduler.get(id).await.unwrap().unwrap().state, "cancelled");

    let ran = Arc::new(AtomicBool::new(false));
    let job: Job = {
        let ran = Arc::clone(&ran);
        Box::new(move |_| {
            async move {
                ran.store(true, Ordering::SeqCst);
                Ok(None)
            }
            .boxed()
        })
    };
    let about = About {
        kind: "scan".to_owned(),
        payload: "{}".to_owned(),
    };
    scheduler.spawn(id, about, Lane::Scan, None, job);
    eventually(
        || scheduler.registry().tasks.is_empty() && scheduler.queue_position(id).is_none(),
        "the runner to end without waiting for the slot",
    )
    .await;
    assert!(
        scheduler.registry().early.is_empty(),
        "the mark was used up"
    );
    drop(cleanup);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!ran.load(Ordering::SeqCst), "the job never ran");
    assert_eq!(scheduler.get(id).await.unwrap().unwrap().state, "cancelled");
}

#[tokio::test]
async fn racing_cancels_leave_one_mark_and_none_once_the_runner_is_gone() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open_sqlite(&dir.path().join("neo.sqlite3"))
        .await
        .unwrap();
    let scheduler = Scheduler::new(db.clone(), 1, 4);
    let cleanup = scheduler.try_scan_slot().unwrap();
    let Enqueued::Created(id) = tasks::enqueue(&db, "scan", Some("scan:a"), "{}", None, 0)
        .await
        .unwrap()
    else {
        panic!("a new task");
    };
    // Two admins cancel before the runner registers, and both read the row
    // while it was queued. The second's write then finds it cancelled.
    let stale = scheduler.get(id).await.unwrap().unwrap();
    assert!(scheduler.cancel(id).await.unwrap());
    assert!(scheduler.cancel_unregistered(stale.clone()).await.unwrap());
    assert_eq!(scheduler.registry().early.get(&id), Some(&1));

    let job: Job = Box::new(|_| async { Ok(None) }.boxed());
    let about = About {
        kind: "scan".to_owned(),
        payload: "{}".to_owned(),
    };
    scheduler.spawn(id, about, Lane::Scan, None, job);
    eventually(
        || scheduler.registry().tasks.is_empty() && scheduler.queue_position(id).is_none(),
        "the runner to end without waiting for the slot",
    )
    .await;
    drop(cleanup);

    // A cancel that read the row before the runner ended arrives after it.
    assert!(scheduler.cancel_unregistered(stale).await.unwrap());
    assert!(!scheduler.cancel(id).await.unwrap());
    assert!(
        scheduler.registry().early.is_empty(),
        "no mark is left behind"
    );
}

#[tokio::test]
async fn a_cancel_for_an_ended_task_leaves_no_mark() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open_sqlite(&dir.path().join("neo.sqlite3"))
        .await
        .unwrap();
    let scheduler = Scheduler::new(db.clone(), 1, 4);
    let Enqueued::Created(id) = tasks::enqueue(&db, "scan", None, "{}", None, 0)
        .await
        .unwrap()
    else {
        panic!("a new task");
    };
    assert!(
        tasks::finish(&db, id, tasks::EndState::Done, None, None, 1)
            .await
            .unwrap()
    );
    assert!(!scheduler.cancel(id).await.unwrap());
    assert!(!scheduler.cancel(id + 1).await.unwrap(), "no such task");
    assert!(scheduler.registry().early.is_empty());
}

/// A job that runs until it is cancelled, reporting when it starts.
fn until_cancelled(started: tokio::sync::oneshot::Sender<()>) -> Job {
    Box::new(move |ctx| {
        async move {
            let _ = started.send(());
            ctx.cancel.cancelled().await;
            Err(super::JobError("stopped".into()))
        }
        .boxed()
    })
}

#[tokio::test]
async fn a_cancel_that_missed_the_runner_finds_it_once_the_row_runs() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open_sqlite(&dir.path().join("neo.sqlite3"))
        .await
        .unwrap();
    let scheduler = Scheduler::new(db.clone(), 1, 4);
    let Enqueued::Created(id) = tasks::enqueue(&db, "transcode", None, "{}", None, 0)
        .await
        .unwrap()
    else {
        panic!("a new task");
    };
    // The cancel's first check ran before this registration, and its read
    // of the row comes after the runner started it.
    let (started, started_rx) = tokio::sync::oneshot::channel();
    let about = About {
        kind: "transcode".to_owned(),
        payload: "{}".to_owned(),
    };
    scheduler.spawn(id, about, Lane::Heavy, None, until_cancelled(started));
    started_rx.await.unwrap();
    assert!(scheduler.cancel_without_runner(id).await.unwrap());
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while scheduler.get(id).await.unwrap().unwrap().state != "cancelled" {
        assert!(std::time::Instant::now() < deadline, "the job stopped");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn a_cancel_whose_write_fails_leaves_no_mark() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open_sqlite(&dir.path().join("neo.sqlite3"))
        .await
        .unwrap();
    let scheduler = Scheduler::new(db.clone(), 1, 4);
    let Enqueued::Created(id) = tasks::enqueue(&db, "scan", Some("scan:a"), "{}", None, 0)
        .await
        .unwrap()
    else {
        panic!("a new task");
    };
    let queued = scheduler.get(id).await.unwrap().unwrap();
    db.close().await;
    assert!(scheduler.cancel_unregistered(queued).await.is_err());
    assert!(scheduler.registry().early.is_empty());
}
