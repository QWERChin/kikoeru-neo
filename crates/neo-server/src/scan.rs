//! Scan tasks: `neo_scanner::scan_root` run as a scheduler job, from
//! `POST /api/admin/scan` or from a queued task after a restart.

use std::{collections::HashSet, sync::Arc, time::Duration};

use futures_util::{FutureExt, future::BoxFuture};
use neo_core::dto::RootKind;
use neo_db::Db;
use neo_scanner::{
    Observer, Root, ScanError, Stage, Summary,
    metadata::{
        CoverDirs, FolderCoverSummary, MetadataProvider, MetadataSummary, fetch_missing_except,
        fill_from_folders,
    },
    scan_root,
};
use neo_storage::Storage;
use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use crate::tasks::{Job, JobContext, JobError, Lane, TaskSpec};

pub const KIND: &str = "scan";

/// How the progress bar is split between the stages. Syncing tracks takes
/// most of a scan; metadata, at one second per new work, can be long too.
/// Folder covers are read only for works that still have none.
const DISCOVER_SHARE: f64 = 0.1;
const TRACKS_SHARE: f64 = 0.7;
const METADATA_SHARE: f64 = 0.15;
const COVERS_SHARE: f64 = 0.05;

/// What a scan task needs besides the root ID, which comes from the request
/// or the stored payload.
/// Queues a background ffprobe for one track.
pub type ProbeQueue = Arc<dyn Fn(i64) -> BoxFuture<'static, ()> + Send + Sync>;

#[derive(Clone)]
pub struct ScanEnv {
    pub db: Db,
    /// Where probe IDs go. Without it a scan queues no probes, and tracks
    /// are probed on first play.
    pub probes: Option<ProbeQueue>,
    /// Root IDs from most to least preferred: local roots first, then the
    /// rest, each in config order. It decides which copy of a file in
    /// several locations is kept.
    pub order: Vec<String>,
    /// Walk every work even when its top-level listing is unchanged.
    pub full: bool,
    /// Fetches metadata for new works after the tracks. Without it new
    /// works keep their directory names until a refresh.
    pub metadata: Option<MetadataEnv>,
    /// Where covers taken from the works' own folders go. Without it works
    /// that no source gave a cover keep the placeholder.
    pub covers: Option<CoverDirs>,
}

/// The metadata source and where its covers go.
#[derive(Clone)]
pub struct MetadataEnv {
    pub provider: Arc<dyn MetadataProvider>,
    pub dirs: CoverDirs,
}

impl ScanEnv {
    pub fn new(db: Db, roots: &[(String, RootKind)]) -> Self {
        Self {
            db,
            probes: None,
            order: root_order(roots),
            full: false,
            metadata: None,
            covers: None,
        }
    }

    pub fn with_metadata(mut self, metadata: MetadataEnv) -> Self {
        self.metadata = Some(metadata);
        self
    }

    pub fn with_covers(mut self, covers: Option<CoverDirs>) -> Self {
        self.covers = covers;
        self
    }

    pub fn with_probes(mut self, probes: ProbeQueue) -> Self {
        self.probes = Some(probes);
        self
    }

    pub fn with_full(mut self, full: bool) -> Self {
        self.full = full;
        self
    }
}

pub(crate) fn root_order(roots: &[(String, RootKind)]) -> Vec<String> {
    let local = roots.iter().filter(|(_, k)| *k == RootKind::Local);
    let remote = roots.iter().filter(|(_, k)| *k != RootKind::Local);
    local.chain(remote).map(|(id, _)| id.clone()).collect()
}

/// One active scan per root: a second request gets the running task.
pub fn spec(root_id: &str, full: bool) -> TaskSpec {
    TaskSpec {
        kind: KIND,
        dedupe_key: Some(dedupe_key(root_id)),
        payload: json!({ "root": root_id, "full": full }),
        lane: Lane::Scan,
        timeout: TIMEOUT,
        queue_limit: None,
    }
}

pub fn dedupe_key(root_id: &str) -> String {
    format!("scan:{root_id}")
}

