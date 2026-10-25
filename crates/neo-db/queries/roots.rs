use neo_core::db::roots::*;

/// Brings `roots` in line with the config at startup. New roots are added
/// and display names follow the config. A root that left the config keeps
/// its row, because its locations and tracks still refer to it, and a
/// renamed ID is a different root.
pub async fn sync(pool: &crate::Pool, roots: &[(&str, &str)]) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    for (id, display_name) in roots {
        sqlx::query!(
            "INSERT INTO roots (id, display_name) VALUES ($1, $2)
             ON CONFLICT (id) DO UPDATE SET display_name = excluded.display_name",
            id,
            display_name
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

/// Roots that still have works present but are not in `known`, with how
/// many works each holds. An imported library whose roots never made it
/// into config.toml shows up here, and its works stay unplayable.
///
/// # Errors
/// Any database error.
pub async fn with_works_outside(
    pool: &crate::Pool,
    known: &[&str],
) -> sqlx::Result<Vec<(String, i64)>> {
    let counts: Vec<(String, i64)> = sqlx::query_as(
        "SELECT root_id, COUNT(DISTINCT work_id) FROM work_locations
         WHERE missing_at IS NULL GROUP BY root_id ORDER BY root_id",
    )
    .fetch_all(pool)
    .await?;
    Ok(counts
        .into_iter()
        .filter(|(id, _)| !known.contains(&id.as_str()))
        .collect())
}

/// Roots added through the API, oldest first.
///
/// # Errors
/// Any database error.
pub async fn list_stored(pool: &crate::Pool) -> sqlx::Result<Vec<StoredRoot>> {
    sqlx::query_as!(
        StoredRoot,
        r#"SELECT s.root_id AS "id!", r.display_name, s.kind, s.settings
           FROM root_settings s JOIN roots r ON r.id = s.root_id
           ORDER BY s.created_at, s.root_id"#
    )
    .fetch_all(pool)
    .await
}

/// Adds a root. Returns false when a root with this id already has
/// settings. A `roots` row left by a removed root is reused, so its
/// locations come back on the next scan.
///
/// # Errors
/// Any database error.
pub async fn insert_stored(pool: &crate::Pool, root: &StoredRoot, now: i64) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    let exists = sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM root_settings WHERE root_id = $1) AS "e!: bool""#,
        root.id
    )
    .fetch_one(&mut *tx)
    .await?;
    if exists {
        return Ok(false);
    }
    sqlx::query!(
        "INSERT INTO roots (id, display_name) VALUES ($1, $2)
         ON CONFLICT (id) DO UPDATE SET display_name = excluded.display_name, status = 'unknown'",
        root.id,
        root.display_name
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "INSERT INTO root_settings (root_id, kind, settings, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5)",
        root.id,
        root.kind,
        root.settings,
        now,
        now
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(true)
}

