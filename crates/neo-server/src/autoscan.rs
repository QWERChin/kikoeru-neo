//! Automatic scans on each root's `auto_scan_interval`
//! (book/src/contract.md, "自动扫描和目录监听").

use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
    time::Duration,
};

use anyhow::Result;
use neo_core::dto::{RootKind, RootSettings};
use neo_db as db;
use neo_db::Db;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::{
    http::{AppState, RootEntry},
    scan,
};

/// Listing a cloud drive more often than this risks the provider's limits.
pub const MIN_OPENLIST_INTERVAL: u32 = 30 * 60;
pub const MIN_LOCAL_INTERVAL: u32 = 5 * 60;
pub const MAX_INTERVAL: u32 = 30 * 24 * 60 * 60;

/// How often a root that is due but still being scanned is looked at
/// again, and the longest the scheduler sleeps without a root change.
const RECHECK: Duration = Duration::from_secs(60);

/// Checks an interval in seconds against the bounds for its kind of root.
///
/// # Errors
/// The reason, worded for a config error message.
pub fn check_interval(kind: RootKind, secs: u32) -> Result<(), &'static str> {
    // WebDAV listings reach a remote server too, so they share the
    // OpenList floor (book/src/contract.md).
    let min = match kind {
        RootKind::Local => MIN_LOCAL_INTERVAL,
        RootKind::OpenList | RootKind::WebDav => MIN_OPENLIST_INTERVAL,
    };
    if secs < min {
        return Err(match kind {
            RootKind::Local => "must be at least 5 minutes for a local root",
            RootKind::OpenList => "must be at least 30 minutes for an OpenList root",
            RootKind::WebDav => "must be at least 30 minutes for a WebDAV root",
        });
    }
    if secs > MAX_INTERVAL {
        return Err("must be at most 30 days");
    }
    Ok(())
}

/// The scan settings of a registry entry.
pub(crate) fn interval_of(root: &RootEntry) -> Option<u32> {
    match &root.settings {
        RootSettings::Local {
            auto_scan_interval, ..
        }
        | RootSettings::OpenList {
            auto_scan_interval, ..
        }
        | RootSettings::WebDav {
            auto_scan_interval, ..
        } => *auto_scan_interval,
    }
}

/// Shared between the scheduler, the watcher and the roots API.
#[derive(Debug)]
pub(crate) struct Automation {
    /// Per root with automatic scans, the interval and when it was turned
    /// on (or the server started). A root that was never scanned counts
    /// its first interval from there.
    armed: Mutex<HashMap<String, (u32, i64)>>,
    /// Roots whose directory watcher is running.
    watching: Mutex<HashSet<String>>,
    /// Bumped whenever roots are added, edited or removed.
    changed: watch::Sender<u64>,
}

impl Default for Automation {
    fn default() -> Self {
        Self {
            armed: Mutex::default(),
            watching: Mutex::default(),
            changed: watch::Sender::new(0),
        }
    }
}

