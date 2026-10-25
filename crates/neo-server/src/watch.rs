//! Watching local roots for file changes and queueing a scan once they
//! settle (book/src/contract.md, "自动扫描和目录监听").

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use neo_core::dto::{RootKind, RootSettings};
use neo_db as db;
use notify::{
    Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
    event::{ModifyKind, RemoveKind},
};
use tokio::{sync::mpsc, task::JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::{autoscan, http::AppState, scan};

/// How long a root must stay quiet before its scan is queued, so copying a
/// whole work queues one scan instead of one per file.
pub(crate) const QUIET: Duration = Duration::from_secs(10);
/// How often a running scan is checked for having ended.
const RUNNING_RECHECK: Duration = Duration::from_secs(2);
/// Events only say that something changed, so a full channel loses
/// nothing that matters.
const EVENT_BUFFER: usize = 64;

/// How often roots whose watcher could not be set up are tried again, for
/// example a root directory that was removed and comes back.
const RETRY: Duration = Duration::from_secs(60);

struct Watched {
    path: PathBuf,
    // Dropping the watcher stops its thread and closes the channel.
    _watcher: RecommendedWatcher,
    debounce: JoinHandle<()>,
}

impl Drop for Watched {
    fn drop(&mut self) {
        self.debounce.abort();
    }
}

/// Keeps one watcher per local root with `watch` on, in step with the
/// registry, until `stop` is cancelled.
pub(crate) async fn run(state: AppState, stop: CancellationToken) {
    run_with(state, stop, QUIET).await;
}

async fn run_with(state: AppState, stop: CancellationToken, quiet: Duration) {
    let mut changed = state.automation().subscribe();
    let mut watchers: HashMap<String, Watched> = HashMap::new();
    let mut failing = HashSet::new();
    loop {
        reconcile(&state, &mut watchers, &mut failing, quiet);
        tokio::select! {
            () = stop.cancelled() => break,
            _ = changed.changed() => {}
            () = tokio::time::sleep(RETRY) => {}
        }
    }
    for id in watchers.keys() {
        state.automation().set_watching(id, false);
    }
}

/// The directory of each root that should be watched now.
fn wanted(state: &AppState) -> HashMap<String, PathBuf> {
    state
        .roots()
        .into_iter()
        .filter(|r| r.kind == RootKind::Local && state.storage(&r.id).is_some())
        .filter_map(|r| match r.settings {
            RootSettings::Local {
                path, watch: true, ..
            } => Some((r.id, PathBuf::from(path))),
            _ => None,
        })
        .collect()
}

/// `failing` holds the roots whose watcher lost events or could not be set
/// up. A retry that fails again is not logged every minute, and the watcher
/// that finally comes up scans the root, because changes went unseen.
fn reconcile(
    state: &AppState,
    watchers: &mut HashMap<String, Watched>,
    failing: &mut HashSet<String>,
    quiet: Duration,
) {
    let wanted = wanted(state);
    failing.retain(|id| wanted.contains_key(id));
    watchers.retain(|id, watched| {
        if wanted.get(id) != Some(&watched.path) {
            state.automation().set_watching(id, false);
            return false;
        }
        // Marked unhealthy by its debounce task: events were lost, so the
        // watches are set up again and the root is scanned.
        if !state.automation().is_watching(id) {
            failing.insert(id.clone());
            return false;
        }
        true
    });
    for (id, path) in wanted {
        if watchers.contains_key(&id) {
            continue;
        }
        let rescan = failing.contains(&id);
        let first_failure = !rescan;
        match start(state, &id, &path, quiet, rescan) {
            Ok(watched) => {
                tracing::info!(root = %id, "watching for file changes");
                state.automation().set_watching(&id, true);
                failing.remove(&id);
                watchers.insert(id, watched);
            }
            Err(e) => {
                // Scans on the interval, if any, still run, and the watcher
                // is tried again after RETRY.
                if first_failure {
                    tracing::warn!(root = %id, "cannot watch for file changes: {e}");
                }
                failing.insert(id.clone());
                state.automation().set_watching(&id, false);
            }
        }
    }
}

/// Watches `path`. With `rescan`, the root is scanned once it is quiet even
/// without a new change, because the watcher it replaces lost events.
fn start(
    state: &AppState,
    id: &str,
    path: &Path,
    quiet: Duration,
    rescan: bool,
) -> notify::Result<Watched> {
    let (tx, rx) = mpsc::channel(EVENT_BUFFER);
    if rescan {
        let _ = tx.try_send(());
    }
    // Sticky, because a full channel may drop the wake-up that carried it.
    let failed = Arc::new(AtomicBool::new(false));
    let root = path.to_owned();
    let watcher_id = id.to_owned();
    let flag = Arc::clone(&failed);
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
        let failure = match event {
            // A queue overflow: events, and with them watches on new
            // directories, were lost.
            Ok(event) if event.need_rescan() => Some("events were lost".to_owned()),
            // The kernel drops the watch with the directory, and a new
            // directory at the same path is not watched.
            Ok(event) if is_root_gone(&event, &root) => {
                Some("the root directory was removed or renamed".to_owned())
            }
            Ok(event) if is_change(&event.kind) => None,
            Ok(_) => return,
            Err(e) => Some(e.to_string()),
        };
        if let Some(reason) = failure {
            tracing::warn!(root = %watcher_id, "file watching failed: {reason}");
            flag.store(true, Ordering::Relaxed);
        }
        let _ = tx.try_send(());
    })?;
    watcher.watch(path, RecursiveMode::Recursive)?;
    Ok(Watched {
        path: path.to_owned(),
        _watcher: watcher,
        debounce: tokio::spawn(debounce(state.clone(), id.to_owned(), rx, failed, quiet)),
    })
}