/// The root ID a stored scan task was created for.
pub fn payload_root(payload: &str) -> Option<String> {
    serde_json::from_str::<Value>(payload)
        .ok()?
        .get("root")?
        .as_str()
        .map(str::to_owned)
}

/// Whether a stored scan task was a full scan. Tasks stored before the flag
/// existed were not.
pub fn payload_full(payload: &str) -> bool {
    serde_json::from_str::<Value>(payload)
        .ok()
        .and_then(|v| v.get("full")?.as_bool())
        .unwrap_or(false)
}

/// The job for one root.
pub fn job(env: ScanEnv, root_id: String, storage: Arc<dyn Storage>, kind: RootKind) -> Job {
    Box::new(move |ctx: JobContext| {
        async move {
            // Probes found while scanning are submitted next to the scan in
            // the same job, so the scheduler waits for every submission
            // before a shutdown closes the database.
            let (probe_tx, mut probe_rx) = mpsc::unbounded_channel::<Vec<i64>>();
            let submit = async {
                while let Some(ids) = probe_rx.recv().await {
                    if let Some(queue) = &env.probes {
                        for id in ids {
                            queue(id).await;
                        }
                    }
                }
            };
            // Set once the scan reaches its track stage, by which time every
            // work it found is in the database.
            let (works_ready, mut ready) = watch::channel(false);
            let observer = TaskObserver {
                cancel: &ctx.cancel,
                progress: ctx.progress(),
                probes: probe_tx,
                works_ready,
                message: MessageWriter {
                    db: env.db.clone(),
                    task_id: ctx.task_id,
                    last: std::sync::Mutex::new(None),
                },
            };
            let root = Root {
                id: &root_id,
                storage: storage.as_ref(),
                // OpenList tracks are probed on first play, because each
                // probe needs a direct link from the provider.
                probe_after_scan: kind == RootKind::Local,
                // Local listings are cheap, and a local directory's time
                // does not change when a file deeper inside it does.
                skip_unchanged: kind != RootKind::Local && !env.full,
            };
            let now = neo_db::now_ms();
            let cancel = ctx.cancel.clone();
            let db = env.db.clone();
            let scan = async move {
                let listed = CancellationToken::new();
                // The early metadata pass's latest count, shown as soon as
                // the listing ends rather than at its next work, which may
                // be a slow DLsite answer away.
                let early_count = std::sync::Mutex::new(None);
                let scanning = async {
                    let scanned = scan_root(&env.db, &root, &env.order, &observer, now).await;
                    listed.cancel();
                    let count = *early_count.lock().expect("count mutex poisoned");
                    if scanned.is_ok()
                        && let Some((done, total)) = count
                    {
                        observer.progress(Stage::Metadata, done, total);
                    }
                    scanned
                };
                // DLsite and the storage have limits of their own, so new
                // works get their metadata while their tracks are still
                // being listed, instead of after the whole listing. The
                // progress bar stays with the listing meanwhile.
                let mut tried = HashSet::new();
                let early = async {
                    let meta = env.metadata.as_ref()?;
                    tokio::select! {
                        _ = ready.wait_for(|ready| *ready) => {}
                        () = listed.cancelled() => return None,
                    }
                    let quiet = Quiet {
                        cancel: &cancel,
                        listed: &listed,
                        task: &observer,
                        count: &early_count,
                    };
                    tokio::select! {
                        fetched = fetch_missing_except(
                            &env.db,
                            meta.provider.as_ref(),
                            &meta.dirs,
                            root.id,
                            &quiet,
                            neo_db::now_ms,
                            &mut tried,
                        ) => Some(fetched),
                        () = cancel.cancelled() => None,
                    }
                };
                let (scanned, early) = tokio::join!(scanning, early);
                let outcome = async {
                    let summary = scanned?;
                    let metadata = match &env.metadata {
                        Some(meta) => {
                            let mut fetched = match early {
                                Some(early) => early.map_err(ScanError::Db)?,
                                None => MetadataSummary::default(),
                            };
                            // The rest: works the early pass did not reach.
                            // With DLsite unreachable a second pass would
                            // only wait out its retries again.
                            if fetched.unavailable == 0 && !observer.is_cancelled() {
                                // Raced against cancel: a fetch can wait on a
                                // backup's covers lock for a long time. A
                                // cover already being published finishes on
                                // its own task.
                                let rest = tokio::select! {
                                    rest = fetch_missing_except(
                                        &env.db,
                                        meta.provider.as_ref(),
                                        &meta.dirs,
                                        root.id,
                                        &observer,
                                        neo_db::now_ms,
                                        &mut tried,
                                    ) => rest.map_err(ScanError::Db)?,
                                    () = cancel.cancelled() => return Err(ScanError::Cancelled),
                                };
                                fetched.fetched += rest.fetched;
                                fetched.not_found += rest.not_found;
                                fetched.unavailable += rest.unavailable;
                            }
                            Some(fetched)
                        }
                        None => None,
                    };
                    // After the metadata, so only works that no source gave
                    // a cover cost a read from the storage.
                    let covers = match &env.covers {
                        Some(dirs) if !observer.is_cancelled() => Some(tokio::select! {
                            covers = fill_from_folders(
                                &env.db,
                                root.storage,
                                dirs,
                                root.id,
                                &observer,
                                neo_db::now_ms,
                            ) => covers.map_err(ScanError::Db)?,
                            () = cancel.cancelled() => return Err(ScanError::Cancelled),
                        }),
                        _ => None,
                    };
                    if observer.is_cancelled() {
                        return Err(ScanError::Cancelled);
                    }
                    Ok((summary, metadata, covers))
                }
                .await;
                // Closes the channel, so the submitter ends once it drains.
                drop(observer);
                outcome
            };
            let (outcome, ()) = tokio::join!(scan, submit);
            // Failed and cancelled scans count too: the next automatic scan
            // is due one interval after any scan.
            if let Err(e) = neo_db::roots::record_scan_end(&db, &root_id, neo_db::now_ms()).await {
                tracing::warn!(root = %root_id, "cannot record when the scan ended: {e}");
            }
            match outcome {
                Ok((summary, metadata, covers)) => {
                    Ok(Some(result(&summary, metadata.as_ref(), covers.as_ref())))
                }
                Err(ScanError::Cancelled) => Err(JobError("扫描已取消".to_owned())),
                Err(ScanError::Unavailable(e)) => {
                    Err(JobError(format!("媒体库 {root_id} 暂时无法访问：{e}")))
                }
                Err(ScanError::UnknownRoot(_)) => {
                    Err(JobError(format!("媒体库 {root_id} 不在配置里")))
                }
                Err(ScanError::Db(e)) => {
                    tracing::error!(root = %root_id, error = %e, "scan failed");
                    Err(JobError("扫描时数据库出错".to_owned()))
                }
            }
        }
        .boxed()
    })
}