impl Automation {
    pub(crate) fn roots_changed(&self) {
        self.changed.send_modify(|v| *v = v.wrapping_add(1));
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    pub(crate) fn is_watching(&self, id: &str) -> bool {
        self.watching.lock().expect("lock poisoned").contains(id)
    }

    pub(crate) fn set_watching(&self, id: &str, on: bool) {
        let mut watching = self.watching.lock().expect("lock poisoned");
        if on {
            watching.insert(id.to_owned());
        } else {
            watching.remove(id);
        }
    }

    /// Records when each root's automatic scans were turned on: now, for a
    /// root seen for the first time or with a new interval.
    fn arm(&self, roots: &[RootEntry], now: i64) {
        let mut armed = self.armed.lock().expect("lock poisoned");
        let mut keep = HashSet::new();
        for root in roots {
            let Some(interval) = interval_of(root) else {
                continue;
            };
            keep.insert(root.id.clone());
            match armed.get(&root.id) {
                Some((old, _)) if *old == interval => {}
                _ => {
                    armed.insert(root.id.clone(), (interval, now));
                }
            }
        }
        armed.retain(|id, _| keep.contains(id));
    }

    fn armed_at(&self, id: &str) -> Option<i64> {
        self.armed
            .lock()
            .expect("lock poisoned")
            .get(id)
            .map(|(_, at)| *at)
    }
}

/// When the next automatic scan of `root` is due, in Unix milliseconds.
/// It counts from the end of the last scan, of any kind and outcome, or
/// for a root never scanned, from when automatic scans were turned on.
pub(crate) async fn next_scan_at(
    db: &Db,
    automation: &Automation,
    root: &RootEntry,
) -> Result<Option<i64>> {
    let Some(interval) = interval_of(root) else {
        return Ok(None);
    };
    let base = db::roots::last_scan_end_at(db, &root.id)
        .await?
        .or_else(|| automation.armed_at(&root.id))
        .unwrap_or_else(db::now_ms);
    Ok(Some(base + i64::from(interval) * 1000))
}

/// Queues an ordinary scan of a root, the same task `POST /api/admin/scan`
/// queues. Returns the task id, or `None` when the root is gone or has no
/// storage. A scan already queued or running is returned as it is.
pub(crate) async fn queue_scan(state: &AppState, root_id: &str) -> Result<Option<i64>> {
    let state = state.clone();
    let root_id = root_id.to_owned();
    // On a task of its own, like the scan handler, so the root-change lock
    // is held until the task row is written.
    tokio::spawn(async move {
        let _changes = state.lock_root_changes().await;
        let Some(root) = state.roots().into_iter().find(|r| r.id == root_id) else {
            return Ok(None);
        };
        let Some(storage) = state.storage(&root_id) else {
            return Ok(None);
        };
        let env = state.scan_env().with_full(false);
        let submitted = state
            .scheduler()
            .submit(
                scan::spec(&root_id, false),
                scan::job(env, root_id.clone(), storage, root.kind),
            )
            .await?;
        Ok(Some(submitted.id))
    })
    .await?
}

/// The scheduler's memory between ticks.
#[derive(Debug, Default)]
pub(crate) struct Schedule {
    /// When this process last queued or tried to queue a root's scan,
    /// plus its interval. The task's own end time takes over once it is
    /// recorded; this keeps a root from being queued again before then.
    /// Keyed by root, with the interval it was set for: a new interval
    /// drops it.
    not_before: HashMap<String, (u32, i64)>,
}

impl Schedule {
    /// Queues every root that is due at `now` and returns how long to wait
    /// before the next tick.
    pub(crate) async fn tick(&mut self, state: &AppState, now: i64) -> Duration {
        let roots = state.roots();
        let automation = state.automation();
        automation.arm(&roots, now);
        self.not_before.retain(|id, (interval, _)| {
            roots
                .iter()
                .any(|r| &r.id == id && interval_of(r) == Some(*interval))
        });
        let mut wait = RECHECK;
        for root in &roots {
            let Some(interval) = interval_of(root) else {
                continue;
            };
            let next = match next_scan_at(state.db(), automation, root).await {
                Ok(Some(next)) => next,
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!(root = %root.id, "cannot work out the next automatic scan: {e:#}");
                    continue;
                }
            };
            let next = next.max(
                self.not_before
                    .get(&root.id)
                    .map_or(i64::MIN, |(_, at)| *at),
            );
            if next > now {
                let until = Duration::from_millis(u64::try_from(next - now).unwrap_or(0));
                wait = wait.min(until);
                continue;
            }
            // Skipped while a scan of this root is queued or running; its
            // end time then sets the next run.
            match db::tasks::active_id(state.db(), &scan::dedupe_key(&root.id)).await {
                Ok(Some(_)) => continue,
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(root = %root.id, "cannot check for a running scan: {e:#}");
                    continue;
                }
            }
            self.not_before.insert(
                root.id.clone(),
                (interval, now + i64::from(interval) * 1000),
            );
            match queue_scan(state, &root.id).await {
                Ok(Some(task)) => tracing::info!(root = %root.id, task, "queued an automatic scan"),
                Ok(None) => {
                    tracing::warn!(root = %root.id, "automatic scan skipped, the root has no storage")
                }
                Err(e) => tracing::warn!(root = %root.id, "cannot queue an automatic scan: {e:#}"),
            }
        }
        wait.max(Duration::from_secs(1))
    }
}

