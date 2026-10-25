//! The log line each task leaves when it ends. It has a test binary of its
//! own: tracing caches whether a call site is enabled, and other tests
//! ending tasks on other threads without a subscriber would turn it off.

use std::{
    io::Write,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use futures_util::FutureExt;
use neo_db::Db;
use neo_server::tasks::{Job, JobError, Lane, Scheduler, TaskSpec};
use serde_json::json;

/// Collects what the fmt subscriber writes.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Captured {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

fn spec(kind: &'static str, payload: serde_json::Value) -> TaskSpec {
    TaskSpec {
        kind,
        dedupe_key: None,
        payload,
        lane: Lane::Heavy,
        timeout: None,
        queue_limit: None,
    }
}

fn done() -> Job {
    Box::new(|_| async { Ok(None) }.boxed())
}

fn ended_line(log: &str, id: i64) -> Option<&str> {
    log.lines()
        .find(|l| l.contains("task ended") && l.contains(&format!("task={id} ")))
}

#[tokio::test]
async fn every_ended_task_is_logged_and_probes_only_at_debug() {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    // The test runtime has one thread, so the scheduler's tasks log here too.
    let _guard = tracing::subscriber::set_default(subscriber);
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open_sqlite(&dir.path().join("neo.sqlite3"))
        .await
        .unwrap();
    // An analysis the previous process left running ends at startup.
    sqlx::raw_sql(
        "INSERT INTO tasks (id, kind, state, payload, created_at, updated_at)
         VALUES (100, 'analysis', 'running', '{\"track_id\": 6}', 0, 0)",
    )
    .execute(&db.sqlite().unwrap().write)
    .await
    .unwrap();
    let scheduler = Scheduler::new(db.clone(), 1, 4);
    scheduler.recover(|_| None).await.unwrap();
    // A queued row this process does not hold, as when a cancel lands
    // between the insert and the spawn, is cancelled in the database alone.
    sqlx::raw_sql(
        "INSERT INTO tasks (id, kind, state, payload, created_at, updated_at)
         VALUES (200, 'transcode', 'queued', '{\"track_id\": 5}', 0, 0)",
    )
    .execute(&db.sqlite().unwrap().write)
    .await
    .unwrap();
    assert!(scheduler.cancel(200).await.unwrap());
    let failing: Job = Box::new(|_| async { Err(JobError("转码失败".into())) }.boxed());
    // One heavy lane: each task starts only after the one before it has
    // ended and logged, so once the last line is there, the probe's would be.
    let analysis = scheduler
        .submit(spec("analysis", json!({"track_id": 7})), done())
        .await
        .unwrap()
        .id;
    let transcode = scheduler
        .submit(spec("transcode", json!({})), failing)
        .await
        .unwrap()
        .id;
    let probe = scheduler
        .submit(spec("probe", json!({"track_id": 8})), done())
        .await
        .unwrap()
        .id;
    let last = scheduler
        .submit(spec("metadata", json!({"work_id": 3})), done())
        .await
        .unwrap()
        .id;
    let deadline = Instant::now() + Duration::from_secs(60);
    while ended_line(&captured.text(), last).is_none() {
        assert!(
            Instant::now() < deadline,
            "no line for the last task: {:?}",
            captured.text()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let log = captured.text();
    let line = |id: i64| ended_line(&log, id);
    let restarted = line(100).unwrap_or_else(|| panic!("no line for the restart in {log:?}"));
    assert!(restarted.contains("state=\"interrupted\""), "{restarted}");
    assert!(restarted.contains("服务重启"), "{restarted}");
    let cancelled = line(200).unwrap_or_else(|| panic!("no line for the cancel in {log:?}"));
    assert!(cancelled.contains("state=\"cancelled\""), "{cancelled}");
    assert!(cancelled.contains("kind=\"transcode\""), "{cancelled}");
    let analysis = line(analysis).unwrap_or_else(|| panic!("no analysis line in {log:?}"));
    assert!(analysis.contains("kind=\"analysis\""), "{analysis}");
    assert!(analysis.contains("state=\"done\""), "{analysis}");
    assert!(analysis.contains("track_id"), "{analysis}");
    let transcode = line(transcode).unwrap_or_else(|| panic!("no transcode line in {log:?}"));
    assert!(transcode.contains("state=\"failed\""), "{transcode}");
    assert!(transcode.contains("转码失败"), "{transcode}");
    assert!(
        line(probe).is_none(),
        "probes stay out of the info log: {log}"
    );
}
