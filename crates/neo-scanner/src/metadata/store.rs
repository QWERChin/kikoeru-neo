//! Writes fetched metadata and covers under the rules of
//! book/src/data-model.md: `manual` metadata and `imported` or `manual`
//! covers are only replaced when the caller forces it.

use std::{
    collections::HashSet,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

use neo_db::{
    Db, Tx,
    metadata::{self as stored_meta, Fields},
};
use serde_json::{Map, Value};
use tokio::sync::RwLock;

use super::{MetadataError, MetadataProvider, MetadataSource, WorkMetadata};
use crate::{Observer, Stage};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchOutcome {
    /// Metadata written, and the cover too when it was allowed and found.
    Updated,
    /// The work's metadata is `manual` and the fetch was not forced. Only
    /// an unprotected cover may have changed.
    ManualKept,
    NotFound,
    Unavailable(String),
    /// The cover could not be written into the data directory, so nothing
    /// was stored and the work stays for the next scan.
    CoverNotStored(String),
    /// The work no longer exists.
    NoWork,
}

/// Where covers go: the final directory and a temporary one on the same
/// file system, for the rename.
#[derive(Debug, Clone)]
pub struct CoverDirs {
    pub covers: PathBuf,
    pub tmp: PathBuf,
    /// A replaced cover file and its `cover_version` change in two steps.
    /// Cover readers hold the read side while they pair the version with
    /// the file, and a replacement holds the write side until it commits
    /// or rolls back, so no thumbnail is cached under the wrong version.
    /// The set holds works whose commit was not confirmed or whose previous
    /// cover could not be put back after a failed replacement. Readers serve
    /// no cover for them, and they get no new one, until startup settles it.
    pub lock: Arc<RwLock<HashSet<i64>>>,
    /// Folder images that could not be read or decoded, so later scans in
    /// this process do not download them again.
    pub folder_failures: Arc<std::sync::Mutex<HashSet<FailedImage>>>,
}

/// A track ID and the size it had, so a replaced file gets another try.
pub type FailedImage = (i64, Option<i64>);

impl CoverDirs {
    pub fn new(covers: PathBuf, tmp: PathBuf) -> Self {
        Self {
            covers,
            tmp,
            lock: Arc::default(),
            folder_failures: Arc::default(),
        }
    }

    /// Takes the lock that keeps cover replacement and an online backup
    /// apart, across processes, and holds it until the returned file is
    /// dropped. The wait polls, so dropping the future abandons it and
    /// leaves no blocking thread behind to hold up shutdown.
    pub async fn lock_across_processes(&self) -> std::io::Result<std::fs::File> {
        wait_for_covers(&self.covers).await
    }

    /// Where a replaced cover and its journal wait until the swap is
    /// settled. Inside `covers/`, so a backup carries them along while
    /// `cache/` is left out.
    pub fn swap_dir(&self) -> PathBuf {
        self.covers.join(".swap")
    }
}

/// See [`CoverDirs::lock_across_processes`]. A backup copies the database
/// first and `covers/` after it, so a cover replaced in between would be
/// restored under the old `cover_version`.
pub fn lock_covers(covers: &Path) -> std::io::Result<std::fs::File> {
    let file = open_lock(covers)?;
    file.lock()?;
    Ok(file)
}

/// [`lock_covers`] for async callers. The wait polls, so dropping the
/// future abandons it and leaves no blocking thread behind.
pub async fn wait_for_covers(covers: &Path) -> std::io::Result<std::fs::File> {
    let file = open_lock(covers)?;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) => {
                tokio::time::sleep(LOCK_POLL).await;
            }
            Err(std::fs::TryLockError::Error(e)) => return Err(e),
        }
    }
}

const LOCK_POLL: std::time::Duration = std::time::Duration::from_millis(100);

fn open_lock(covers: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(covers.join(".lock"))
}

