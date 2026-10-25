//! Writes to `play_progress`, following "收听状态怎么算" and "什么算播完"
//! in book/src/data-model.md.

use neo_core::db::progress::*;

/// # Errors
/// Any database error.
pub async fn track_place(
    read: &crate::Pool,
    track_id: i64,
) -> Result<Option<TrackPlace>, sqlx::Error> {
    sqlx::query_as!(
        TrackPlace,
        r#"SELECT work_id AS "work_id!: i64", display_path, kind FROM tracks WHERE id = $1"#,
        track_id
    )
    .fetch_optional(read)
    .await
}

/// Whether `track_id` is the last playable track directly inside
/// `group_path` of a work, in the order the player queues them: natural
/// order by `sort_key`, skipping missing files and duplicates of another
/// location, which the player never sees. `group_path` is `""` for the
/// work's top directory. A hidden duplicate counts as the copy it
/// duplicates, by the rule `report` follows. One statement resolves the
/// copy and finds the last track, so a scan cannot change the link between.
///
/// # Errors
/// Any database error.
pub async fn ends_group(
    read: &crate::Pool,
    work_id: i64,
    group_path: &str,
    track_id: i64,
) -> Result<bool, sqlx::Error> {
    let prefix = if group_path.is_empty() {
        String::new()
    } else {
        format!("{group_path}/")
    };
    // substr keeps `%` and `_` in directory names from acting as LIKE
    // wildcards, and the NOT LIKE drops files in subdirectories. The length
    // comes from length() rather than a bound i64, because PostgreSQL's
    // substr only takes a 32-bit length.
    sqlx::query_scalar!(
        r#"SELECT COALESCE(
             (SELECT id FROM tracks
              WHERE work_id = $1
                AND kind IN ('audio', 'video')
                AND missing_at IS NULL
                AND duplicate_of IS NULL
                AND substr(display_path, 1, length($2)) = $2
                AND substr(display_path, length($2) + 1) NOT LIKE '%/%'
              ORDER BY sort_key DESC, id DESC
              LIMIT 1)
             = COALESCE((SELECT k.id FROM tracks d JOIN tracks k ON k.id = d.duplicate_of
                         WHERE d.id = $3 AND k.rel_path = d.rel_path
                           AND k.size = d.size AND k.missing_at IS NULL), $3),
             FALSE) AS "last!: bool""#,
        work_id,
        prefix,
        track_id
    )
    .fetch_one(read)
    .await
}

/// Records a position. Playing a work that was already finished marks it as
/// being replayed. `finished_at` is kept.
///
/// A player whose queue predates a scan may still report a copy the scan
/// has since hidden as a duplicate. The position is stored on the copy the
/// scan kept, in the same statement, as the scan itself would move it. A
/// scan in progress may have changed the kept file's size or marked it
/// missing before it works out the duplicates again, so the link counts
/// only while the two are still the same present file, the rule of
/// `dedupe_work` in neo-scanner. `ends_group` and `finish` use the same
/// rule.
///
/// # Errors
/// Any database error.
pub async fn report(
    write: &crate::Pool,
    user_id: i64,
    work_id: i64,
    input: &ReportInput<'_>,
    now: i64,
) -> Result<(), sqlx::Error> {
    let ReportInput {
        track_id,
        position_ms,
        group_path,
        ..
    } = *input;
    neo_core::db::retry_busy(|| async {
        let mut tx = write.begin().await?;
        if let Some(order) = input.report_order {
            let accepted = sqlx::query_scalar!(
                r#"INSERT INTO progress_report_cursors (user_id, work_id, session_id, last_sequence)
                   VALUES ($1, $2, $3, $4)
                   ON CONFLICT (user_id, work_id, session_id) DO UPDATE
                   SET last_sequence = excluded.last_sequence
                   WHERE excluded.last_sequence > progress_report_cursors.last_sequence
                   RETURNING last_sequence AS "last_sequence!: i64""#,
                user_id,
                work_id,
                order.session_id,
                order.sequence
            )
            .fetch_optional(&mut *tx)
            .await?;
            if accepted.is_none() {
                return tx.commit().await;
            }
        }
        // The watermark must roll back with the progress, including when
        // retry_busy starts a fresh transaction after a write conflict.
        sqlx::query!(
            "INSERT INTO play_progress (user_id, work_id, track_id, position_ms, group_path, updated_at)
             VALUES ($1, $2, COALESCE((SELECT k.id FROM tracks d JOIN tracks k ON k.id = d.duplicate_of
                                       WHERE d.id = $3 AND k.rel_path = d.rel_path
                                         AND k.size = d.size AND k.missing_at IS NULL), $3), $4, $5, $6)
             ON CONFLICT (user_id, work_id) DO UPDATE SET
               track_id = excluded.track_id,
               position_ms = excluded.position_ms,
               group_path = excluded.group_path,
               updated_at = excluded.updated_at,
               replaying = play_progress.finished_at IS NOT NULL",
            user_id,
            work_id,
            track_id,
            position_ms,
            group_path,
            now
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await
    })
    .await?;
    Ok(())
}

/// Marks the work as finished on `track_id` and ends any replay. A position
/// already recorded is kept, so the work can still be resumed. A hidden
/// duplicate is stored as the copy it duplicates, as in [`report`].
///
/// # Errors
/// Any database error.
pub async fn finish(
    write: &crate::Pool,
    user_id: i64,
    work_id: i64,
    track_id: i64,
    group_path: &str,
    now: i64,
) -> Result<(), sqlx::Error> {
    neo_core::db::retry_busy(|| async {
        sqlx::query!(
            "INSERT INTO play_progress
               (user_id, work_id, track_id, position_ms, group_path, finished_at, replaying, updated_at)
             VALUES ($1, $2, COALESCE((SELECT k.id FROM tracks d JOIN tracks k ON k.id = d.duplicate_of
                                       WHERE d.id = $3 AND k.rel_path = d.rel_path
                                         AND k.size = d.size AND k.missing_at IS NULL), $3),
                     0, $4, $5, FALSE, $6)
             ON CONFLICT (user_id, work_id) DO UPDATE SET
               position_ms = CASE WHEN play_progress.track_id IS NOT DISTINCT FROM excluded.track_id
                                  THEN play_progress.position_ms ELSE 0 END,
               track_id = excluded.track_id,
               group_path = excluded.group_path,
               finished_at = excluded.finished_at,
               replaying = FALSE,
               updated_at = excluded.updated_at",
            user_id,
            work_id,
            track_id,
            group_path,
            now,
            now
        )
        .execute(write)
        .await
    })
    .await?;
    Ok(())
}

/// Deletes the user's progress for the work. Deleting nothing is fine.
///
/// # Errors
/// Any database error.
pub async fn clear(write: &crate::Pool, user_id: i64, work_id: i64) -> Result<(), sqlx::Error> {
    neo_core::db::retry_busy(|| async {
        sqlx::query!(
            "DELETE FROM play_progress WHERE user_id = $1 AND work_id = $2",
            user_id,
            work_id
        )
        .execute(write)
        .await
    })
    .await?;
    Ok(())
}