/// Runs the scheduler until `stop` is cancelled. Times compared with the
/// stored scan times come from the wall clock, which keeps counting
/// through a suspend; only the sleeps use tokio's clock.
pub(crate) async fn run(state: AppState, stop: CancellationToken) {
    let mut changed = state.automation().subscribe();
    let mut schedule = Schedule::default();
    loop {
        // A tick can wait on the root-change lock, which a root edit holds
        // while a backup holds the secrets lock, so shutdown does not wait
        // for it. A scan it already handed to the scheduler is the
        // scheduler's to stop.
        let wait = tokio::select! {
            () = stop.cancelled() => return,
            wait = schedule.tick(&state, db::now_ms()) => wait,
        };
        tokio::select! {
            () = stop.cancelled() => return,
            () = tokio::time::sleep(wait) => {}
            // Roots were added or edited: their schedule may have moved.
            _ = changed.changed() => {}
        }
    }
}

/// An app state with one local root `lib` for the scheduler and watcher
/// tests.
#[cfg(test)]
pub(crate) async fn test_state(dir: &std::path::Path, settings: RootSettings) -> AppState {
    use neo_core::dto::RootSource;

    let data_dir = crate::datadir::DataDir::prepare(&dir.join("data")).unwrap();
    let db = Db::open_sqlite(&data_dir.database()).await.unwrap();
    db::roots::sync(&db, &[("lib", "Lib")]).await.unwrap();
    let library = dir.join("library");
    std::fs::create_dir_all(library.join("RJ000001")).unwrap();
    AppState::new(
        db,
        crate::token::Tokens::new(b"test signing key, 32 bytes long!"),
        vec![],
    )
    .with_data_dir(data_dir)
    .with_roots(vec![RootEntry {
        id: "lib".into(),
        kind: RootKind::Local,
        source: RootSource::Config,
        settings,
    }])
    .with_storages(vec![(
        "lib".into(),
        std::sync::Arc::new(neo_storage::LocalStorage::new(library)) as _,
    )])
}