/// A downloaded cover in the temporary directory. Removed when dropped, so
/// a cancelled or failed refresh leaves nothing behind; after a successful
/// rename there is nothing left to remove.
struct TmpCover(PathBuf);

impl Drop for TmpCover {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Fetches one work's metadata and cover and stores them.
pub async fn fetch_and_store(
    db: &Db,
    provider: &dyn MetadataProvider,
    dirs: &CoverDirs,
    work_id: i64,
    force: bool,
    now: i64,
) -> Result<FetchOutcome, sqlx::Error> {
    let Some(work) = stored_meta::sources(db, work_id).await? else {
        return Ok(FetchOutcome::NoWork);
    };
    let meta = match provider.fetch(&work.code).await {
        Ok(meta) => meta,
        Err(MetadataError::NotFound) => return Ok(FetchOutcome::NotFound),
        Err(MetadataError::Unavailable(e)) => return Ok(FetchOutcome::Unavailable(e)),
    };
    // The cover is downloaded before the transaction, so no network wait
    // holds the write connection. Whether it may replace the current one is
    // checked again inside the transaction.
    let cover = if cover_allowed(&work.cover_src, force) {
        match download_cover(provider, &work.code, &meta).await {
            Ok(cover) => cover,
            // An outage leaves the whole work for later, so the scan stops
            // here and the next one fetches metadata and cover together.
            Err(e) => return Ok(FetchOutcome::Unavailable(format!("cover: {e}"))),
        }
    } else {
        None
    };
    // Storing the metadata without the cover would take the work out of
    // the set scans fetch, and nothing would bring the cover back.
    let cover_tmp = match cover {
        Some((bytes, source)) => match write_tmp(dirs, work_id, &bytes) {
            Ok(path) => Some((TmpCover(path), source)),
            Err(e) => return Ok(FetchOutcome::CoverNotStored(e.to_string())),
        },
        None => None,
    };
    // Taken before the write transaction, so a backup holding it never
    // keeps the database locked, and outside the task below, so a caller
    // cancelled while a backup runs writes nothing.
    let backup_lock = match &cover_tmp {
        Some(_) => Some(dirs.lock_across_processes().await?),
        None => None,
    };
    // Storing runs on its own task: once the cover is renamed into place
    // the transaction must commit too, or the new file would sit behind the
    // old cover_version and stale thumbnails. A cancelled caller drops only
    // its wait, not the task.
    let db = db.clone();
    let dirs = dirs.clone();
    let stored = tokio::spawn(async move {
        let tmp = cover_tmp
            .as_ref()
            .map(|(t, source)| (t.0.as_path(), *source));
        let result = store(&db, &dirs, work_id, &meta, tmp, force, now).await;
        drop(backup_lock);
        result
    });
    match stored.await {
        Ok(result) => result,
        Err(e) => Err(sqlx::Error::Protocol(format!(
            "storing metadata panicked: {e}"
        ))),
    }
}

/// A folder cover only stands in until a source has one, so it is not
/// protected like an imported or manual cover.
fn cover_allowed(cover_src: &str, force: bool) -> bool {
    force || matches!(cover_src, "none" | "dlsite" | "asmr_one" | "folder")
}

/// The cover the metadata points at, or, when that is missing or cannot be
/// downloaded, a cover from the fallback source (book/src/scanning.md,
/// "备用来源"). `None` when neither has one; an error when the work should
/// wait for a source that is down.
async fn download_cover(
    provider: &dyn MetadataProvider,
    code: &str,
    meta: &WorkMetadata,
) -> Result<Option<(Vec<u8>, MetadataSource)>, String> {
    let first = match &meta.cover_url {
        Some(url) => match provider.cover(url).await {
            Ok(bytes) => return Ok(Some((bytes, meta.source))),
            Err(e) => Some(e),
        },
        None => None,
    };
    // Only DLsite covers fall back; a fallback cover has nowhere else to go.
    if meta.source == MetadataSource::Dlsite {
        match provider.fallback_cover(code).await {
            Ok(Some(bytes)) => return Ok(Some((bytes, MetadataSource::AsmrOne))),
            Ok(None) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    match first {
        Some(MetadataError::Unavailable(e)) => Err(e),
        Some(MetadataError::NotFound) => {
            tracing::warn!(code, "the cover is gone; storing the rest");
            Ok(None)
        }
        None => Ok(None),
    }
}

fn write_tmp(dirs: &CoverDirs, work_id: i64, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let nonce: u64 = rand_u64();
    let path = dirs.tmp.join(format!("cover-{work_id}-{nonce:016x}.jpg"));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    // The caller only learns the path on success, so a partial file is
    // removed here or it would hold disk space until the next start.
    if let Err(e) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }
    Ok(path)
}

fn rand_u64() -> u64 {
    // A unique enough suffix for a temporary name; collisions only fail the
    // create_new above.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    now.as_nanos() as u64 ^ u64::from(std::process::id()).rotate_left(32)
}

/// A cover renamed into place while the transaction that records it is
/// still open. The previous cover waits in the temporary directory until
/// the commit is confirmed. A journal next to it lets
/// [`recover_cover_swaps`] match the file to `cover_version` after a crash
/// or an unconfirmed commit.
struct CoverSwap {
    journal: PathBuf,
    entry: SwapJournal,
    target: PathBuf,
    swap_dir: PathBuf,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SwapJournal {
    work_id: i64,
    /// The `cover_version` the commit sets. A lower one in the database
    /// means the commit never happened.
    version: i64,
    /// The file name the previous cover moves to, or `None` when the work
    /// had no cover file.
    previous: Option<String>,
}

const JOURNAL_PREFIX: &str = "cover-swap-";

impl CoverSwap {
    fn publish(
        tmp: &Path,
        dirs: &CoverDirs,
        work_id: i64,
        version: i64,
        unsettled: &mut HashSet<i64>,
    ) -> std::io::Result<Self> {
        let nonce = rand_u64();
        let target = dirs.covers.join(format!("{work_id}.jpg"));
        let entry = SwapJournal {
            work_id,
            version,
            // exists() reads an unreadable directory as "no cover", and a
            // journal saying so would let recovery delete the original.
            previous: target
                .try_exists()?
                .then(|| format!("previous-{work_id}-{nonce:016x}.jpg")),
        };
        let swap_dir = dirs.swap_dir();
        std::fs::create_dir_all(&swap_dir)?;
        let journal = swap_dir.join(format!("{JOURNAL_PREFIX}{work_id}-{nonce:016x}.json"));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&journal)?;
        file.write_all(&serde_json::to_vec(&entry).map_err(std::io::Error::other)?)?;
        file.sync_all()?;
        let swap = Self {
            journal,
            entry,
            target,
            swap_dir: dirs.swap_dir(),
        };
        let moved = match &swap.entry.previous {
            Some(name) => std::fs::rename(&swap.target, swap.swap_dir.join(name)),
            None => Ok(()),
        };
        if let Err(e) = moved.and_then(|()| std::fs::rename(tmp, &swap.target)) {
            swap.roll_back(unsettled);
            return Err(e);
        }
        Ok(swap)
    }

    /// Puts the previous cover back, or drops it once the commit is in,
    /// then removes the journal. On failure the journal stays, so the next
    /// start tries again.
    fn settle(self, committed: bool) -> std::io::Result<()> {
        let previous = self.entry.previous.as_ref().map(|n| self.swap_dir.join(n));
        let result = match (committed, previous) {
            (true, Some(previous)) => remove_if_present(&previous),
            (true, None) => Ok(()),
            // Not there means it never moved, and the target is still the
            // original.
            (false, Some(previous)) if previous.exists() => std::fs::rename(previous, &self.target),
            (false, Some(_)) => Ok(()),
            (false, None) => remove_if_present(&self.target),
        };
        result?;
        remove_if_present(&self.journal)
    }

    /// Puts the previous cover back. When that fails too, the file no
    /// longer matches `cover_version`, so the work is marked unsettled.
    fn roll_back(self, unsettled: &mut HashSet<i64>) {
        let work_id = self.entry.work_id;
        if let Err(e) = self.settle(false) {
            tracing::error!(work = work_id, error = %e, "cannot put a cover back; hiding it until restart");
            unsettled.insert(work_id);
        }
    }

    /// COMMIT may have reached the database even when its reply was lost.
    /// Keep both files and the journal for startup to check the stored version.
    fn defer_recovery(self, unsettled: &mut HashSet<i64>) {
        unsettled.insert(self.entry.work_id);
        tracing::error!(
            work = self.entry.work_id,
            "cover commit was not confirmed; hiding it until restart"
        );
    }

    /// Drops the previous cover. A leftover file is only cleared at the
    /// next start.
    fn finish(self) {
        let work_id = self.entry.work_id;
        if let Err(e) = self.settle(true) {
            tracing::warn!(work = work_id, error = %e, "cannot remove a replaced cover");
        }
    }
}

fn remove_if_present(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Settles cover swaps a crash interrupted, by the `cover_version` the
/// database ended up with. Runs at startup, and after a restore from
/// backup, since the swap directory travels with `covers/`.
pub async fn recover_cover_swaps(db: &Db, dirs: &CoverDirs) -> std::io::Result<u64> {
    let mut settled = 0;
    let entries = match std::fs::read_dir(dirs.swap_dir()) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    // Settling moves covers too, so a backup must not run meanwhile.
    let _backup_lock = dirs.lock_across_processes().await?;
    for entry in entries {
        let path = entry?.path();
        let is_journal = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with(JOURNAL_PREFIX) && n.ends_with(".json"));
        if !is_journal {
            continue;
        }
        // A read error stops startup like any other failed settlement,
        // because the previous cover must not be lost.
        let Ok(journal) = serde_json::from_slice::<SwapJournal>(&std::fs::read(&path)?) else {
            // Torn while being written, so no rename had started.
            remove_if_present(&path)?;
            continue;
        };
        let version = stored_meta::cover_version(db, journal.work_id)
            .await
            .map_err(std::io::Error::other)?;
        // A deleted work loses its cover anyway, so the new file can stay
        // for cleanup to remove.
        let committed = version.is_none_or(|v| v >= journal.version);
        CoverSwap {
            target: dirs.covers.join(format!("{}.jpg", journal.work_id)),
            journal: path,
            entry: journal,
            swap_dir: dirs.swap_dir(),
        }
        .settle(committed)?;
        settled += 1;
    }
    Ok(settled)
}

async fn store(
    db: &Db,
    dirs: &CoverDirs,
    work_id: i64,
    meta: &WorkMetadata,
    cover_tmp: Option<(&Path, MetadataSource)>,
    force: bool,
    now: i64,
) -> Result<FetchOutcome, sqlx::Error> {
    let mut tx = db.begin().await?;
    let Some(work) = stored_meta::stored(&mut tx, work_id).await? else {
        return Ok(FetchOutcome::NoWork);
    };
    // Recovery may discard the first cover, so metadata must stay retryable
    // even when this fetch did not download a replacement.
    let mut unsettled = dirs.lock.write().await;
    if unsettled.contains(&work_id) {
        return Ok(FetchOutcome::CoverNotStored(
            "a previous cover swap is awaiting recovery".to_owned(),
        ));
    }
    let write_metadata = force || work.metadata_src != "manual";
    if write_metadata {
        write_fields(&mut tx, work_id, meta, &work.extra, now).await?;
    }
    let mut swap = None;
    if let Some((tmp, cover_source)) = cover_tmp
        && cover_allowed(&work.cover_src, force)
    {
        let cover = NewCover {
            tmp,
            source: cover_source.as_str(),
            version: work.cover_version + 1,
        };
        match publish_cover(&mut tx, dirs, &mut unsettled, work_id, cover, now).await? {
            Published::Swap(published) => swap = Some(published),
            // Dropping the transaction writes nothing, for the reason above.
            Published::Failed(e) => return Ok(FetchOutcome::CoverNotStored(e.to_string())),
        }
    }
    if let Err(e) = tx.commit().await {
        if let Some(swap) = swap {
            swap.defer_recovery(&mut unsettled);
        }
        return Err(e);
    }
    if let Some(swap) = swap {
        swap.finish();
    }
    Ok(if write_metadata {
        FetchOutcome::Updated
    } else {
        FetchOutcome::ManualKept
    })
}

struct NewCover<'a> {
    tmp: &'a Path,
    source: &'a str,
    version: i64,
}

enum Published {
    Swap(CoverSwap),
    /// The covers directory refused the rename. Nothing changed.
    Failed(std::io::Error),
}

/// Renames the new cover into place and records it in `tx`. The caller
/// commits, then finishes or defers recovery of the returned swap while it still
/// holds `unsettled`.
async fn publish_cover(
    tx: &mut Tx<'_>,
    dirs: &CoverDirs,
    unsettled: &mut HashSet<i64>,
    work_id: i64,
    cover: NewCover<'_>,
    now: i64,
) -> Result<Published, sqlx::Error> {
    match CoverSwap::publish(cover.tmp, dirs, work_id, cover.version, unsettled) {
        Ok(published) => {
            // A new version gives the thumbnail and cover URLs a new cache
            // key.
            if let Err(e) = stored_meta::bump_cover(tx, work_id, cover.source, now).await {
                published.roll_back(unsettled);
                return Err(e);
            }
            Ok(Published::Swap(published))
        }
        Err(e) => Ok(Published::Failed(e)),
    }
}

/// Stores a JPEG made from an image in the work's folder, unless the work
/// got a cover meanwhile. Returns whether it was stored.
///
/// # Errors
/// A database error, or a failure to write the cover into the covers
/// directory.
pub async fn store_folder_cover(
    db: &Db,
    dirs: &CoverDirs,
    work_id: i64,
    jpeg: &[u8],
    now: i64,
) -> Result<bool, FolderCoverError> {
    let tmp = TmpCover(write_tmp(dirs, work_id, jpeg).map_err(FolderCoverError::Io)?);
    let backup_lock = dirs
        .lock_across_processes()
        .await
        .map_err(FolderCoverError::Io)?;
    // On its own task for the reason fetch_and_store gives.
    let db = db.clone();
    let dirs = dirs.clone();
    let stored = tokio::spawn(async move {
        let result = async {
            let mut tx = db.begin().await.map_err(FolderCoverError::Db)?;
            let Some(work) = stored_meta::stored(&mut tx, work_id)
                .await
                .map_err(FolderCoverError::Db)?
            else {
                return Ok(false);
            };
            if work.cover_src != "none" {
                return Ok(false);
            }
            let mut unsettled = dirs.lock.write().await;
            if unsettled.contains(&work_id) {
                tracing::warn!(
                    work = work_id,
                    "a cover swap is unsettled; skipping the new cover"
                );
                return Ok(false);
            }
            let cover = NewCover {
                tmp: &tmp.0,
                source: "folder",
                version: work.cover_version + 1,
            };
            let swap = match publish_cover(&mut tx, &dirs, &mut unsettled, work_id, cover, now)
                .await
                .map_err(FolderCoverError::Db)?
            {
                Published::Swap(swap) => swap,
                // Every other folder cover would fail the same way.
                Published::Failed(e) => return Err(FolderCoverError::Io(e)),
            };
            if let Err(e) = tx.commit().await {
                swap.defer_recovery(&mut unsettled);
                return Err(FolderCoverError::Db(e));
            }
            swap.finish();
            Ok(true)
        }
        .await;
        drop(backup_lock);
        drop(tmp);
        result
    });
    match stored.await {
        Ok(result) => result,
        Err(e) => Err(FolderCoverError::Db(sqlx::Error::Protocol(format!(
            "storing a folder cover panicked: {e}"
        )))),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FolderCoverError {
    #[error(transparent)]
    Db(sqlx::Error),
    #[error("cannot write the cover: {0}")]
    Io(std::io::Error),
}

async fn write_fields(
    tx: &mut Tx<'_>,
    work_id: i64,
    meta: &WorkMetadata,
    extra: &str,
    now: i64,
) -> Result<(), sqlx::Error> {
    if let Some(c) = &meta.circle {
        stored_meta::upsert_circle(tx, c.id, &c.name, work_id).await?;
    }
    // Other keys, such as the import's legacy_durations, stay.
    let mut extra = match serde_json::from_str::<Value>(extra) {
        Ok(Value::Object(map)) => map,
        _ => Map::new(),
    };
    for (key, value) in [
        ("rank", &meta.rank),
        ("rate_count_detail", &meta.rate_count_detail),
    ] {
        match value {
            Some(v) => extra.insert(key.to_owned(), v.clone()),
            None => extra.remove(key),
        };
    }
    let extra = Value::Object(extra).to_string();
    let fields = Fields {
        title: &meta.title,
        circle_id: meta.circle.as_ref().map(|c| c.id),
        release_date: meta.release_date.as_deref(),
        nsfw: meta.nsfw,
        dl_count: meta.dl_count,
        price: meta.price,
        rate_average: meta.rate_average,
        rate_count: meta.rate_count,
        extra: &extra,
        source: meta.source.as_str(),
    };
    stored_meta::update_work(tx, work_id, fields, now).await?;
    let tags: Vec<(String, Option<i64>)> = meta
        .tags
        .iter()
        .map(|t| (t.name.clone(), t.dlsite_id))
        .collect();
    stored_meta::replace_credits(tx, work_id, &meta.vas, &tags).await
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MetadataSummary {
    pub fetched: u64,
    pub not_found: u64,
    /// Works left as they were because the source could not be reached.
    /// The next scan tries them again.
    pub unavailable: u64,
}

/// Fetches metadata for the works of a root that have none yet, as the
/// third stage of a scan. The first unreachable answer stops the stage:
/// with the network down, every further request would wait out its retries
/// for nothing, and the next scan picks the rest up.
pub async fn fetch_missing(
    db: &Db,
    provider: &dyn MetadataProvider,
    dirs: &CoverDirs,
    root_id: &str,
    observer: &dyn Observer,
    now: impl Fn() -> i64,
) -> Result<MetadataSummary, sqlx::Error> {
    let mut tried = HashSet::new();
    fetch_missing_except(db, provider, dirs, root_id, observer, now, &mut tried).await
}

/// [`fetch_missing`], skipping the works in `tried` and adding each work it
/// asks about. A scan that fetches while it is still listing runs a second
/// pass afterwards, and a work DLsite does not know is not asked twice.
pub async fn fetch_missing_except(
    db: &Db,
    provider: &dyn MetadataProvider,
    dirs: &CoverDirs,
    root_id: &str,
    observer: &dyn Observer,
    now: impl Fn() -> i64,
    tried: &mut HashSet<i64>,
) -> Result<MetadataSummary, sqlx::Error> {
    let works: Vec<i64> = stored_meta::works_without_metadata(db, root_id)
        .await?
        .into_iter()
        .filter(|id| !tried.contains(id))
        .collect();
    let mut summary = MetadataSummary::default();
    let total = works.len() as u64;
    for (done, work_id) in works.into_iter().enumerate() {
        if observer.is_cancelled() {
            break;
        }
        observer.progress(Stage::Metadata, done as u64, total);
        tried.insert(work_id);
        match fetch_and_store(db, provider, dirs, work_id, false, now()).await? {
            FetchOutcome::Updated | FetchOutcome::ManualKept => summary.fetched += 1,
            FetchOutcome::NotFound => summary.not_found += 1,
            FetchOutcome::NoWork => {}
            FetchOutcome::Unavailable(e) => {
                tracing::warn!(work = work_id, error = %e, "metadata source unavailable; stopping");
                summary.unavailable = total - done as u64;
                break;
            }
            // Every other cover would meet the same data directory.
            FetchOutcome::CoverNotStored(e) => {
                tracing::warn!(work = work_id, error = %e, "cannot store a cover; stopping");
                summary.unavailable = total - done as u64;
                break;
            }
        }
    }
    observer.progress(Stage::Metadata, total, total);
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup() -> (tempfile::TempDir, Db, CoverDirs) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open_sqlite(&dir.path().join("neo.sqlite3"))
            .await
            .unwrap();
        let dirs = CoverDirs::new(dir.path().join("covers"), dir.path().join("tmp"));
        std::fs::create_dir_all(&dirs.covers).unwrap();
        std::fs::create_dir_all(&dirs.tmp).unwrap();
        sqlx::raw_sql(
            "INSERT INTO works (id, code, title, metadata_src, cover_src, cover_version,
                                created_at, updated_at)
             VALUES (1, 'RJ01000001', 'a', 'none', 'dlsite', 3, 0, 0),
                    (2, 'RJ01000002', 'b', 'none', 'none', 0, 0, 0)",
        )
        .execute(&db.sqlite().unwrap().write)
        .await
        .unwrap();
        std::fs::write(dirs.covers.join("1.jpg"), b"old").unwrap();
        (dir, db, dirs)
    }

    /// Publishes a new cover and stops there, as a crash before the commit
    /// would.
    fn crash_mid_swap(dirs: &CoverDirs, work_id: i64, version: i64) {
        let new = write_tmp(dirs, work_id, b"new").unwrap();
        let swap = CoverSwap::publish(&new, dirs, work_id, version, &mut HashSet::new()).unwrap();
        drop(swap);
    }

    fn tmp_is_empty(dirs: &CoverDirs) -> bool {
        let empty = |dir: PathBuf| std::fs::read_dir(dir).map_or(true, |mut d| d.next().is_none());
        empty(dirs.tmp.clone()) && empty(dirs.swap_dir())
    }

    #[tokio::test]
    async fn an_uncommitted_swap_is_rolled_back_at_startup() {
        let (_dir, db, dirs) = setup().await;
        crash_mid_swap(&dirs, 1, 4);
        crash_mid_swap(&dirs, 2, 1);
        assert_eq!(std::fs::read(dirs.covers.join("1.jpg")).unwrap(), b"new");
        // A backup leaves cache/ out, and recovery still works without it.
        std::fs::remove_dir_all(&dirs.tmp).unwrap();
        std::fs::create_dir_all(&dirs.tmp).unwrap();

        assert_eq!(recover_cover_swaps(&db, &dirs).await.unwrap(), 2);
        assert_eq!(std::fs::read(dirs.covers.join("1.jpg")).unwrap(), b"old");
        assert!(!dirs.covers.join("2.jpg").exists());
        assert!(tmp_is_empty(&dirs));
    }

    #[tokio::test]
    async fn a_recovery_that_cannot_restore_fails_and_keeps_its_journal() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, db, dirs) = setup().await;
        crash_mid_swap(&dirs, 1, 4);
        let mode = |m| std::fs::Permissions::from_mode(m);
        std::fs::set_permissions(&dirs.covers, mode(0o555)).unwrap();
        let result = recover_cover_swaps(&db, &dirs).await;
        std::fs::set_permissions(&dirs.covers, mode(0o755)).unwrap();
        assert!(result.is_err());
        assert!(!tmp_is_empty(&dirs), "the journal and the old cover stay");

        assert_eq!(recover_cover_swaps(&db, &dirs).await.unwrap(), 1);
        assert_eq!(std::fs::read(dirs.covers.join("1.jpg")).unwrap(), b"old");
        assert!(tmp_is_empty(&dirs));
    }

