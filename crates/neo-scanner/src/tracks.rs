//! Stage 2: sync one location's file tree into `tracks`, then decide which
//! copy of a file present in several locations is shown.

use std::collections::{HashMap, HashSet};

use neo_db::{
    Db, Tx,
    scan::{self, CopyRow, ListedFile},
};
use neo_storage::{Entry, EntryKind, StorageError, WalkEntry};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    Observer, Root, Summary,
    names::{self, Kind},
    works::Present,
};

struct Existing {
    id: i64,
    size: Option<i64>,
    mtime: Option<i64>,
    missing: bool,
    probed: bool,
}

/// What listing a location found, before anything is written.
pub enum Listing {
    /// The top level matches the digest of the last full walk.
    Unchanged,
    Tree {
        entries: Vec<WalkEntry>,
        reprobe: bool,
    },
    Failed(StorageError),
}

/// The digest each location had after its last full walk. Read once before
/// listing starts, so listings never hold a database connection.
pub async fn stored_digests(db: &Db, root: &Root<'_>) -> Result<HashMap<i64, String>, sqlx::Error> {
    Ok(scan::stored_digests(db, root.id)
        .await?
        .into_iter()
        .collect())
}

/// Lists a location's whole tree, or only its top level when that matches
/// `stored`. It touches neither the database nor anything shared, so a
/// scan can list several locations at once and still write them one by
/// one in a fixed order.
pub async fn list_location(root: &Root<'_>, location: &Present, stored: Option<&str>) -> Listing {
    if root.skip_unchanged
        && let Some(stored) = stored
    {
        match root.storage.list(&location.rel_path).await {
            Ok(entries) if listing_digest(&entries) == stored => return Listing::Unchanged,
            Ok(_) => {}
            Err(e) => return Listing::Failed(e),
        }
    }
    match root.storage.walk(&location.rel_path).await {
        Ok(entries) => Listing::Tree {
            entries,
            reprobe: stored.is_none(),
        },
        Err(e) => Listing::Failed(e),
    }
}

/// Writes a listing in one transaction. A failed listing leaves every
/// track of the location as it was, because a partial tree would mark the
/// rest missing. Returns whether the location counts as synced.
pub async fn apply_listing(
    db: &Db,
    root: &Root<'_>,
    location: &Present,
    listing: Listing,
    now: i64,
    observer: &dyn Observer,
    summary: &mut Summary,
) -> Result<bool, sqlx::Error> {
    let (tree, reprobe) = match listing {
        Listing::Unchanged => {
            summary.works_unchanged += 1;
            return Ok(true);
        }
        Listing::Tree { entries, reprobe } => (entries, reprobe),
        Listing::Failed(e) => return not_listed(location, &e, summary),
    };
    let top: Vec<Entry> = tree
        .iter()
        .filter(|item| !item.path.as_bytes().contains(&b'/'))
        .map(|item| item.entry.clone())
        .collect();
    let digest = listing_digest(&top);
    let existing: HashMap<Vec<u8>, Existing> = scan::location_tracks(db, location.location_id)
        .await?
        .into_iter()
        .map(|r| {
            (
                r.rel_path,
                Existing {
                    id: r.id,
                    size: r.size,
                    mtime: r.mtime,
                    missing: r.missing,
                    probed: r.probed,
                },
            )
        })
        .collect();
    let durations = legacy_durations(db, location.work_id).await?;

    let mut seen = HashSet::new();
    let mut probe_ids = Vec::new();
    let mut tx = db.begin().await?;
    for item in tree {
        match item.entry.kind {
            EntryKind::File => {}
            EntryKind::Dir => continue,
            EntryKind::Symlink | EntryKind::Other => {
                summary.log(format!(
                    "skipped {}: {}/{}",
                    if item.entry.kind == EntryKind::Symlink {
                        "symlink"
                    } else {
                        "special file"
                    },
                    location.display,
                    names::display_text(item.path.as_bytes())
                ));
                continue;
            }
        }
        let rel = item.path.as_bytes();
        // OpenList can list one name twice. The first entry wins; a second
        // insert would break the location's unique path and fail the scan.
        if !seen.insert(rel.to_vec()) {
            summary.log(format!(
                "listed twice, kept the first: {}/{}",
                location.display,
                names::display_text(rel)
            ));
            continue;
        }
        let display = names::display_text(rel);
        let sort_key = names::sort_key(&display);
        let kind = Kind::of(&display);
        let kind_str = kind.as_str();
        let size = item.entry.size.and_then(|s| i64::try_from(s).ok());
        let mtime = item.entry.mtime_ms;
        let probe = root.probe_after_scan && matches!(kind, Kind::Audio | Kind::Video);
        let file = ListedFile {
            display: &display,
            sort_key: &sort_key,
            kind: kind_str,
            size,
            mtime,
        };
        match existing.get(rel) {
            Some(old) if old.size != size || old.mtime != mtime => {
                scan::update_changed_track(&mut tx, old.id, file).await?;
                summary.tracks_changed += 1;
                if probe {
                    probe_ids.push(old.id);
                }
            }
            Some(old) => {
                if old.missing {
                    scan::restore_track(&mut tx, old.id).await?;
                    summary.tracks_restored += 1;
                }
                // Relocation clears the listing digest. A NULL probe alone
                // can also mean ffprobe failed on an unchanged file.
                if probe && reprobe && !old.probed {
                    probe_ids.push(old.id);
                }
            }
            None => {
                let imported = durations.get(&display).copied();
                let id = scan::insert_track(
                    &mut tx,
                    location.work_id,
                    location.location_id,
                    rel,
                    file,
                    imported,
                )
                .await?;
                summary.tracks_new += 1;
                if probe {
                    probe_ids.push(id);
                }
            }
        }
    }
    for (rel, old) in &existing {
        if !old.missing && !seen.contains(rel) {
            scan::mark_track_missing(&mut tx, old.id, now).await?;
            summary.tracks_missing += 1;
        }
    }
    scan::set_listed(&mut tx, location.location_id, now, &digest).await?;
    tx.commit().await?;
    if !probe_ids.is_empty() {
        observer.probe(&probe_ids);
        summary.probe.extend(probe_ids);
    }
    Ok(true)
}