/// Scan tasks of `lib`, oldest first, as (id, state).
#[cfg(test)]
pub(crate) async fn scan_tasks(state: &AppState) -> Vec<(i64, String)> {
    sqlx::query_as("SELECT id, state FROM tasks WHERE dedupe_key = 'scan:lib' ORDER BY id")
        .fetch_all(&state.db().sqlite().unwrap().read)
        .await
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every(secs: u32) -> RootSettings {
        RootSettings::Local {
            path: "/unused".into(),
            watch: false,
            auto_scan_interval: Some(secs),
        }
    }

    /// Waits, at most 10 s, until no scan of `lib` is queued or running.
    async fn settled(state: &AppState) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while db::tasks::active_id(state.db(), "scan:lib")
            .await
            .unwrap()
            .is_some()
        {
            assert!(std::time::Instant::now() < deadline, "the scan never ended");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[test]
    fn intervals_have_a_floor_per_kind() {
        assert!(check_interval(RootKind::Local, 300).is_ok());
        assert!(check_interval(RootKind::Local, 299).is_err());
        assert!(check_interval(RootKind::OpenList, 1799).is_err());
        assert!(check_interval(RootKind::OpenList, 1800).is_ok());
        assert!(check_interval(RootKind::OpenList, MAX_INTERVAL + 1).is_err());
    }

    #[tokio::test]
    async fn a_new_root_is_scanned_one_interval_after_it_was_turned_on() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(dir.path(), every(300)).await;
        let mut schedule = Schedule::default();
        let t0 = db::now_ms();
        let wait = schedule.tick(&state, t0).await;
        assert!(wait <= Duration::from_secs(60), "{wait:?}");
        assert!(scan_tasks(&state).await.is_empty());
        schedule.tick(&state, t0 + 299_000).await;
        assert!(scan_tasks(&state).await.is_empty());

        schedule.tick(&state, t0 + 300_000).await;
        assert_eq!(scan_tasks(&state).await.len(), 1);
        // Due again at once by the task's end time, but not before this
        // process's own interval has passed.
        schedule.tick(&state, t0 + 300_001).await;
        settled(&state).await;
        schedule.tick(&state, t0 + 599_000).await;
        assert_eq!(scan_tasks(&state).await.len(), 1);
        schedule.tick(&state, t0 + 600_000).await;
        assert_eq!(scan_tasks(&state).await.len(), 2);
        settled(&state).await;
    }

    #[tokio::test]
    async fn a_shorter_interval_applies_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(dir.path(), every(MAX_INTERVAL)).await;
        let mut schedule = Schedule::default();
        let t0 = db::now_ms();
        schedule.tick(&state, t0).await;
        schedule
            .tick(&state, t0 + i64::from(MAX_INTERVAL) * 1000)
            .await;
        assert_eq!(scan_tasks(&state).await.len(), 1);
        settled(&state).await;
        let mut root = state.roots().remove(0);
        root.settings = every(300);
        let storage = state.storage("lib");
        state.put_root(root, storage);
        // Ten minutes after the scan, which the old interval would have
        // held off for 30 days.
        let ended = db::roots::last_scan_end_at(state.db(), "lib")
            .await
            .unwrap()
            .unwrap();
        schedule
            .tick(
                &state,
                (t0 + i64::from(MAX_INTERVAL) * 1000).max(ended) + 600_000,
            )
            .await;
        assert_eq!(scan_tasks(&state).await.len(), 2);
        settled(&state).await;
    }

    #[tokio::test]
    async fn a_running_scan_skips_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(dir.path(), every(300)).await;
        sqlx::query(
            "INSERT INTO tasks (kind, dedupe_key, state, payload, created_at, updated_at)
             VALUES ('scan', 'scan:lib', 'running', '{}', 0, 0)",
        )
        .execute(&state.db().sqlite().unwrap().write)
        .await
        .unwrap();
        let mut schedule = Schedule::default();
        let t0 = db::now_ms();
        schedule.tick(&state, t0 + 10 * 300_000).await;
        assert_eq!(scan_tasks(&state).await.len(), 1, "only the running one");
    }

    #[tokio::test]
    async fn a_restart_counts_from_the_last_scan() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(dir.path(), every(300)).await;
        let t0 = db::now_ms();
        // A failed scan ended 200 s before this process started. Its task
        // may be pruned; the time is kept on the root.
        sqlx::query("UPDATE roots SET last_scan_end_at = $1 WHERE id = 'lib'")
            .bind(t0 - 200_000)
            .execute(&state.db().sqlite().unwrap().write)
            .await
            .unwrap();
        let mut schedule = Schedule::default();
        let wait = schedule.tick(&state, t0).await;
        assert_eq!(wait, Duration::from_secs(60), "capped at the recheck");
        assert!(scan_tasks(&state).await.is_empty());
        let root = state.roots().remove(0);
        let next = next_scan_at(state.db(), state.automation(), &root)
            .await
            .unwrap();
        assert_eq!(next, Some(t0 + 100_000));
        schedule.tick(&state, t0 + 100_000).await;
        assert_eq!(scan_tasks(&state).await.len(), 1);
        settled(&state).await;
    }

    #[tokio::test]
    async fn an_overdue_root_is_scanned_at_startup() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(dir.path(), every(300)).await;
        let t0 = db::now_ms();
        sqlx::query("UPDATE roots SET last_scan_at = $1 WHERE id = 'lib'")
            .bind(t0 - 400_000)
            .execute(&state.db().sqlite().unwrap().write)
            .await
            .unwrap();
        Schedule::default().tick(&state, t0).await;
        assert_eq!(scan_tasks(&state).await.len(), 1);
        settled(&state).await;
    }

    #[tokio::test]
    async fn roots_without_an_interval_are_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(
            dir.path(),
            RootSettings::Local {
                path: "/unused".into(),
                watch: false,
                auto_scan_interval: None,
            },
        )
        .await;
        let t0 = db::now_ms();
        Schedule::default().tick(&state, t0 + 1_000_000_000).await;
        assert!(scan_tasks(&state).await.is_empty());
        let root = state.roots().remove(0);
        let next = next_scan_at(state.db(), state.automation(), &root)
            .await
            .unwrap();
        assert_eq!(next, None);
    }
}
