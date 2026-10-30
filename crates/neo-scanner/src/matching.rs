//! Matches imported play positions to tracks after a work is scanned, as
//! "扫描之后补匹配" in book/src/migration.md describes. The material stays
//! in `import_pending`, so a later scan can still match what this one
//! could not.

use neo_db::{Tx, scan};
use serde_json::Value;
use unicode_normalization::UnicodeNormalization;

use crate::Summary;

/// The old queue item's path relative to the work directory, and its
/// directory, which becomes the play group.
fn legacy_path(source: &str) -> Option<(String, String)> {
    let source: Value = serde_json::from_str(source).ok()?;
    let item = source.get("item")?;
    let title = item.get("title")?.as_str()?;
    let dir = item.get("subtitle").and_then(Value::as_str).unwrap_or("");
    let dir: String = dir.nfc().collect();
    let title: String = title.nfc().collect();
    let path = if dir.is_empty() {
        title
    } else {
        format!("{dir}/{title}")
    };
    Some((path, dir))
}

pub async fn match_work(
    tx: &mut Tx<'_>,
    work_id: i64,
    summary: &mut Summary,
) -> Result<(), sqlx::Error> {
    let pending = scan::pending_imports(tx, work_id).await?;
    if pending.is_empty() {
        return Ok(());
    }
    // Every present location needs a complete listing. An unscanned or
    // partially listed location may hold a second candidate for an old
    // path, and a match is never revisited.
    let unlisted = scan::unlisted_locations(tx, work_id).await?;
    if unlisted > 0 {
        summary.pending_deferred += pending.len() as u64;
        return Ok(());
    }
    for row in pending {
        // The import leaves track_id empty, and only playback in the new
        // server sets it. A set track or a cleared row means the user has
        // moved on, so the old position must not overwrite it.
        let progress = scan::progress_track(tx, row.user_id, work_id).await?;
        let state = match progress {
            None | Some(Some(_)) => {
                summary.pending_superseded += 1;
                "superseded"
            }
            Some(None) => {
                let Some((path, dir)) = legacy_path(&row.source) else {
                    summary.log(format!(
                        "import_pending user {} work {work_id}: source has no queue item",
                        row.user_id
                    ));
                    continue;
                };
                // Only tracks that are present and shown count. A copy in a
                // second location is the same file, not a second candidate.
                let candidates = scan::tracks_at(tx, work_id, &path).await?;
                match candidates.as_slice() {
                    [] => {
                        summary.pending_not_found += 1;
                        continue;
                    }
                    [track_id] => {
                        scan::set_progress_track(tx, row.user_id, work_id, *track_id, &dir).await?;
                        summary.pending_matched += 1;
                        "matched"
                    }
                    _ => {
                        summary.pending_ambiguous += 1;
                        "ambiguous"
                    }
                }
            }
        };
        scan::set_pending_state(tx, row.user_id, work_id, state).await?;
    }
    Ok(())
}