/// The task result: the counters and the scan log, for the settings page.
fn result(
    s: &Summary,
    metadata: Option<&MetadataSummary>,
    covers: Option<&FolderCoverSummary>,
) -> Value {
    let metadata = metadata.cloned().unwrap_or_default();
    let covers = covers.cloned().unwrap_or_default();
    json!({
        "metadata_fetched": metadata.fetched,
        "metadata_not_found": metadata.not_found,
        "metadata_unavailable": metadata.unavailable,
        "folder_covers_stored": covers.stored,
        "folder_covers_failed": covers.failed,
        "folder_covers_unavailable": covers.unavailable,
        "works_found": s.works_found,
        "works_new": s.works_new,
        "works_missing": s.works_missing,
        "works_restored": s.works_restored,
        "works_failed": s.works_failed,
        "works_unchanged": s.works_unchanged,
        "locations_new": s.locations_new,
        "locations_missing": s.locations_missing,
        "locations_restored": s.locations_restored,
        "tracks_new": s.tracks_new,
        "tracks_changed": s.tracks_changed,
        "tracks_missing": s.tracks_missing,
        "tracks_restored": s.tracks_restored,
        "pending_matched": s.pending_matched,
        "pending_ambiguous": s.pending_ambiguous,
        "pending_superseded": s.pending_superseded,
        "pending_not_found": s.pending_not_found,
        "pending_deferred": s.pending_deferred,
        "probe_needed": s.probe.len(),
        "log": s.log,
    })
}

