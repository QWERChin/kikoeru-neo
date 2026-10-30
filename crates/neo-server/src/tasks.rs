//! The in-process task scheduler. HTTP handlers only create and look up
//! tasks, so a browser that disconnects never stops a transcode or a scan.
//! A task ends only when it finishes, is cancelled, times out, or the
//! server shuts down (see "子进程" in book/src/media.md).

mod scan_queue;
#[cfg(test)]
mod tests;

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use futures_util::future::BoxFuture;
use neo_db::{
    Db, now_ms,
    tasks::{self, CommitStatus, EndState, Enqueued, TaskRow},
};
use tokio::sync::{Semaphore, oneshot, watch};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

pub use scan_queue::ScanPermit;
use scan_queue::{ScanQueue, Ticket};

/// How often a running task's progress is written to the database.
const PROGRESS_INTERVAL: Duration = Duration::from_secs(1);

/// Which concurrency limit a task waits for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    /// Transcodes, limited by `media.heavy_jobs`.
    Heavy,
    /// ffprobe runs, limited by `media.light_jobs`.
    Light,
    /// Scans run one at a time, so two roots never compete for the disk or
    /// the OpenList rate limits. Waiting scans can be reordered.
    Scan,
}

/// What a task waits for before its job starts.
enum Gate {
    Shared(Arc<Semaphore>),
    Scan(Ticket),
}

/// Held while a job runs.
enum Slot {
    Shared {
        _permit: tokio::sync::OwnedSemaphorePermit,
    },
    Scan(ScanPermit),
}

impl Gate {
    async fn acquire(&mut self) -> Slot {
        match self {
            Self::Shared(semaphore) => Slot::Shared {
                _permit: Arc::clone(semaphore)
                    .acquire_owned()
                    .await
                    .expect("lane semaphores are never closed"),
            },
            Self::Scan(ticket) => Slot::Scan(ticket.acquire().await),
        }
    }
}

impl Slot {
    /// Called once the row says running, which takes a scan out of the
    /// queue's first place.
    fn started(&self) {
        if let Self::Scan(permit) = self {
            permit.started();
        }
    }
}

/// The answer to [`Scheduler::move_to_front`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Moved {
    /// The task is now the next scan to run.
    Front,
    /// The task is not a scan waiting in this process's queue.
    NotWaiting,
}

/// What to create. `kind` is one of the values `TaskKind` serializes to.
pub struct TaskSpec {
    pub kind: &'static str,
    /// Only one queued or running task per key. A second submit returns the
    /// first task's ID and drops its job.
    pub dedupe_key: Option<String>,
    pub payload: serde_json::Value,
    pub lane: Lane,
    pub timeout: Option<Duration>,
    /// Refuse with [`SubmitError::QueueFull`] when this many tasks already
    /// wait. `None` for internal tasks such as scans.
    pub queue_limit: Option<usize>,
}

/// Handed to a running job.
pub struct JobContext {
    pub task_id: i64,
    /// Cancelled on an explicit cancel, a timeout and at shutdown. A job
    /// must stop its subprocesses and clean up, then return.
    pub cancel: CancellationToken,
    progress: watch::Sender<f64>,
}

impl JobContext {
    /// Progress from 0 to 1. Written to the database at most once a second.
    pub fn progress(&self) -> &watch::Sender<f64> {
        &self.progress
    }
}

/// What a task is, for the line the server log gets when it ends.
struct About {
    kind: String,
    payload: String,
}

/// A job's failure, shown to users as the task message.
#[derive(Debug, Clone)]
pub struct JobError(pub String);

