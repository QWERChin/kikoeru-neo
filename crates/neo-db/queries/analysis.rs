//! Measured loudness per track (`track_analysis`, book/src/data-model.md).
//! A row describes the file at the size and mtime it was measured at, so
//! a changed file reads as not measured.

use neo_core::db::analysis::*;

/// The loudness measured for the track's file at `size` and `mtime`.
/// Without an mtime, only a measurement younger than
/// [`UNKNOWN_MTIME_TTL_MS`] counts.
///
/// # Errors
/// Any database error.
pub async fn loudness(
    read: &crate::Pool,
    track_id: i64,
    size: Option<i64>,
    mtime: Option<i64>,
    now: i64,
) -> Result<Option<StoredLoudness>, sqlx::Error> {
    let fresh_since = now - UNKNOWN_MTIME_TTL_MS;
    sqlx::query_as!(
        StoredLoudness,
        "SELECT integrated_lufs, true_peak_dbtp FROM track_analysis
         WHERE track_id = $1 AND size IS NOT DISTINCT FROM $2 AND mtime IS NOT DISTINCT FROM $3
           AND (mtime IS NOT NULL OR analyzed_at >= $4)",
        track_id,
        size,
        mtime,
        fresh_since
    )
    .fetch_optional(read)
    .await
}

/// Stores a measurement of the file at `size` and `mtime`, replacing any
/// earlier one. These are the file's values when it was measured, which
/// can be newer than the track row's until the next scan. Nothing is
/// stored for a track that no longer exists or whose root has moved.
///
/// # Errors
/// Any database error.
pub async fn save(
    write: &crate::Pool,
    track: &neo_core::db::media::TrackFile,
    size: Option<i64>,
    mtime: Option<i64>,
    loudness: StoredLoudness,
    now: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO track_analysis
             (track_id, size, mtime, integrated_lufs, true_peak_dbtp, analyzed_at)
         SELECT t.id, $1, $2, $3, $4, $5 FROM tracks t
         JOIN work_locations l ON l.id = t.location_id JOIN roots r ON r.id = l.root_id
         WHERE t.id = $6 AND r.source_version = $7
         ON CONFLICT (track_id) DO UPDATE SET
             size = excluded.size, mtime = excluded.mtime,
             integrated_lufs = excluded.integrated_lufs,
             true_peak_dbtp = excluded.true_peak_dbtp,
             analyzed_at = excluded.analyzed_at",
        size,
        mtime,
        loudness.integrated_lufs,
        loudness.true_peak_dbtp,
        now,
        track.id,
        track.source_version
    )
    .execute(write)
    .await?;
    Ok(())
}
