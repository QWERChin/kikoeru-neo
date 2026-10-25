//! Lookups for the media endpoints: where a track's file is, and which
//! subtitle belongs to it.

use neo_core::db::media::*;

use super::detail::strip_extension;

/// The display name of the root a track's location is in, for messages
/// that tell the user which library failed.
///
/// # Errors
/// Any database error.
pub async fn root_name(read: &crate::Pool, track_id: i64) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT r.display_name
           FROM tracks t
           JOIN work_locations l ON l.id = t.location_id
           JOIN roots r ON r.id = l.root_id
           WHERE t.id = $1"#,
        track_id
    )
    .fetch_optional(read)
    .await
}

/// # Errors
/// Any database error.
pub async fn track_file(read: &crate::Pool, id: i64) -> Result<Option<TrackFile>, sqlx::Error> {
    sqlx::query_as!(
        TrackFile,
        r#"SELECT t.id AS "id!: i64", t.work_id AS "work_id!: i64",
                  t.location_id AS "location_id!: i64", t.kind, t.display_path, t.rel_path,
                  t.size, t.mtime, t.duration_ms, t.probe,
                  (t.missing_at IS NOT NULL OR l.missing_at IS NOT NULL) AS "missing!: bool",
                  l.root_id, l.rel_path AS location_path, r.source_version
           FROM tracks t JOIN work_locations l ON l.id = t.location_id
           JOIN roots r ON r.id = l.root_id
           WHERE t.id = $1"#,
        id
    )
    .fetch_optional(read)
    .await
}

/// The subtitle file for a track, using the same rule as `has_subtitle`
/// in the work detail: same location and directory, named after the track
/// with its extension replaced (`Tr01.lrc`) or added (`Tr01.mp3.vtt`).
/// When several match, WebVTT is preferred, then srt, lrc and ass.
///
/// # Errors
/// Any database error.
pub async fn subtitle_for(
    read: &crate::Pool,
    track: &TrackFile,
    extensions: &[&str],
) -> Result<Option<i64>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT id AS "id!: i64", rel_path FROM tracks
           WHERE location_id = $1 AND kind = 'subtitle' AND missing_at IS NULL"#,
        track.location_id
    )
    .fetch_all(read)
    .await?;
    let stem = strip_extension(&track.rel_path);
    let rank = |path: &[u8]| -> Option<usize> {
        let base = strip_extension(path)?;
        if base != track.rel_path.as_slice() && Some(base) != stem {
            return None;
        }
        let ext = String::from_utf8_lossy(&path[base.len() + 1..]).to_ascii_lowercase();
        extensions.iter().position(|e| *e == ext)
    };
    Ok(rows
        .iter()
        .filter_map(|r| rank(&r.rel_path).map(|rank| (rank, r.id)))
        .min()
        .map(|(_, id)| id))
}

const FONT_EXTENSIONS: [&str; 5] = ["ttf", "otf", "ttc", "woff", "woff2"];

/// Font files of a work for JASSUB: `(track id, display path)` in natural
/// order. Hidden duplicates and missing files are left out.
///
/// # Errors
/// Any database error.
pub async fn work_fonts(
    read: &crate::Pool,
    work_id: i64,
) -> Result<Vec<(i64, String)>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT t.id AS "id!: i64", t.display_path
           FROM tracks t JOIN work_locations l ON l.id = t.location_id
           WHERE t.work_id = $1 AND t.duplicate_of IS NULL
             AND t.missing_at IS NULL AND l.missing_at IS NULL
           ORDER BY t.sort_key, t.id"#,
        work_id
    )
    .fetch_all(read)
    .await?;
    Ok(rows
        .into_iter()
        .filter(|r| {
            r.display_path.rsplit_once('.').is_some_and(|(_, ext)| {
                FONT_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())
            })
        })
        .take(MAX_FONTS)
        .map(|r| (r.id, r.display_path))
        .collect())
}

/// Stores a probe result, unless the file changed since the row was read:
/// a probe of an older version must not describe the new one.
///
/// # Errors
/// Any database error.
pub async fn save_probe(
    write: &crate::Pool,
    track: &TrackFile,
    probe_json: &str,
    duration_ms: Option<i64>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE tracks
         SET probe = $1, duration_ms = COALESCE($2, duration_ms),
             duration_src = CASE WHEN $2 IS NULL THEN duration_src ELSE 'probe' END
         WHERE id = $3 AND size IS NOT DISTINCT FROM $4 AND mtime IS NOT DISTINCT FROM $5
           AND EXISTS (SELECT 1 FROM work_locations l JOIN roots r ON r.id = l.root_id
                       WHERE l.id = tracks.location_id AND r.source_version = $6)",
        probe_json,
        duration_ms,
        track.id,
        track.size,
        track.mtime,
        track.source_version
    )
    .execute(write)
    .await?;
    Ok(())
}

/// The work's code, or `None` when there is no such work.
///
/// # Errors
/// Any database error.
pub async fn work_code(read: &crate::Pool, work_id: i64) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar!("SELECT code FROM works WHERE id = $1", work_id)
        .fetch_optional(read)
        .await
}

/// Every file of a work that a download includes: present, in a present
/// location, and not a duplicate of a file in another location. Sorted the
/// way the work detail lists them.
///
/// # Errors
/// Any database error.
pub async fn work_files(read: &crate::Pool, work_id: i64) -> Result<Vec<TrackFile>, sqlx::Error> {
    sqlx::query_as!(
        TrackFile,
        r#"SELECT t.id AS "id!: i64", t.work_id AS "work_id!: i64",
                  t.location_id AS "location_id!: i64", t.kind, t.display_path, t.rel_path,
                  t.size, t.mtime, t.duration_ms, t.probe,
                  (t.missing_at IS NOT NULL OR l.missing_at IS NOT NULL) AS "missing!: bool",
                  l.root_id, l.rel_path AS location_path, r.source_version
           FROM tracks t JOIN work_locations l ON l.id = t.location_id
           JOIN roots r ON r.id = l.root_id
           WHERE t.work_id = $1 AND t.duplicate_of IS NULL
             AND t.missing_at IS NULL AND l.missing_at IS NULL
           ORDER BY t.sort_key, t.id"#,
        work_id
    )
    .fetch_all(read)
    .await
}

/// The source version, size and modification time that key a track's probe.
///
/// # Errors
/// Any database error.
pub async fn track_version(
    read: &crate::Pool,
    id: i64,
) -> Result<Option<(i64, Option<i64>, Option<i64>)>, sqlx::Error> {
    Ok(sqlx::query!(
        "SELECT r.source_version, t.size, t.mtime FROM tracks t
             JOIN work_locations l ON l.id = t.location_id JOIN roots r ON r.id = l.root_id
             WHERE t.id = $1",
        id
    )
    .fetch_optional(read)
    .await?
    .map(|r| (r.source_version, r.size, r.mtime)))
}

/// The current source version, for pairing a row with its storage backend.
///
/// # Errors
/// Any database error.
pub async fn source_version(read: &crate::Pool, root_id: &str) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar!("SELECT source_version FROM roots WHERE id = $1", root_id)
        .fetch_optional(read)
        .await
}

/// # Errors
/// Any database error.
pub async fn cover_state(
    read: &crate::Pool,
    work_id: i64,
) -> Result<Option<CoverState>, sqlx::Error> {
    sqlx::query_as!(
        CoverState,
        r#"SELECT cover_src <> 'none' AS "has_cover!: bool", cover_version FROM works WHERE id = $1"#,
        work_id
    )
    .fetch_optional(read)
    .await
}