struct TaskObserver<'a> {
    cancel: &'a CancellationToken,
    progress: &'a watch::Sender<f64>,
    probes: mpsc::UnboundedSender<Vec<i64>>,
    works_ready: watch::Sender<bool>,
    /// The stage line shown as the task message, written at most once a
    /// second, like progress.
    message: MessageWriter,
}

struct MessageWriter {
    db: Db,
    task_id: i64,
    last: std::sync::Mutex<Option<(Stage, std::time::Instant)>>,
}

impl MessageWriter {
    fn update(&self, stage: Stage, done: u64, total: u64) {
        let now = std::time::Instant::now();
        {
            let mut last = self.last.lock().expect("message mutex poisoned");
            // A new stage always shows at once; counts within a stage are
            // throttled, except the final one.
            if let Some((prev, at)) = *last
                && prev == stage
                && done < total
                && now.duration_since(at) < Duration::from_secs(1)
            {
                return;
            }
            *last = Some((stage, now));
        }
        let text = match stage {
            Stage::Discover => "发现作品".to_owned(),
            Stage::Tracks => format!("同步轨道 {done}/{total}"),
            Stage::Metadata => format!("抓取元数据 {done}/{total}"),
            Stage::Covers => format!("读取封面 {done}/{total}"),
        };
        let db = self.db.clone();
        let id = self.task_id;
        // The observer is synchronous; the write is best effort.
        tokio::spawn(async move {
            let _ = neo_db::tasks::set_message(&db, id, &text, neo_db::now_ms()).await;
        });
    }
}

impl Observer for TaskObserver<'_> {
    fn progress(&self, stage: Stage, done: u64, total: u64) {
        let fraction = if total == 0 {
            1.0
        } else {
            done as f64 / total as f64
        };
        let fraction = fraction.min(1.0);
        let value = match stage {
            Stage::Discover => DISCOVER_SHARE * fraction,
            Stage::Tracks => DISCOVER_SHARE + TRACKS_SHARE * fraction,
            Stage::Metadata => DISCOVER_SHARE + TRACKS_SHARE + METADATA_SHARE * fraction,
            Stage::Covers => {
                DISCOVER_SHARE + TRACKS_SHARE + METADATA_SHARE + COVERS_SHARE * fraction
            }
        };
        self.progress.send_replace(value);
        self.message.update(stage, done, total);
        if stage == Stage::Tracks {
            self.works_ready
                .send_if_modified(|ready| !std::mem::replace(ready, true));
        }
    }

    fn probe(&self, track_ids: &[i64]) {
        // The receiver lives as long as the job, so a send cannot fail.
        let _ = self.probes.send(track_ids.to_vec());
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
}

/// What the metadata pass that starts during the listing reports to. It
/// follows cancellation. The listing keeps the progress bar while it runs,
/// and the metadata pass takes it over once the listing is done.
struct Quiet<'a, 'b> {
    cancel: &'a CancellationToken,
    listed: &'a CancellationToken,
    task: &'a TaskObserver<'b>,
    /// The latest metadata count, for the listing to show when it ends.
    count: &'a std::sync::Mutex<Option<(u64, u64)>>,
}

impl Observer for Quiet<'_, '_> {
    fn progress(&self, stage: Stage, done: u64, total: u64) {
        *self.count.lock().expect("count mutex poisoned") = Some((done, total));
        if self.listed.is_cancelled() {
            self.task.progress(stage, done, total);
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
}

/// Scans have no timeout: a large OpenList library can take hours, and a
/// cancel stops them between works.
pub const TIMEOUT: Option<Duration> = None;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_roots_come_first() {
        let roots = [
            ("cloud".to_owned(), RootKind::OpenList),
            ("b".to_owned(), RootKind::Local),
            ("a".to_owned(), RootKind::Local),
        ];
        assert_eq!(root_order(&roots), ["b", "a", "cloud"]);
        assert_eq!(payload_root(r#"{"root":"a"}"#).as_deref(), Some("a"));
        assert_eq!(payload_root("{}"), None);
    }
}