    #[tokio::test]
    async fn an_unreadable_journal_fails_recovery_and_stays() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, db, dirs) = setup().await;
        crash_mid_swap(&dirs, 1, 4);
        let journal = std::fs::read_dir(dirs.swap_dir())
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|e| e == "json"))
            .unwrap();
        let mode = |m| std::fs::Permissions::from_mode(m);
        std::fs::set_permissions(&journal, mode(0o000)).unwrap();
        let result = recover_cover_swaps(&db, &dirs).await;
        std::fs::set_permissions(&journal, mode(0o644)).unwrap();
        assert!(result.is_err());
        assert!(journal.exists());

        assert_eq!(recover_cover_swaps(&db, &dirs).await.unwrap(), 1);
        assert_eq!(std::fs::read(dirs.covers.join("1.jpg")).unwrap(), b"old");
    }

    #[tokio::test]
    async fn a_failed_rollback_marks_the_cover_unsettled() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, db, dirs) = setup().await;
        let mut unsettled = HashSet::new();
        let new = write_tmp(&dirs, 1, b"new").unwrap();
        let swap = CoverSwap::publish(&new, &dirs, 1, 4, &mut unsettled).unwrap();
        let mode = |m| std::fs::Permissions::from_mode(m);
        std::fs::set_permissions(dirs.swap_dir(), mode(0o555)).unwrap();
        swap.roll_back(&mut unsettled);
        std::fs::set_permissions(dirs.swap_dir(), mode(0o755)).unwrap();
        assert!(unsettled.contains(&1));

        assert_eq!(recover_cover_swaps(&db, &dirs).await.unwrap(), 1);
        assert_eq!(std::fs::read(dirs.covers.join("1.jpg")).unwrap(), b"old");
    }

    #[tokio::test]
    async fn an_unreadable_covers_directory_writes_no_journal() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, db, dirs) = setup().await;
        let new = write_tmp(&dirs, 1, b"new").unwrap();
        let mode = |m| std::fs::Permissions::from_mode(m);
        std::fs::set_permissions(&dirs.covers, mode(0o600)).unwrap();
        let published = CoverSwap::publish(&new, &dirs, 1, 4, &mut HashSet::new());
        std::fs::set_permissions(&dirs.covers, mode(0o755)).unwrap();
        assert!(published.is_err());

        assert_eq!(recover_cover_swaps(&db, &dirs).await.unwrap(), 0);
        assert_eq!(std::fs::read(dirs.covers.join("1.jpg")).unwrap(), b"old");
    }

    #[tokio::test]
    async fn recovery_waits_for_a_running_backup() {
        let (_dir, db, dirs) = setup().await;
        crash_mid_swap(&dirs, 1, 4);
        let backup = lock_covers(&dirs.covers).unwrap();
        let recovery = {
            let (db, dirs) = (db.clone(), dirs.clone());
            tokio::spawn(async move { recover_cover_swaps(&db, &dirs).await })
        };
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert_eq!(std::fs::read(dirs.covers.join("1.jpg")).unwrap(), b"new");
        drop(backup);
        assert_eq!(recovery.await.unwrap().unwrap(), 1);
        assert_eq!(std::fs::read(dirs.covers.join("1.jpg")).unwrap(), b"old");
    }

    #[tokio::test]
    async fn a_committed_swap_keeps_the_new_cover_at_startup() {
        let (_dir, db, dirs) = setup().await;
        crash_mid_swap(&dirs, 1, 4);
        sqlx::raw_sql("UPDATE works SET cover_version = 4 WHERE id = 1")
            .execute(&db.sqlite().unwrap().write)
            .await
            .unwrap();

        assert_eq!(recover_cover_swaps(&db, &dirs).await.unwrap(), 1);
        assert_eq!(std::fs::read(dirs.covers.join("1.jpg")).unwrap(), b"new");
        assert!(tmp_is_empty(&dirs));
    }
}