fn is_root_gone(event: &Event, root: &Path) -> bool {
    matches!(
        event.kind,
        EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_))
    ) && event.paths.iter().any(|p| p == root)
}

/// Whether an event can change what a scan finds. Reads, which a scan and
/// ffprobe make themselves, and metadata changes such as access times are
/// left out, or every scan would queue the next one.
fn is_change(kind: &EventKind) -> bool {
    match kind {
        EventKind::Create(_) => true,
        EventKind::Remove(kind) => !matches!(kind, RemoveKind::Other),
        EventKind::Modify(kind) => !matches!(kind, ModifyKind::Metadata(_) | ModifyKind::Other),
        EventKind::Access(_) | EventKind::Other => false,
        EventKind::Any => true,
    }
}

/// Waits for a change, then for `quiet` without another, then queues a
/// scan of the root. After a watcher failure the task ends once the root is
/// quiet and marks it unhealthy, so [`reconcile`] replaces the watcher and
/// the replacement scans.
async fn debounce(
    state: AppState,
    id: String,
    mut rx: mpsc::Receiver<()>,
    failed: Arc<AtomicBool>,
    quiet: Duration,
) {
    loop {
        if rx.recv().await.is_none() {
            return;
        }
        loop {
            loop {
                match tokio::time::timeout(quiet, rx.recv()).await {
                    Err(_) => break,
                    Ok(None) => return,
                    Ok(Some(())) => {}
                }
            }
            if failed.load(Ordering::Relaxed) {
                state.automation().set_watching(&id, false);
                state.automation().roots_changed();
                return;
            }
            // A scan already walking the root may have passed the changed
            // directory, so the new one waits for it to end. A queued scan
            // has not started and will see the change. Changes during the
            // wait start the quiet period again.
            let mut changed_again = false;
            loop {
                match db::tasks::active_state(state.db(), &scan::dedupe_key(&id)).await {
                    Ok(Some(task_state)) if task_state == "running" => {}
                    _ => break,
                }
                match tokio::time::timeout(RUNNING_RECHECK, rx.recv()).await {
                    Err(_) => {}
                    Ok(None) => return,
                    Ok(Some(())) => {
                        changed_again = true;
                        break;
                    }
                }
            }
            if !changed_again {
                break;
            }
        }
        match autoscan::queue_scan(&state, &id).await {
            Ok(Some(task)) => tracing::info!(root = %id, task, "files changed, queued a scan"),
            Ok(None) => {}
            Err(e) => tracing::warn!(root = %id, "cannot queue a scan after file changes: {e:#}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use notify::event::{AccessKind, CreateKind, DataChange, MetadataKind, RenameMode};

    use super::*;

    /// Polls `condition` every 20 ms, failing after 10 s.
    async fn eventually(what: &str, mut condition: impl AsyncFnMut() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !condition().await {
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {what}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn watched(path: &Path, watch: bool) -> RootSettings {
        RootSettings::Local {
            path: path.display().to_string(),
            watch,
            auto_scan_interval: None,
        }
    }

    #[tokio::test]
    async fn a_new_file_queues_a_scan_once_the_root_is_quiet() {
        let dir = tempfile::tempdir().unwrap();
        let library = dir.path().join("library");
        let state = autoscan::test_state(dir.path(), watched(&library, true)).await;
        let stop = CancellationToken::new();
        let task = tokio::spawn(run_with(
            state.clone(),
            stop.clone(),
            Duration::from_millis(200),
        ));
        eventually("the watcher", async || {
            state.automation().is_watching("lib")
        })
        .await;
        assert!(autoscan::scan_tasks(&state).await.is_empty());

        std::fs::write(library.join("RJ000001/01.mp3"), b"audio").unwrap();
        eventually("a scan", async || {
            !autoscan::scan_tasks(&state).await.is_empty()
        })
        .await;

        stop.cancel();
        task.await.unwrap();
        assert!(!state.automation().is_watching("lib"));
    }

    #[tokio::test]
    async fn a_watcher_that_lost_events_is_replaced_and_the_root_scanned() {
        let dir = tempfile::tempdir().unwrap();
        let library = dir.path().join("library");
        let state = autoscan::test_state(dir.path(), watched(&library, true)).await;
        let stop = CancellationToken::new();
        let task = tokio::spawn(run_with(
            state.clone(),
            stop.clone(),
            Duration::from_millis(200),
        ));
        eventually("the watcher", async || {
            state.automation().is_watching("lib")
        })
        .await;
        // What the debounce task does after a watcher error.
        state.automation().set_watching("lib", false);
        state.automation().roots_changed();
        eventually("a scan", async || {
            !autoscan::scan_tasks(&state).await.is_empty()
        })
        .await;
        assert!(state.automation().is_watching("lib"));
        stop.cancel();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn removing_the_root_directory_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let library = dir.path().join("library");
        let state = autoscan::test_state(dir.path(), watched(&library, true)).await;
        let stop = CancellationToken::new();
        let task = tokio::spawn(run_with(
            state.clone(),
            stop.clone(),
            Duration::from_millis(200),
        ));
        eventually("the watcher", async || {
            state.automation().is_watching("lib")
        })
        .await;
        std::fs::remove_dir_all(&library).unwrap();
        eventually("the watcher to report the loss", async || {
            !state.automation().is_watching("lib")
        })
        .await;
        stop.cancel();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn turning_watch_off_stops_the_watcher() {
        let dir = tempfile::tempdir().unwrap();
        let library = dir.path().join("library");
        let state = autoscan::test_state(dir.path(), watched(&library, true)).await;
        let stop = CancellationToken::new();
        let task = tokio::spawn(run_with(state.clone(), stop.clone(), QUIET));
        eventually("the watcher", async || {
            state.automation().is_watching("lib")
        })
        .await;
        let mut root = state.roots().remove(0);
        root.settings = watched(&library, false);
        let storage = state.storage("lib");
        state.put_root(root, storage);
        eventually("the watcher to stop", async || {
            !state.automation().is_watching("lib")
        })
        .await;
        stop.cancel();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_directory_that_cannot_be_watched_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let library = dir.path().join("library");
        let state =
            autoscan::test_state(dir.path(), watched(&dir.path().join("absent"), true)).await;
        let stop = CancellationToken::new();
        let task = tokio::spawn(run_with(state.clone(), stop.clone(), QUIET));
        // The failed root does not stop the loop: fixing its path starts it.
        let mut root = state.roots().remove(0);
        root.settings = watched(&library, true);
        let storage = state.storage("lib");
        state.put_root(root, storage);
        eventually("the watcher", async || {
            state.automation().is_watching("lib")
        })
        .await;
        stop.cancel();
        task.await.unwrap();
    }

    #[test]
    fn reads_and_metadata_changes_do_not_count() {
        assert!(is_change(&EventKind::Create(CreateKind::File)));
        assert!(is_change(&EventKind::Remove(RemoveKind::Folder)));
        assert!(is_change(&EventKind::Modify(ModifyKind::Data(
            DataChange::Content
        ))));
        assert!(is_change(&EventKind::Modify(ModifyKind::Name(
            RenameMode::Both
        ))));
        assert!(!is_change(&EventKind::Access(AccessKind::Read)));
        assert!(!is_change(&EventKind::Modify(ModifyKind::Metadata(
            MetadataKind::AccessTime
        ))));
    }
}