/// `Some(result)` is stored as the task's result JSON.
pub type JobResult = Result<Option<serde_json::Value>, JobError>;
pub type Job = Box<dyn FnOnce(JobContext) -> BoxFuture<'static, JobResult> + Send>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Submitted {
    pub id: i64,
    /// False when an active task with the same `dedupe_key` was returned.
    pub created: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum SubmitError {
    #[error("the task queue is full")]
    QueueFull,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// How long finished tasks are remembered. Without a limit, repeated
/// failing `prepare` calls would grow the task table and the requester map
/// forever.
#[derive(Debug, Clone, Copy)]
pub struct Retention {
    /// How long after a task ends its requesters may still read it.
    pub requesters: Duration,
    /// Finished rows older than this are deleted.
    pub rows: Duration,
    /// Of the younger finished rows, only this many of the newest are kept.
    pub keep_rows: i64,
    /// The task table is pruned at most this often, as tasks end.
    pub prune_every: Duration,
}

impl Default for Retention {
    fn default() -> Self {
        Self {
            requesters: Duration::from_secs(3600),
            rows: Duration::from_secs(7 * 24 * 3600),
            keep_rows: 2000,
            prune_every: Duration::from_secs(60),
        }
    }
}

#[derive(Clone)]
pub struct Scheduler(Arc<Inner>);

/// Who asked for one task, and when it ended.
#[derive(Default)]
struct Requesters {
    users: HashSet<i64>,
    ended: Option<Instant>,
}

struct Inner {
    db: Db,
    heavy: Arc<Semaphore>,
    light: Arc<Semaphore>,
    scan: Arc<ScanQueue>,
    /// Serializes moves, so the order written to the database for the next
    /// start is the order the queue in memory ends up in.
    moves: tokio::sync::Mutex<()>,
    /// Every task this process runs or holds in a queue.
    live: Mutex<Registry>,
    /// Users who asked for each task through `prepare`. Kept in memory: after
    /// a restart a user's next `prepare` returns the same task and adds them
    /// again.
    requesters: Mutex<HashMap<i64, Requesters>>,
    retention: Retention,
    /// When the task table was last pruned.
    pruned_at: Mutex<Option<Instant>>,
    shutdown: CancellationToken,
    force_shutdown: CancellationToken,
    tracker: TaskTracker,
}

#[derive(Default)]
struct Registry {
    tasks: HashMap<i64, Live>,
    /// Tasks a caller cancelled before their runner registered, with how
    /// many cancels left the mark. A task's row commits before its runner
    /// registers, so a cancel can land between the two. The runner takes
    /// the mark and starts out cancelled instead of running.
    early: HashMap<i64, usize>,
}

/// A task this process runs or holds in a queue. The caller's cancel is
/// kept in the same entry, so it leaves with the task: a cancel landing
/// while the task finishes cannot leave a mark behind.
struct Live {
    cancel: CancellationToken,
    /// A caller cancelled it, as opposed to shutdown stopping it. An
    /// explicit cancel must end as `cancelled` even if shutdown follows.
    by_caller: bool,
}

impl Scheduler {
    pub fn new(db: Db, heavy_jobs: usize, light_jobs: usize) -> Self {
        Self(Arc::new(Inner {
            db,
            heavy: Arc::new(Semaphore::new(heavy_jobs)),
            light: Arc::new(Semaphore::new(light_jobs)),
            scan: Arc::new(ScanQueue::default()),
            moves: tokio::sync::Mutex::new(()),
            live: Mutex::new(Registry::default()),
            requesters: Mutex::new(HashMap::new()),
            retention: Retention::default(),
            pruned_at: Mutex::new(None),
            shutdown: CancellationToken::new(),
            force_shutdown: CancellationToken::new(),
            tracker: TaskTracker::new(),
        }))
    }

    /// Replaces the default [`Retention`]. Call before the scheduler is
    /// cloned or used.
    #[must_use]
    pub fn with_retention(mut self, retention: Retention) -> Self {
        Arc::get_mut(&mut self.0)
            .expect("retention is set before the scheduler is shared")
            .retention = retention;
        self
    }

    /// Creates a task and starts its job, or returns the active task with
    /// the same `dedupe_key`.
    ///
    /// # Errors
    /// [`SubmitError::QueueFull`], or a database error.
    pub async fn submit(&self, spec: TaskSpec, job: Job) -> Result<Submitted, SubmitError> {
        // The insert and the spawn run in a task of their own. If the caller
        // is dropped after the row commits, the job still starts, instead of
        // leaving a queued row that nobody runs and that blocks its key.
        let (reply, answer) = oneshot::channel();
        let this = self.clone();
        self.spawn_tracked(async move {
            let _ = reply.send(this.enqueue_and_spawn(spec, job).await);
        });
        answer
            .await
            .unwrap_or(Err(SubmitError::Db(sqlx::Error::WorkerCrashed)))
    }

    async fn enqueue_and_spawn(&self, spec: TaskSpec, job: Job) -> Result<Submitted, SubmitError> {
        let payload = spec.payload.to_string();
        let limit = spec
            .queue_limit
            .map(|n| i64::try_from(n).unwrap_or(i64::MAX));
        let enqueued = tasks::enqueue(
            &self.0.db,
            spec.kind,
            spec.dedupe_key.as_deref(),
            &payload,
            limit,
            now_ms(),
        )
        .await?;
        let about = About {
            kind: spec.kind.to_owned(),
            payload,
        };
        let id = match enqueued {
            Enqueued::Existing(id) => return Ok(Submitted { id, created: false }),
            Enqueued::QueueFull => return Err(SubmitError::QueueFull),
            Enqueued::Created(id) => id,
            Enqueued::CommitUnknown { id, xid, error } => {
                let this = self.clone();
                self.spawn_tracked(async move {
                    // Until commit is confirmed, this ID might belong to
                    // another submit. Do not register or cancel its runner.
                    if this.wait_for_enqueue_commit(id, &xid).await {
                        this.spawn(id, about, spec.lane, spec.timeout, job);
                    }
                });
                return Err(SubmitError::Db(error));
            }
        };
        self.spawn(id, about, spec.lane, spec.timeout, job);
        Ok(Submitted { id, created: true })
    }

    async fn wait_for_enqueue_commit(&self, id: i64, xid: &str) -> bool {
        let status = self
            .0
            .shutdown
            .run_until_cancelled(self.retry_task_state(id, || async {
                let status = tasks::enqueue_commit_status(&self.0.db, xid).await?;
                Ok((status != CommitStatus::InProgress).then_some(status))
            }))
            .await
            .flatten();
        match status {
            Some(CommitStatus::Committed) => true,
            Some(CommitStatus::Aborted) => false,
            _ => {
                tracing::warn!(task = id, %xid, ?status,
                    "enqueue commit unresolved; leaving any queued row to startup recovery");
                false
            }
        }
    }

    /// Picks up tasks left by the previous process: `running` ones become
    /// `interrupted`, and `queued` ones are handed to `rebuild`, which turns
    /// a stored task back into its lane, timeout and job. Tasks it cannot
    /// rebuild are marked `interrupted` too. Call once, before serving.
    ///
    /// # Errors
    /// A database error.
    pub async fn recover<F>(&self, rebuild: F) -> Result<(), sqlx::Error>
    where
        F: Fn(&TaskRow) -> Option<(Lane, Option<Duration>, Job)>,
    {
        self.prune_rows().await;
        let recovered = tasks::recover(&self.0.db, now_ms()).await?;
        for row in recovered.interrupted {
            let about = About {
                kind: row.kind,
                payload: row.payload,
            };
            log_end(
                row.id,
                &about,
                EndState::Interrupted,
                row.message.as_deref(),
            );
        }
        for row in recovered.queued {
            let rebuilt = rebuild(&row);
            let about = About {
                kind: row.kind,
                payload: row.payload,
            };
            match rebuilt {
                Some((lane, timeout, job)) => self.spawn(row.id, about, lane, timeout, job),
                None => {
                    let message = "服务重启，任务被中断";
                    let state = EndState::Interrupted;
                    if tasks::finish(&self.0.db, row.id, state, None, Some(message), now_ms())
                        .await?
                    {
                        log_end(row.id, &about, state, Some(message));
                    }
                }
            }
        }
        Ok(())
    }

    /// Requests cancellation. Returns false when the task is not active in
    /// this process.
    ///
    /// # Errors
    /// A database error while cancelling a task this process does not hold.
    pub async fn cancel(&self, id: i64) -> Result<bool, sqlx::Error> {
        if self.cancel_runner(id) {
            return Ok(true);
        }
        self.cancel_without_runner(id).await
    }

    /// A cancel that found no runner. The task's row may have committed
    /// with the runner still to register, or the task stayed queued at
    /// shutdown. The row also gives the kind and payload for the log line.
    async fn cancel_without_runner(&self, id: i64) -> Result<bool, sqlx::Error> {
        match tasks::get(&self.0.db, id).await? {
            Some(row) if row.state == "queued" => self.cancel_unregistered(row).await,
            // A runner that registered after the first check may have
            // started it. Without one, only a failed write back to queued
            // at shutdown leaves a running row, for startup recovery.
            Some(row) if row.state == "running" => Ok(self.cancel_runner(id)),
            _ => Ok(false),
        }
    }

    /// Cancels a task that was queued with no runner when `row` was read.
    async fn cancel_unregistered(&self, row: TaskRow) -> Result<bool, sqlx::Error> {
        let id = row.id;
        // The mark comes first, so a runner that registers from now on
        // starts out cancelled. One that registered since the first check
        // is cancelled directly.
        if self.mark_or_cancel(id) {
            return Ok(true);
        }
        // Only a queued row. A running row marked cancelled under its job
        // would free the dedupe key while the job still runs.
        let state = EndState::Cancelled;
        let cancelled = match tasks::cancel_queued(&self.0.db, id, now_ms()).await {
            Ok(cancelled) => cancelled,
            Err(error) => {
                // The caller sees the cancel fail, so its mark goes too, and
                // a mark for a runner that has already exited cannot linger.
                self.unmark(id);
                return Err(error);
            }
        };
        if cancelled {
            let about = About {
                kind: row.kind,
                payload: row.payload,
            };
            log_end(id, &about, state, None);
            // The mark stays for the runner that may still register.
            return Ok(true);
        }
        // Another cancel, or the runner that took the marks, ended the row
        // first. This cancel's mark is no longer needed, and dropping only
        // its own count keeps the one another cancel left for the runner.
        self.unmark(id);
        Ok(tasks::get(&self.0.db, id)
            .await?
            .is_some_and(|row| row.state == "cancelled"))
    }

    fn registry(&self) -> std::sync::MutexGuard<'_, Registry> {
        self.0.live.lock().expect("live mutex poisoned")
    }

    /// Cancels the task's runner, if one is registered.
    fn cancel_runner(&self, id: i64) -> bool {
        let token = self.registry().tasks.get_mut(&id).map(|live| {
            live.by_caller = true;
            live.cancel.clone()
        });
        token.map(|token| token.cancel()).is_some()
    }

    /// Cancels the task's runner if one has registered, and otherwise adds
    /// a mark that the next runner for it takes. Runners register under the
    /// same lock, so none slips in between the check and the mark.
    fn mark_or_cancel(&self, id: i64) -> bool {
        let mut registry = self.registry();
        let token = registry.tasks.get_mut(&id).map(|live| {
            live.by_caller = true;
            live.cancel.clone()
        });
        match token {
            Some(token) => {
                drop(registry);
                token.cancel();
                true
            }
            None => {
                *registry.early.entry(id).or_default() += 1;
                false
            }
        }
    }

    fn unmark(&self, id: i64) {
        let mut registry = self.registry();
        if let Some(count) = registry.early.get_mut(&id) {
            *count -= 1;
            if *count == 0 {
                registry.early.remove(&id);
            }
        }
    }

    /// # Errors
    /// A database error.
    pub async fn get(&self, id: i64) -> Result<Option<TaskRow>, sqlx::Error> {
        tasks::get(&self.0.db, id).await
    }

    /// Records that `user_id` asked for task `id`. The grace period starts
    /// only when the task ends: `submit` returns active tasks, and one that
    /// ended before this call has already left its end time here.
    pub fn add_requester(&self, id: i64, user_id: i64) {
        self.0
            .requesters
            .lock()
            .expect("requester mutex poisoned")
            .entry(id)
            .or_default()
            .users
            .insert(user_id);
    }

    pub fn is_requester(&self, id: i64, user_id: i64) -> bool {
        self.0
            .requesters
            .lock()
            .expect("requester mutex poisoned")
            .get(&id)
            .is_some_and(|r| r.users.contains(&user_id))
    }

    /// After a task ends: starts its requesters' grace period and, when a
    /// prune is due, forgets the requesters whose grace period is over and
    /// prunes the task table.
    async fn forget_finished(&self, id: i64) {
        let now = Instant::now();
        // Recorded even with no requester yet: `prepare` may add one just
        // after a fast task ended, and it must find the end time.
        self.0
            .requesters
            .lock()
            .expect("requester mutex poisoned")
            .entry(id)
            .or_default()
            .ended = Some(now);
        let due = {
            let mut pruned_at = self.0.pruned_at.lock().expect("prune mutex poisoned");
            let due =
                pruned_at.is_none_or(|at| now.duration_since(at) >= self.0.retention.prune_every);
            if due {
                *pruned_at = Some(now);
            }
            due
        };
        if due {
            // The sweep visits every entry, so it runs on the prune cadence,
            // not after each of the thousands of probes a scan finishes.
            let keep = self.0.retention.requesters;
            self.0
                .requesters
                .lock()
                .expect("requester mutex poisoned")
                .retain(|_, r| r.ended.is_none_or(|ended| now.duration_since(ended) < keep));
            self.prune_rows().await;
        }
    }

    async fn prune_rows(&self) {
        let retention = self.0.retention;
        let cutoff =
            now_ms().saturating_sub(i64::try_from(retention.rows.as_millis()).unwrap_or(i64::MAX));
        if let Err(error) = tasks::prune(&self.0.db, cutoff, retention.keep_rows).await {
            tracing::warn!(%error, "cannot prune finished tasks");
        }
    }

    /// Cancels every task without waiting. The server calls this as soon as
    /// it receives SIGTERM, so jobs start stopping while HTTP connections
    /// drain, and no queued task starts in the meantime.
    pub fn begin_shutdown(&self) {
        self.0.shutdown.cancel();
        self.0.tracker.close();
    }

    /// Resolves once shutdown has begun. Long-lived responses such as the
    /// task event stream end on it, because the server drains every open
    /// connection before it stops.
    pub async fn shutting_down(&self) {
        self.0.shutdown.cancelled().await;
    }

    /// The scan lane's slot, if no scan holds it. Work that must not
    /// interleave with a scan, such as deleting missing works, holds it
    /// for its duration; queued scans wait meanwhile.
    pub fn try_scan_slot(&self) -> Option<ScanPermit> {
        self.0.scan.try_acquire()
    }

    /// Where a queued scan stands in the scan lane, 1 being the next to run
    /// once the running scan ends. `None` for any other task.
    pub fn queue_position(&self, id: i64) -> Option<usize> {
        self.0.scan.position(id)
    }

    /// Moves a queued scan to the front of the scan lane. The running scan
    /// keeps running. The order is stored, so a restart keeps it.
    ///
    /// # Errors
    /// A database error. The queue in memory is left as it was.
    pub async fn move_to_front(&self, id: i64) -> Result<Moved, sqlx::Error> {
        // The write and the move run in a task of their own, like `submit`.
        // A caller dropped after the rank commits would otherwise leave a
        // stored order that the queue in memory never took on.
        let (reply, answer) = oneshot::channel();
        let this = self.clone();
        self.spawn_tracked(async move {
            let _ = reply.send(this.move_now(id).await);
        });
        answer.await.unwrap_or(Err(sqlx::Error::WorkerCrashed))
    }

    async fn move_now(&self, id: i64) -> Result<Moved, sqlx::Error> {
        let _serial = self.0.moves.lock().await;
        match self.0.scan.position(id) {
            None => return Ok(Moved::NotWaiting),
            Some(1) => return Ok(Moved::Front),
            Some(_) => {}
        }
        // A scan that started during the write gets a rank it never uses.
        tasks::move_to_front(&self.0.db, id).await?;
        Ok(if self.0.scan.move_to_front(id) {
            Moved::Front
        } else {
            Moved::NotWaiting
        })
    }

    /// Cancels every task and waits for the jobs to clean up. Running tasks
    /// end as `interrupted`, queued ones stay `queued` for the next start.
    /// After the grace period, requests dropping stuck jobs and waits for
    /// their destructors. The server bounds its wait on a separate cleanup
    /// task, since synchronous I/O and destructors cannot be preempted.
    /// Unfinished rows are left for startup recovery.
    pub async fn shutdown(&self, grace: Duration) {
        self.begin_shutdown();
        if tokio::time::timeout(grace, self.0.tracker.wait())
            .await
            .is_err()
        {
            tracing::warn!("tasks did not stop within the shutdown grace period");
            self.0.force_shutdown.cancel();
            self.0.tracker.wait().await;
            self.registry().tasks.clear();
        }
    }

    fn spawn_tracked(
        &self,
        work: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> tokio::task::JoinHandle<Option<()>> {
        // Cooperative cancellation cannot interrupt a query whose peer
        // stopped responding. Drop it once the shutdown grace expires.
        self.0.tracker.spawn(
            self.0
                .force_shutdown
                .clone()
                .run_until_cancelled_owned(work),
        )
    }

    /// Scans join their queue here rather than when the runner first polls,
    /// so they line up in the order they were spawned.
    fn gate(&self, id: i64, lane: Lane) -> Gate {
        match lane {
            Lane::Heavy => Gate::Shared(Arc::clone(&self.0.heavy)),
            Lane::Light => Gate::Shared(Arc::clone(&self.0.light)),
            Lane::Scan => Gate::Scan(self.0.scan.join(id)),
        }
    }

    fn spawn(&self, id: i64, about: About, lane: Lane, timeout: Option<Duration>, job: Job) {
        let cancel = self.0.shutdown.child_token();
        let by_caller = {
            let mut registry = self.registry();
            let by_caller = registry.early.remove(&id).is_some();
            registry.tasks.insert(
                id,
                Live {
                    cancel: cancel.clone(),
                    by_caller,
                },
            );
            by_caller
        };
        if by_caller {
            cancel.cancel();
        }
        let this = self.clone();
        let gate = self.gate(id, lane);
        self.spawn_tracked(async move {
            let ended = this.run(id, &about, gate, timeout, job, cancel).await;
            this.registry().tasks.remove(&id);
            // Only after run() has given up the queue place and the lane
            // slot: a prune that waits for the database must hold up neither
            // the next task nor the places of the scans behind this one.
            if ended {
                this.forget_finished(id).await;
            }
        });
    }

    /// A task cancelled before it started. At shutdown it stays queued and
    /// runs after the restart. True when it ended.
    async fn end_unstarted(&self, id: i64, about: &About) -> bool {
        let ends = !self.0.shutdown.is_cancelled() || !self.stay_queued(id);
        if ends {
            self.end(id, about, EndState::Cancelled, None, None).await;
        }
        ends
    }

    /// At shutdown, gives up the runner of a task whose row is queued, so
    /// the next start runs it. False when a caller cancelled it first, and
    /// the task must end as cancelled instead. The check and the removal
    /// share one lock: a cancel after it finds no runner and cancels the
    /// queued row itself, so it is never accepted and then dropped.
    fn stay_queued(&self, id: i64) -> bool {
        let by_caller = self
            .registry()
            .tasks
            .remove(&id)
            .is_some_and(|live| live.by_caller);
        !by_caller
    }

    fn cancelled_by_caller(&self, id: i64) -> bool {
        self.registry()
            .tasks
            .get(&id)
            .is_some_and(|live| live.by_caller)
    }

    async fn end(
        &self,
        id: i64,
        about: &About,
        state: EndState,
        result: Option<&str>,
        message: Option<&str>,
    ) {
        let db = &self.0.db;
        let ended = self
            .persist(id, || {
                tasks::finish(db, id, state, result, message, now_ms())
            })
            .await;
        if ended == Some(true) {
            log_end(id, about, state, message);
        }
    }

    /// Retries a state write until it succeeds, backing off up to 30 s. A
    /// task whose state could not be written would otherwise block its
    /// dedupe key with a row nobody owns. At shutdown it gives up, and
    /// startup recovery settles the row.
    async fn persist<T, F, Fut>(&self, id: i64, write: F) -> Option<T>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<T, sqlx::Error>>,
    {
        self.retry_task_state(id, || async { write().await.map(Some) })
            .await
    }

    /// A successful read can still be pending, such as an in-progress commit.
    async fn retry_task_state<T, F, Fut>(&self, id: i64, query: F) -> Option<T>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = Result<Option<T>, sqlx::Error>>,
    {
        let mut delay = Duration::from_millis(100);
        loop {
            match query().await {
                Ok(Some(value)) => return Some(value),
                Ok(None) => {}
                Err(error) => {
                    tracing::error!(task = id, %error, "cannot access the task state, retrying")
                }
            }
            tokio::select! {
                () = tokio::time::sleep(delay) => {}
                () = self.0.shutdown.cancelled() => return None,
            }
            delay = (delay * 2).min(Duration::from_secs(30));
        }
    }

    /// Waits for the lane, runs the job and records how the task ended.
    /// True when an end was recorded, so the caller knows to forget the task
    /// once the slot is free. False when it stays queued for the restart or
    /// its row was ended elsewhere.
    async fn run(
        &self,
        id: i64,
        about: &About,
        gate: Gate,
        timeout: Option<Duration>,
        job: Job,
        cancel: CancellationToken,
    ) -> bool {
        let db = &self.0.db;
        // The gate lives to the end of this function, so a cancelled scan
        // keeps its queue place until its row has ended.
        let mut gate = gate;
        // Biased, so a task cancelled while it waited never starts even when
        // a permit frees up at the same moment.
        let permit = tokio::select! {
            biased;
            () = cancel.cancelled() => None,
            slot = gate.acquire() => Some(slot),
        };
        let Some(slot) = permit else {
            return self.end_unstarted(id, about).await;
        };
        if cancel.is_cancelled() {
            return self.end_unstarted(id, about).await;
        }
        let started = self.persist(id, || tasks::start(db, id, now_ms())).await;
        if started != Some(true) {
            return false;
        }
        // Right after the write, so the scans behind it move up even when a
        // cancel below still waits for its own write.
        slot.started();
        // A cancel that arrived during the database write still wins. The job
        // has not run, so a shutdown puts the task back in the queue.
        if cancel.is_cancelled() {
            if self.cancelled_by_caller(id) || !self.0.shutdown.is_cancelled() {
                self.end(id, about, EndState::Cancelled, None, None).await;
                return true;
            }
            // Back to queued first, so that a cancel arriving after the
            // runner is gone finds a queued row to cancel.
            self.persist(id, || tasks::unstart(db, id, now_ms())).await;
            if self.stay_queued(id) {
                return false;
            }
            self.end(id, about, EndState::Cancelled, None, None).await;
            return true;
        }
        let (progress, watcher) = watch::channel(0.0);
        let context = JobContext {
            task_id: id,
            cancel: cancel.clone(),
            progress,
        };
        let mut work = job(context);
        // Progress is written by its own task. Awaiting the write here would
        // stop polling the job, and a job holding the single write
        // connection would then deadlock with it.
        let flusher = self.spawn_tracked(flush_progress(self.0.db.clone(), id, watcher));
        // Without a timeout the deadline is a century away. tokio clamps it.
        let deadline =
            tokio::time::sleep(timeout.unwrap_or(Duration::from_secs(100 * 365 * 86_400)));
        tokio::pin!(deadline);
        let mut timed_out = false;
        // After a cooperative cancel or timeout, the job still finishes so
        // it can stop its subprocess and remove temporary files. Only
        // forced shutdown drops it.
        let outcome = loop {
            tokio::select! {
                outcome = &mut work => break outcome,
                () = &mut deadline, if !timed_out => {
                    timed_out = true;
                    cancel.cancel();
                }
            }
        };
        flusher.abort();
        let _ = flusher.await;
        // A job that finished its work counts as done even if a cancel
        // arrived at the last moment.
        let (state, result, message) = match outcome {
            Ok(result) => (EndState::Done, result.map(|r| r.to_string()), None),
            Err(_) if self.cancelled_by_caller(id) => (EndState::Cancelled, None, None),
            Err(_) if self.0.shutdown.is_cancelled() => (
                EndState::Interrupted,
                None,
                Some("服务关闭，任务被中断".to_owned()),
            ),
            Err(_) if timed_out => (EndState::Failed, None, Some("任务超时".to_owned())),
            Err(_) if cancel.is_cancelled() => (EndState::Cancelled, None, None),
            Err(JobError(message)) => (EndState::Failed, None, Some(message)),
        };
        self.end(id, about, state, result.as_deref(), message.as_deref())
            .await;
        true
    }
}

/// One line per finished task, so the tasks the settings page leaves out,
/// such as loudness analyses, can still be traced. A scan queues a probe
/// for every new track, thousands at a time, so probes log at debug.
fn log_end(id: i64, about: &About, state: EndState, message: Option<&str>) {
    let (kind, state, payload) = (about.kind.as_str(), state.as_str(), about.payload.as_str());
    if kind == "probe" {
        tracing::debug!(task = id, kind, state, payload, message, "task ended");
    } else {
        tracing::info!(task = id, kind, state, payload, message, "task ended");
    }
}

/// Writes the latest progress at most once per interval, best effort.
async fn flush_progress(db: Db, id: i64, mut watcher: watch::Receiver<f64>) {
    // Each wait starts after the previous write finished, so a write that
    // was held up by a busy writer is never followed by a catch-up burst.
    loop {
        tokio::time::sleep(PROGRESS_INTERVAL).await;
        if !watcher.has_changed().unwrap_or(false) {
            continue;
        }
        let value = *watcher.borrow_and_update();
        if let Err(error) = tasks::set_progress(&db, id, value, now_ms()).await {
            tracing::warn!(task = id, %error, "cannot record task progress");
        }
    }
}
