//! Stage 1: find every work directory in a root. A directory whose name
//! holds a work code is a work, and the scanner does not look inside it
//! here. Other directories are searched further, so works may sit several
//! levels down.

use std::collections::HashSet;

use neo_storage::{EntryKind, RawPath, Storage, StorageError};

use crate::{Observer, Stage, Summary, names};

/// Deeper trees are almost certainly a mistake such as a directory loop on
/// a remote, and walking them would only cost time.
const MAX_DEPTH: usize = 16;

pub struct Found {
    pub rel_path: RawPath,
    pub display: String,
    pub code: String,
}

pub enum Failure {
    Storage(StorageError),
    Cancelled,
}

/// Lists the whole root before returning anything. Any failed listing
/// fails the stage, because a work missing from a partial list would be
/// marked missing.
pub async fn discover(
    storage: &dyn Storage,
    observer: &dyn Observer,
    summary: &mut Summary,
) -> Result<Vec<Found>, Failure> {
    let mut found = Vec::new();
    // OpenList can list one name twice. A work found twice would insert
    // its location twice and break the unique path, failing the scan, and
    // a directory queued twice would be walked twice.
    let mut seen: HashSet<Vec<u8>> = HashSet::new();
    let mut queue = vec![(RawPath::root(), 0usize)];
    let mut listed = 0u64;
    while let Some((dir, depth)) = queue.pop() {
        if observer.is_cancelled() {
            return Err(Failure::Cancelled);
        }
        let entries = storage.list(&dir).await.map_err(Failure::Storage)?;
        listed += 1;
        observer.progress(Stage::Discover, listed, listed + queue.len() as u64);
        let mut subdirs = Vec::new();
        for entry in entries {
            let path = dir.join(&entry.name).map_err(|e| {
                Failure::Storage(StorageError::Upstream(format!("bad name in listing: {e}")))
            })?;
            let display = names::display_text(path.as_bytes());
            match entry.kind {
                EntryKind::Dir if !seen.insert(path.as_bytes().to_vec()) => {
                    summary.log(format!("listed twice, kept the first: {display}"));
                }
                EntryKind::Dir => {
                    let name = names::display_text(&entry.name);
                    if let Some(code) = names::work_code(&name) {
                        found.push(Found {
                            rel_path: path,
                            display,
                            code,
                        });
                    } else if depth + 1 < MAX_DEPTH {
                        subdirs.push((path, depth + 1));
                    } else {
                        summary.log(format!("not searched, too deep: {display}"));
                    }
                }
                EntryKind::Symlink => summary.log(format!("skipped symlink: {display}")),
                EntryKind::File | EntryKind::Other => {}
            }
        }
        queue.extend(subdirs.into_iter().rev());
    }
    Ok(found)
}
