//! The user's own ratings and short reviews (`reviews`, book/src/data-model.md).

use neo_core::db::reviews::*;

use neo_core::{
    dto::{Review, SafeInteger},
    review::Valid,
};

fn to_dto(
    work_id: i64,
    rating: Option<i64>,
    text: Option<String>,
    updated_at: Option<i64>,
) -> Result<Review, ReviewError> {
    let invalid = |reason| ReviewError::InvalidRow { work_id, reason };
    Ok(Review {
        rating: rating
            .map(u8::try_from)
            .transpose()
            .map_err(|_| invalid("rating is out of range"))?,
        text,
        updated_at: updated_at
            .map(SafeInteger::try_from)
            .transpose()
            .map_err(invalid)?,
    })
}

/// rating, text, updated_at
type Row = (Option<i64>, Option<String>, Option<i64>);

async fn row(
    read: impl sqlx::Executor<'_, Database = crate::Database>,
    user_id: i64,
    work_id: i64,
) -> sqlx::Result<Option<Row>> {
    let row = sqlx::query!(
        "SELECT rating, text, updated_at FROM reviews WHERE user_id = $1 AND work_id = $2",
        user_id,
        work_id
    )
    .fetch_optional(read)
    .await?;
    Ok(row.map(|r| (r.rating, r.text, r.updated_at)))
}

/// # Errors
/// Any database error, or a stored value the DTO cannot carry.
pub async fn get(
    read: impl sqlx::Executor<'_, Database = crate::Database>,
    user_id: i64,
    work_id: i64,
) -> Result<Option<Review>, ReviewError> {
    row(read, user_id, work_id)
        .await?
        .map(|(rating, text, updated_at)| to_dto(work_id, rating, text, updated_at))
        .transpose()
}

async fn work_exists(tx: &mut crate::Tx<'_>, work_id: i64) -> sqlx::Result<bool> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM works WHERE id = $1) AS "e!: bool""#,
        work_id
    )
    .fetch_one(&mut **tx)
    .await
}

/// Replaces the user's review of a work. Returns `None` when the work does
/// not exist. The transaction reads before it writes, so a `user` or
/// `migrate` command writing meanwhile makes it busy, and it runs again.
///
/// # Errors
/// Any database error.
pub async fn put(
    write: &crate::Pool,
    user_id: i64,
    work_id: i64,
    review: &Valid,
    now: i64,
) -> Result<Option<Review>, ReviewError> {
    let stored = neo_core::db::retry_busy(|| async {
        let mut tx = write.begin().await?;
        if !work_exists(&mut tx, work_id).await? {
            return Ok(None);
        }
        // PostgreSQL has no unsigned integers.
        let rating = review.rating.map(i64::from);
        sqlx::query!(
            "INSERT INTO reviews (user_id, work_id, rating, text, updated_at) VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (user_id, work_id) DO UPDATE SET
             rating = excluded.rating, text = excluded.text, updated_at = excluded.updated_at",
            user_id,
            work_id,
            rating,
            review.text,
            now
        )
        .execute(&mut *tx)
        .await?;
        let stored = row(&mut *tx, user_id, work_id).await?;
        tx.commit().await?;
        Ok(stored)
    })
    .await?;
    stored
        .map(|(rating, text, updated_at)| to_dto(work_id, rating, text, updated_at))
        .transpose()
}

/// Clears the user's review of a work. Returns false when the work does not
/// exist. Busy runs it again, as for [`put`].
///
/// # Errors
/// Any database error.
pub async fn delete(write: &crate::Pool, user_id: i64, work_id: i64) -> sqlx::Result<bool> {
    neo_core::db::retry_busy(|| async {
        let mut tx = write.begin().await?;
        let exists = work_exists(&mut tx, work_id).await?;
        if exists {
            sqlx::query!(
                "DELETE FROM reviews WHERE user_id = $1 AND work_id = $2",
                user_id,
                work_id
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(exists)
    })
    .await
}

/// Adds a review carried over from an old library, keeping one the user
/// already has. Returns how many rows were written (0 or 1).
///
/// # Errors
/// Any database error.
pub async fn insert_legacy(
    tx: &mut crate::Tx<'_>,
    user_id: i64,
    work_id: i64,
    review: &Valid,
    updated_at: Option<i64>,
) -> sqlx::Result<u64> {
    let rating = review.rating.map(i64::from);
    let done = sqlx::query!(
        "INSERT INTO reviews (user_id, work_id, rating, text, updated_at) VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (user_id, work_id) DO NOTHING",
        user_id,
        work_id,
        rating,
        review.text,
        updated_at
    )
    .execute(&mut **tx)
    .await?;
    Ok(done.rows_affected())
}
