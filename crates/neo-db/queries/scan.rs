//! The scan's queries (book/src/scanning.md). neo-scanner decides what to
//! write; each function here is one statement, or two that always go
//! together.

use neo_core::db::scan::*;

/// # Errors
/// Any database error.
pub async fn root_exists(read: &crate::Pool, root_id: &str) -> sqlx::Result<bool> {
    Ok(
        sqlx::query_scalar!("SELECT id FROM roots WHERE id = $1", root_id)
            .fetch_optional(read)
            .await?
            .is_some(),
    )
}

/// # Errors
/// Any database error.
pub async fn set_root_status(
    write: &crate::Pool,
    root_id: &str,
    status: &str,
    scanned_at: Option<i64>,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE roots SET status = $1, last_scan_at = COALESCE($2, last_scan_at) WHERE id = $3",
        status,
        scanned_at,
        root_id
    )
    .execute(write)
    .await?;
    Ok(())
}

/// # Errors
/// Any database error.
pub async fn root_locations(read: &crate::Pool, root_id: &str) -> sqlx::Result<Vec<LocationRow>> {
    sqlx::query_as!(
        LocationRow,
        r#"SELECT id AS "id!", work_id, rel_path, missing_at IS NOT NULL AS "missing!: bool"
           FROM work_locations WHERE root_id = $1"#,
        root_id
    )
    .fetch_all(read)
    .await
}

/// # Errors
/// Any database error.
pub async fn restore_location(tx: &mut crate::Tx<'_>, location_id: i64) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE work_locations SET missing_at = NULL WHERE id = $1",
        location_id
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// # Errors
/// Any database error.
pub async fn work_by_code(tx: &mut crate::Tx<'_>, code: &str) -> sqlx::Result<Option<i64>> {
    sqlx::query_scalar!("SELECT id FROM works WHERE code = $1", code)
        .fetch_optional(&mut **tx)
        .await
}

/// A work without metadata yet. Its title is the directory name.
///
/// # Errors
/// Any database error.
pub async fn insert_work(
    tx: &mut crate::Tx<'_>,
    code: &str,
    title: &str,
    now: i64,
) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"INSERT INTO works (code, title, metadata_src, created_at, updated_at)
           VALUES ($1, $2, 'none', $3, $4) RETURNING id AS "id!""#,
        code,
        title,
        now,
        now
    )
    .fetch_one(&mut **tx)
    .await
}

/// # Errors
/// Any database error.
pub async fn insert_location(
    tx: &mut crate::Tx<'_>,
    work_id: i64,
    root_id: &str,
    rel_path: &[u8],
    display: &str,
) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"INSERT INTO work_locations (work_id, root_id, rel_path, display_path)
           VALUES ($1, $2, $3, $4) RETURNING id AS "id!""#,
        work_id,
        root_id,
        rel_path,
        display
    )
    .fetch_one(&mut **tx)
    .await
}

