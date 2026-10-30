//! Scanning a root: discover work directories, sync each work's tracks,
//! mark what is gone as missing, and match imported play progress to the
//! new tracks. The rules are in book/src/scanning.md and
//! book/src/migration.md.
//!
//! `scan_root` neither fetches metadata nor probes durations. The server's
//! scan job runs the fetchers in `metadata` beside it, and the summary lists
//! the tracks that need a probe so the caller can queue them.

mod discover;
mod matching;
pub mod metadata;
pub mod names;
mod tracks;
mod works;

pub use tracks::dedupe_work;

use std::collections::BTreeSet;

use futures_util::StreamExt;

use neo_db::Db;
use neo_storage::{Storage, StorageError};

/// The most lines a scan keeps in its log. A broken library can produce
/// one line per file, which nobody reads past the first few hundred.
const LOG_LIMIT: usize = 500;

pub struct Root<'a> {
    /// The `roots.id` the config gives this root.
    pub id: &'a str,
    pub storage: &'a dyn Storage,
    /// Local tracks are probed after the scan. OpenList tracks are probed
    /// on first play, because each probe needs a direct link.
    pub probe_after_scan: bool,
    /// Skip the deep walk of a work whose top-level listing has not changed
    /// since the last full walk. Meant for OpenList, where every listing
    /// counts against the provider's rate limit (book/src/storage.md).
    pub skip_unchanged: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Discover,
    Tracks,
    /// Fetching metadata for new works, the third stage.
    Metadata,
    /// Taking covers from the works' own folders where no source gave
    /// one, the last stage.
    Covers,
}

/// How the caller follows a scan. The task scheduler implements it.
pub trait Observer: Send + Sync {
    fn progress(&self, _stage: Stage, _done: u64, _total: u64) {}

    /// Called right after a work's tracks are committed with the tracks
    /// that need ffprobe, so a cancelled scan has already
    /// handed over the probes for the works it finished.
    fn probe(&self, _track_ids: &[i64]) {}

    /// Checked between works. Works already synced stay committed.
    fn is_cancelled(&self) -> bool {
        false
    }
}

pub struct NoObserver;

impl Observer for NoObserver {}

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("root {0} is not in the database; it must be in the config")]
    UnknownRoot(String),
    /// The work list could not be read in full, so nothing was changed.
    #[error(transparent)]
    Unavailable(StorageError),
    #[error("scan cancelled")]
    Cancelled,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    pub works_found: u64,
    pub works_new: u64,
    pub works_missing: u64,
    pub works_restored: u64,
    /// New locations of works that already existed, in this or another root.
    pub locations_new: u64,
    pub locations_missing: u64,
    pub locations_restored: u64,
    pub tracks_new: u64,
    pub tracks_changed: u64,
    pub tracks_missing: u64,
    pub tracks_restored: u64,
    /// Locations whose file tree could not be fully listed. Failed
    /// subtrees keep their tracks while readable siblings are synced.
    pub works_failed: u64,
    /// Locations not walked because their top-level listing matched the
    /// stored digest.
    pub works_unchanged: u64,
    pub pending_matched: u64,
    pub pending_ambiguous: u64,
    pub pending_superseded: u64,
    /// Pending rows with no track at the old path yet. They stay pending.
    pub pending_not_found: u64,
    /// Pending rows of works with a location that has no complete listing,
    /// in this or another root. They stay pending.
    pub pending_deferred: u64,
    /// Local audio and video tracks that need ffprobe.
    pub probe: Vec<i64>,
    pub log: Vec<String>,
}

impl Summary {
    fn log(&mut self, line: String) {
        if self.log.len() < LOG_LIMIT {
            self.log.push(line);
        } else if self.log.len() == LOG_LIMIT {
            self.log.push("further log lines were dropped".to_owned());
        }
    }
}