fn not_listed(
    location: &Present,
    error: &StorageError,
    summary: &mut Summary,
) -> Result<bool, sqlx::Error> {
    summary.works_failed += 1;
    summary.log(format!(
        "{}: not listed, tracks unchanged: {error}",
        location.display
    ));
    Ok(false)
}

/// A digest of one directory level: every entry's name, type, size,
/// modification time and ETag when there is one, independent of listing
/// order. A subdirectory's time
/// stands in for its contents, so a change deep inside a work is seen only
/// when the backend updates the times of the directories above it.
pub(crate) fn listing_digest(entries: &[Entry]) -> String {
    let mut sorted: Vec<&Entry> = entries.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    let mut hasher = Sha256::new();
    for entry in sorted {
        hasher.update((entry.name.len() as u64).to_le_bytes());
        hasher.update(&entry.name);
        hasher.update([entry.kind as u8]);
        // A leading flag keeps "unknown" apart from every real value.
        match entry.size {
            Some(size) => hasher.update([&[1u8][..], &size.to_le_bytes()].concat()),
            None => hasher.update([0u8]),
        }
        match entry.mtime_ms {
            Some(mtime) => hasher.update([&[1u8][..], &mtime.to_le_bytes()].concat()),
            None => hasher.update([0u8]),
        }
        // Only WebDAV entries carry an ETag. Without one nothing is added,
        // so local and OpenList digests stay the same as before.
        if let Some(etag) = &entry.etag {
            hasher.update((etag.len() as u64).to_le_bytes());
            hasher.update(etag.as_bytes());
        }
    }
    format!("{:x}", hasher.finalize())
}

/// The import keeps the old server's durations in `extra.legacy_durations`,
/// keyed by the path relative to the work directory, in seconds.
async fn legacy_durations(db: &Db, work_id: i64) -> Result<HashMap<String, i64>, sqlx::Error> {
    let extra = scan::work_extra(db, work_id).await?;
    let Ok(Value::Object(mut extra)) = serde_json::from_str::<Value>(&extra) else {
        return Ok(HashMap::new());
    };
    let Some(Value::Object(map)) = extra.remove("legacy_durations") else {
        return Ok(HashMap::new());
    };
    Ok(map
        .into_iter()
        .filter_map(|(path, secs)| {
            let secs = secs.as_f64().filter(|s| s.is_finite() && *s >= 0.0)?;
            // Track lengths are far below 2^53 ms, so the cast is exact.
            Some((
                names::display_text(path.as_bytes()),
                (secs * 1000.0).round() as i64,
            ))
        })
        .collect())
}

/// Files with the same relative path and size in several locations of one
/// work are one file. The copy in the most preferred root is kept, and the
/// others point at it with `duplicate_of`. When the kept copy goes missing,
/// another copy takes over and play progress follows it.
pub async fn dedupe_work(
    tx: &mut Tx<'_>,
    work_id: i64,
    root_order: &[String],
) -> Result<(), sqlx::Error> {
    let copies = scan::work_copies(tx, work_id).await?;
    let rank = |c: &CopyRow| {
        let root = root_order
            .iter()
            .position(|r| *r == c.root_id)
            .unwrap_or(usize::MAX);
        (root, c.location_id, c.id)
    };
    // The kept copy of each (path, size) among the tracks that are present.
    let mut keep: HashMap<(&[u8], i64), &CopyRow> = HashMap::new();
    for c in copies.iter().filter(|c| !c.missing) {
        let Some(size) = c.size else { continue };
        let slot = keep.entry((c.rel_path.as_slice(), size)).or_insert(c);
        if rank(c) < rank(slot) {
            *slot = c;
        }
    }
    for c in &copies {
        let kept = c
            .size
            .and_then(|size| keep.get(&(c.rel_path.as_slice(), size)))
            .map(|k| k.id)
            .filter(|&k| k != c.id);
        if kept != c.duplicate_of {
            scan::set_duplicate_of(tx, c.id, kept).await?;
        }
        // Even when the link did not change: a player may have reported
        // the hidden copy's ID since the last scan.
        if let Some(kept) = kept {
            scan::move_progress(tx, c.id, kept).await?;
        }
    }
    Ok(())
}