/// Marks a location and its tracks missing.
///
/// # Errors
/// Any database error.
pub async fn mark_location_missing(
    tx: &mut crate::Tx<'_>,
    location_id: i64,
    now: i64,
) -> sqlx::Result<()> {
    sqlx::query!(
        // Its tracks are marked missing below, so the next scan that finds
        // it again must walk it rather than trust the stored digest.
        "UPDATE work_locations SET missing_at = $1, listing_hash = NULL WHERE id = $2",
        now,
        location_id
    )
    .execute(&mut **tx)
    .await?;
    // A missing location's tracks cannot be played, so progress on them
    // must not count as resumable.
    sqlx::query!(
        "UPDATE tracks SET missing_at = $1 WHERE location_id = $2 AND missing_at IS NULL",
        now,
        location_id
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// # Errors
/// Any database error.
pub async fn work_state(tx: &mut crate::Tx<'_>, work_id: i64) -> sqlx::Result<WorkState> {
    sqlx::query_as!(
        WorkState,
        r#"SELECT missing_at IS NOT NULL AS "missing!: bool",
                  EXISTS (SELECT 1 FROM work_locations
                          WHERE work_id = works.id AND missing_at IS NULL) AS "present!: bool"
           FROM works WHERE id = $1"#,
        work_id
    )
    .fetch_one(&mut **tx)
    .await
}

/// Sets or clears a work's missing time.
///
/// # Errors
/// Any database error.
pub async fn set_work_missing(
    tx: &mut crate::Tx<'_>,
    work_id: i64,
    missing_at: Option<i64>,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE works SET missing_at = $1 WHERE id = $2",
        missing_at,
        work_id
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// The digest of every location of a root that has one.
///
/// # Errors
/// Any database error.
pub async fn stored_digests(read: &crate::Pool, root_id: &str) -> sqlx::Result<Vec<(i64, String)>> {
    Ok(sqlx::query!(
        r#"SELECT id AS "id!", listing_hash AS "hash!" FROM work_locations
           WHERE root_id = $1 AND listing_hash IS NOT NULL"#,
        root_id
    )
    .fetch_all(read)
    .await?
    .into_iter()
    .map(|r| (r.id, r.hash))
    .collect())
}

/// # Errors
/// Any database error.
pub async fn location_tracks(read: &crate::Pool, location_id: i64) -> sqlx::Result<Vec<TrackRow>> {
    sqlx::query_as!(
        TrackRow,
        r#"SELECT id AS "id!", rel_path, size, mtime, missing_at IS NOT NULL AS "missing!: bool",
                  probe IS NOT NULL AS "probed!: bool"
           FROM tracks WHERE location_id = $1"#,
        location_id
    )
    .fetch_all(read)
    .await
}

/// The work's `extra` JSON.
///
/// # Errors
/// Any database error, or no such work.
pub async fn work_extra(read: &crate::Pool, work_id: i64) -> sqlx::Result<String> {
    sqlx::query_scalar!("SELECT extra FROM works WHERE id = $1", work_id)
        .fetch_one(read)
        .await
}

/// A file whose size or time changed. Earlier probe results and caches no
/// longer apply, but an imported duration stays as a hint.
///
/// # Errors
/// Any database error.
pub async fn update_changed_track(
    tx: &mut crate::Tx<'_>,
    track_id: i64,
    file: ListedFile<'_>,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE tracks SET size = $1, mtime = $2, display_path = $3, sort_key = $4,
             kind = $5, probe = NULL, missing_at = NULL,
             duration_ms = CASE WHEN duration_src = 'imported' THEN duration_ms END,
             duration_src = CASE WHEN duration_src = 'imported' THEN duration_src END
         WHERE id = $6",
        file.size,
        file.mtime,
        file.display,
        file.sort_key,
        file.kind,
        track_id
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// # Errors
/// Any database error.
pub async fn restore_track(tx: &mut crate::Tx<'_>, track_id: i64) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE tracks SET missing_at = NULL WHERE id = $1",
        track_id
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// # Errors
/// Any database error.
pub async fn insert_track(
    tx: &mut crate::Tx<'_>,
    work_id: i64,
    location_id: i64,
    rel_path: &[u8],
    file: ListedFile<'_>,
    imported_duration_ms: Option<i64>,
) -> sqlx::Result<i64> {
    let duration_src = imported_duration_ms.map(|_| "imported");
    sqlx::query_scalar!(
        r#"INSERT INTO tracks (work_id, location_id, rel_path, display_path, sort_key,
                               kind, size, mtime, duration_ms, duration_src)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING id AS "id!""#,
        work_id,
        location_id,
        rel_path,
        file.display,
        file.sort_key,
        file.kind,
        file.size,
        file.mtime,
        imported_duration_ms,
        duration_src
    )
    .fetch_one(&mut **tx)
    .await
}

/// # Errors
/// Any database error.
pub async fn mark_track_missing(
    tx: &mut crate::Tx<'_>,
    track_id: i64,
    now: i64,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE tracks SET missing_at = $1 WHERE id = $2",
        now,
        track_id
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Records a full walk of a location and the digest of its top level.
///
/// # Errors
/// Any database error.
pub async fn set_listed(
    tx: &mut crate::Tx<'_>,
    location_id: i64,
    now: i64,
    digest: &str,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE work_locations SET listed_at = $1, listing_hash = $2 WHERE id = $3",
        now,
        digest,
        location_id
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Every copy of every file of a work.
///
/// # Errors
/// Any database error.
pub async fn work_copies(tx: &mut crate::Tx<'_>, work_id: i64) -> sqlx::Result<Vec<CopyRow>> {
    sqlx::query_as!(
        CopyRow,
        r#"SELECT t.id AS "id!", t.location_id, l.root_id, t.rel_path, t.size,
                  t.missing_at IS NOT NULL AS "missing!: bool", t.duplicate_of
           FROM tracks t JOIN work_locations l ON l.id = t.location_id
           WHERE t.work_id = $1"#,
        work_id
    )
    .fetch_all(&mut **tx)
    .await
}

/// # Errors
/// Any database error.
pub async fn set_duplicate_of(
    tx: &mut crate::Tx<'_>,
    track_id: i64,
    kept: Option<i64>,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE tracks SET duplicate_of = $1 WHERE id = $2",
        kept,
        track_id
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Moves play progress on a hidden copy to the copy that is shown.
///
/// # Errors
/// Any database error.
pub async fn move_progress(tx: &mut crate::Tx<'_>, from: i64, to: i64) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE play_progress SET track_id = $1 WHERE track_id = $2",
        to,
        from
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// # Errors
/// Any database error.
pub async fn pending_imports(
    tx: &mut crate::Tx<'_>,
    work_id: i64,
) -> sqlx::Result<Vec<PendingRow>> {
    sqlx::query_as!(
        PendingRow,
        "SELECT user_id, source FROM import_pending WHERE work_id = $1 AND state = 'pending'",
        work_id
    )
    .fetch_all(&mut **tx)
    .await
}

/// Present locations of the work that no scan has listed yet, in any root.
///
/// # Errors
/// Any database error.
pub async fn unlisted_locations(tx: &mut crate::Tx<'_>, work_id: i64) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "n!: i64" FROM work_locations
           WHERE work_id = $1 AND missing_at IS NULL AND listed_at IS NULL"#,
        work_id
    )
    .fetch_one(&mut **tx)
    .await
}

/// The track of the user's progress on the work: `None` without progress,
/// `Some(None)` for progress that has no track yet.
///
/// # Errors
/// Any database error.
pub async fn progress_track(
    tx: &mut crate::Tx<'_>,
    user_id: i64,
    work_id: i64,
) -> sqlx::Result<Option<Option<i64>>> {
    sqlx::query_scalar!(
        "SELECT track_id FROM play_progress WHERE user_id = $1 AND work_id = $2",
        user_id,
        work_id
    )
    .fetch_optional(&mut **tx)
    .await
}

/// Present, shown tracks of the work at `display_path`.
///
/// # Errors
/// Any database error.
pub async fn tracks_at(
    tx: &mut crate::Tx<'_>,
    work_id: i64,
    display_path: &str,
) -> sqlx::Result<Vec<i64>> {
    sqlx::query_scalar!(
        r#"SELECT id AS "id!" FROM tracks
           WHERE work_id = $1 AND display_path = $2
             AND missing_at IS NULL AND duplicate_of IS NULL"#,
        work_id,
        display_path
    )
    .fetch_all(&mut **tx)
    .await
}

/// Puts a matched track on progress that has none yet.
///
/// # Errors
/// Any database error.
pub async fn set_progress_track(
    tx: &mut crate::Tx<'_>,
    user_id: i64,
    work_id: i64,
    track_id: i64,
    group_path: &str,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE play_progress SET track_id = $1, group_path = $2
         WHERE user_id = $3 AND work_id = $4 AND track_id IS NULL",
        track_id,
        group_path,
        user_id,
        work_id
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// # Errors
/// Any database error.
pub async fn set_pending_state(
    tx: &mut crate::Tx<'_>,
    user_id: i64,
    work_id: i64,
    state: &str,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE import_pending SET state = $1 WHERE user_id = $2 AND work_id = $3",
        state,
        user_id,
        work_id
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}