/// Scans one root. `root_order` lists root IDs from most to least preferred
/// (local roots first, then config order) and decides which copy of a file
/// that exists in several locations is kept. `now` is Unix milliseconds.
pub async fn scan_root(
    db: &Db,
    root: &Root<'_>,
    root_order: &[String],
    observer: &dyn Observer,
    now: i64,
) -> Result<Summary, ScanError> {
    if !neo_db::scan::root_exists(db, root.id).await? {
        return Err(ScanError::UnknownRoot(root.id.to_owned()));
    }
    let mut summary = Summary::default();

    let found = match discover::discover(root.storage, observer, &mut summary).await {
        Ok(found) => found,
        Err(discover::Failure::Storage(e)) => {
            set_root_status(db, root.id, "unavailable", None).await?;
            return Err(ScanError::Unavailable(e));
        }
        Err(discover::Failure::Cancelled) => return Err(ScanError::Cancelled),
    };
    summary.works_found = found.len() as u64;
    let (present, affected) = works::sync(db, root.id, &found, now, &mut summary).await?;

    let total = present.len() as u64;
    let mut synced = BTreeSet::new();
    let mut failed = BTreeSet::new();
    // Up to list_concurrency locations are listed ahead, but `buffered`
    // yields them in order, so tracks are written in the same order and
    // get the same IDs as a scan that lists one location at a time.
    // Returning drops the stream and with it any listing still running.
    // Boxed futures keep the scan future Send for any lifetime, which the
    // higher-ranked closure of `StreamExt::map` would not. The listings read
    // no database: a buffered future that is not being polled must not hold
    // a pool connection that the writes below wait for.
    let digests = tracks::stored_digests(db, root).await?;
    let listings: Vec<futures_util::future::BoxFuture<'_, _>> = present
        .iter()
        .map(|location| {
            let stored = digests.get(&location.location_id).map(String::as_str);
            Box::pin(tracks::list_location(root, location, stored)) as _
        })
        .collect();
    let mut listings =
        futures_util::stream::iter(listings).buffered(root.storage.list_concurrency().max(1));
    for (done, location) in present.iter().enumerate() {
        if observer.is_cancelled() {
            return cancel(db, &synced, &affected, root_order).await;
        }
        observer.progress(Stage::Tracks, done as u64, total);
        let listing = listings.next().await.expect("one listing per location");
        if observer.is_cancelled() {
            return cancel(db, &synced, &affected, root_order).await;
        }
        if tracks::apply_listing(db, root, location, listing, now, observer, &mut summary).await? {
            synced.insert(location.work_id);
        } else {
            failed.insert(location.work_id);
        }
    }
    observer.progress(Stage::Tracks, total, total);
    // A cancel that came in while the last location was synced.
    if observer.is_cancelled() {
        return cancel(db, &synced, &affected, root_order).await;
    }
    // Only after every location of this root is synced, so that a work with
    // two locations here is matched against both and an ambiguous old path
    // is not matched to whichever copy happened to be listed first. Works
    // that lost a location here are included, so a copy in another root
    // takes over from the one that went missing.
    // A work with a location that could not be listed is not matched: the
    // unlisted files might be the other candidate for an old path.
    // A cancel between works stops matching but still settles duplicates
    // for every work, since their tracks are already committed.
    let mut cancelled = false;
    for work_id in affected {
        cancelled = cancelled || observer.is_cancelled();
        let matching = (!cancelled && !failed.contains(&work_id)).then_some(&mut summary);
        reconcile(db, work_id, root_order, matching).await?;
    }
    if cancelled {
        return Err(ScanError::Cancelled);
    }
    set_root_status(db, root.id, "ok", Some(now)).await?;
    Ok(summary)
}

/// Committed changes (synced tracks, locations marked missing) may have
/// changed which copy is kept, so duplicates are settled. Imports are not
/// matched against a scan that did not finish.
async fn cancel(
    db: &Db,
    synced: &BTreeSet<i64>,
    affected: &BTreeSet<i64>,
    root_order: &[String],
) -> Result<Summary, ScanError> {
    for &work_id in synced.union(affected) {
        reconcile(db, work_id, root_order, None).await?;
    }
    Err(ScanError::Cancelled)
}

/// Settles duplicates and, when `summary` is given, matches imported
/// progress, in one write transaction. A scan of another root cannot slip
/// a new copy in between and make a path look ambiguous.
async fn reconcile(
    db: &Db,
    work_id: i64,
    root_order: &[String],
    summary: Option<&mut Summary>,
) -> Result<(), sqlx::Error> {
    let mut tx = db.begin().await?;
    tracks::dedupe_work(&mut tx, work_id, root_order).await?;
    if let Some(summary) = summary {
        matching::match_work(&mut tx, work_id, summary).await?;
    }
    tx.commit().await
}

async fn set_root_status(
    db: &Db,
    root_id: &str,
    status: &str,
    scanned_at: Option<i64>,
) -> Result<(), sqlx::Error> {
    neo_db::scan::set_root_status(db, root_id, status, scanned_at).await
}
