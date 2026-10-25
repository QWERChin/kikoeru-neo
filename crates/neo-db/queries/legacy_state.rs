//! `legacy_user_state`: old favorites, reviews and play positions that wait
//! for their work to be scanned. "合并到已有的库" in book/src/migration.md
//! has the rules.

use neo_core::db::legacy_state::*;

use super::reviews;

use neo_core::review;

/// Stores `state`, replacing an earlier merge of the same user and code.
///
/// # Errors
/// Any database error.
pub async fn put(tx: &mut crate::Tx<'_>, state: &State) -> Result<(), sqlx::Error> {
    let review = state.review.as_ref();
    let progress = state.progress.as_ref();
    let pending = progress.and_then(|p| p.pending.as_ref());
    let has_review = review.is_some();
    let rating = review.and_then(|r| r.rating);
    let review_text = review.and_then(|r| r.review_text.as_deref());
    let review_progress = review.and_then(|r| r.progress.as_deref());
    let review_at = review.and_then(|r| r.updated_at);
    let progress_at = progress.map(|p| p.updated_at);
    let position_ms = progress.map(|p| p.position_ms);
    let finished_at = progress.and_then(|p| p.finished_at);
    let pending_source = pending.map(|p| p.0.as_str());
    let pending_durations = pending.and_then(|p| p.1.as_deref());
    let pending_state = pending.map(|p| p.2.as_str());
    sqlx::query!(
        "INSERT INTO legacy_user_state
           (user_id, code, favorite_at, has_review, rating, review_text, review_progress,
            review_at, progress_at, position_ms, finished_at, pending_source,
            pending_durations, pending_state)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
         ON CONFLICT (user_id, code) DO UPDATE SET
           favorite_at = excluded.favorite_at, has_review = excluded.has_review,
           rating = excluded.rating, review_text = excluded.review_text,
           review_progress = excluded.review_progress, review_at = excluded.review_at,
           progress_at = excluded.progress_at, position_ms = excluded.position_ms,
           finished_at = excluded.finished_at, pending_source = excluded.pending_source,
           pending_durations = excluded.pending_durations,
           pending_state = excluded.pending_state",
        state.user_id,
        state.code,
        state.favorite_at,
        has_review,
        rating,
        review_text,
        review_progress,
        review_at,
        progress_at,
        position_ms,
        finished_at,
        pending_source,
        pending_durations,
        pending_state
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Moves the waiting rows for `code` onto the work `work_id`, then deletes
/// them. Favorites and reviews the user already has stay as they are. An
/// old position replaces the library's only when it is newer, because the
/// newer one is where the user really is.
///
/// # Errors
/// Any database error.
pub async fn claim(
    tx: &mut crate::Tx<'_>,
    work_id: i64,
    code: &str,
) -> Result<Claimed, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT user_id, favorite_at, has_review AS "has_review: bool", rating, review_text,
                  review_progress, review_at, progress_at, position_ms, finished_at, pending_source,
                  pending_durations, pending_state
           FROM legacy_user_state WHERE code = $1"#,
        code
    )
    .fetch_all(&mut **tx)
    .await?;
    let mut claimed = Claimed::default();
    for row in rows {
        let user_id = row.user_id;
        if let Some(at) = row.favorite_at {
            let done = sqlx::query!(
                "INSERT INTO favorites (user_id, work_id, created_at) VALUES ($1, $2, $3)
                 ON CONFLICT DO NOTHING",
                user_id,
                work_id,
                at
            )
            .execute(&mut **tx)
            .await?;
            claimed.favorites += done.rows_affected();
        }
        if row.has_review {
            let done = sqlx::query!(
                "INSERT INTO legacy_reviews (user_id, work_id, rating, review_text, progress)
                 VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING",
                user_id,
                work_id,
                row.rating,
                row.review_text,
                row.review_progress
            )
            .execute(&mut **tx)
            .await?;
            claimed.reviews += done.rows_affected();
            if let Some(valid) = review::from_legacy(row.rating, row.review_text.as_deref()).review
            {
                reviews::insert_legacy(tx, user_id, work_id, &valid, row.review_at).await?;
            }
        }
        if let (Some(at), Some(position)) = (row.progress_at, row.position_ms) {
            let current = sqlx::query_scalar!(
                "SELECT updated_at FROM play_progress WHERE user_id = $1 AND work_id = $2",
                user_id,
                work_id
            )
            .fetch_optional(&mut **tx)
            .await?;
            if current.is_some_and(|c| c >= at) {
                claimed.progress_kept += 1;
            } else {
                sqlx::query!(
                    "INSERT INTO play_progress
                       (user_id, work_id, track_id, position_ms, group_path, finished_at,
                        replaying, updated_at)
                     VALUES ($1, $2, NULL, $3, NULL, $4, FALSE, $5)
                     ON CONFLICT (user_id, work_id) DO UPDATE SET
                       track_id = excluded.track_id, position_ms = excluded.position_ms,
                       group_path = excluded.group_path, finished_at = excluded.finished_at,
                       replaying = excluded.replaying, updated_at = excluded.updated_at",
                    user_id,
                    work_id,
                    position,
                    row.finished_at,
                    at
                )
                .execute(&mut **tx)
                .await?;
                if let (Some(source), Some(state)) = (&row.pending_source, &row.pending_state) {
                    sqlx::query!(
                        "INSERT INTO import_pending
                           (user_id, work_id, source, durations, state)
                         VALUES ($1, $2, $3, $4, $5)
                         ON CONFLICT (user_id, work_id) DO UPDATE SET
                           source = excluded.source, durations = excluded.durations,
                           state = excluded.state",
                        user_id,
                        work_id,
                        source,
                        row.pending_durations,
                        state
                    )
                    .execute(&mut **tx)
                    .await?;
                } else {
                    // Material from an earlier import belongs to the
                    // position just replaced, and matching it would put
                    // that old track on this one.
                    sqlx::query!(
                        "DELETE FROM import_pending WHERE user_id = $1 AND work_id = $2",
                        user_id,
                        work_id
                    )
                    .execute(&mut **tx)
                    .await?;
                }
                claimed.progress += 1;
            }
        }
    }
    sqlx::query!("DELETE FROM legacy_user_state WHERE code = $1", code)
        .execute(&mut **tx)
        .await?;
    Ok(claimed)
}