/// Replaces a stored root's display name and settings. With
/// `location_changed`, advances the source version and invalidates derived
/// media and listing digests even when the new source has matching file
/// metadata. Returns false when there is no such stored root.
///
/// # Errors
/// Any database error.
pub async fn update_stored(
    pool: &crate::Pool,
    root: &StoredRoot,
    location_changed: bool,
    now: i64,
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    let done = sqlx::query!(
        "UPDATE root_settings SET kind = $1, settings = $2, updated_at = $3 WHERE root_id = $4",
        root.kind,
        root.settings,
        now,
        root.id
    )
    .execute(&mut *tx)
    .await?;
    if done.rows_affected() == 0 {
        return Ok(false);
    }
    sqlx::query!(
        "UPDATE roots SET display_name = $1 WHERE id = $2",
        root.display_name,
        root.id
    )
    .execute(&mut *tx)
    .await?;
    if location_changed {
        sqlx::query!(
            "UPDATE roots SET source_version = source_version + 1 WHERE id = $1",
            root.id
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "UPDATE tracks SET probe = NULL,
                 duration_ms = CASE WHEN duration_src = 'imported' THEN duration_ms END,
                 duration_src = CASE WHEN duration_src = 'imported' THEN duration_src END
             WHERE location_id IN (SELECT id FROM work_locations WHERE root_id = $1)",
            root.id
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "DELETE FROM track_analysis WHERE track_id IN
                 (SELECT t.id FROM tracks t JOIN work_locations l ON l.id = t.location_id
                  WHERE l.root_id = $1)",
            root.id
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "UPDATE work_locations SET listing_hash = NULL WHERE root_id = $1",
            root.id
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(true)
}

/// Removes a stored root's settings and marks its locations and tracks
/// missing, and its works too when no other location of theirs is present.
/// Nothing is deleted: favorites and progress stay, and the missing-items
/// cleanup removes the rows when an administrator asks for it. Returns the
/// works that had a location in the root, whose duplicates the caller
/// settles in the same transaction, or `None` when there is no such stored
/// root.
///
/// # Errors
/// Any database error.
pub async fn remove_stored(
    tx: &mut crate::Tx<'_>,
    id: &str,
    now: i64,
) -> sqlx::Result<Option<Vec<i64>>> {
    let done = sqlx::query!("DELETE FROM root_settings WHERE root_id = $1", id)
        .execute(&mut **tx)
        .await?;
    if done.rows_affected() == 0 {
        return Ok(None);
    }
    let works = sqlx::query_scalar!(
        r#"SELECT DISTINCT work_id AS "work_id!: i64" FROM work_locations WHERE root_id = $1"#,
        id
    )
    .fetch_all(&mut **tx)
    .await?;
    sqlx::query!(
        "UPDATE tracks SET missing_at = $1
         WHERE missing_at IS NULL
           AND location_id IN (SELECT id FROM work_locations WHERE root_id = $2)",
        now,
        id
    )
    .execute(&mut **tx)
    .await?;
    // The digest goes too, so a root re-added later is walked in full and
    // its tracks restored.
    sqlx::query!(
        "UPDATE work_locations SET missing_at = $1, listing_hash = NULL
         WHERE root_id = $2 AND missing_at IS NULL",
        now,
        id
    )
    .execute(&mut **tx)
    .await?;
    sqlx::query!(
        "UPDATE works SET missing_at = $1
         WHERE missing_at IS NULL
           AND id IN (SELECT work_id FROM work_locations WHERE root_id = $2)
           AND NOT EXISTS (SELECT 1 FROM work_locations l
                           WHERE l.work_id = works.id AND l.missing_at IS NULL)",
        now,
        id
    )
    .execute(&mut **tx)
    .await?;
    sqlx::query!("UPDATE roots SET status = 'unknown' WHERE id = $1", id)
        .execute(&mut **tx)
        .await?;
    Ok(Some(works))
}

/// When the last scan of a root ended, whatever its outcome, in Unix
/// milliseconds. Falls back to the last successful scan for roots scanned
/// before this was recorded.
///
/// # Errors
/// Any database error.
pub async fn last_scan_end_at(pool: &crate::Pool, id: &str) -> sqlx::Result<Option<i64>> {
    let row = sqlx::query!(
        r#"SELECT last_scan_end_at AS "end: i64", last_scan_at AS "ok: i64"
           FROM roots WHERE id = $1"#,
        id
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|r| r.end.max(r.ok)))
}

/// Records that a scan of a root ended.
///
/// # Errors
/// Any database error.
pub async fn record_scan_end(pool: &crate::Pool, id: &str, now: i64) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE roots SET last_scan_end_at = $1 WHERE id = $2",
        now,
        id
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// # Errors
/// Any database error.
pub async fn status(pool: &crate::Pool, id: &str) -> sqlx::Result<Option<RootRow>> {
    sqlx::query_as!(
        RootRow,
        r#"SELECT r.display_name, r.status, r.last_scan_at,
                  (SELECT COUNT(DISTINCT l.work_id) FROM work_locations l
                   WHERE l.root_id = r.id AND l.missing_at IS NULL) AS "work_count!: i64"
           FROM roots r WHERE r.id = $1"#,
        id
    )
    .fetch_optional(pool)
    .await
}
