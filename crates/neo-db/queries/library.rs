//! Favorites and the circle, voice actor and tag lists of M3. The work
//! lists themselves are in `works`.

use neo_core::db::library::*;

use neo_core::dto::{CircleCount, Id, SafeInteger, TagCount, VaCount};

fn invalid(id: impl ToString) -> impl Fn(&'static str) -> LibraryError {
    let id = id.to_string();
    move |reason| LibraryError::InvalidRow {
        id: id.clone(),
        reason,
    }
}

/// Returns false when the work does not exist.
pub async fn set_favorite(
    pool: &crate::Pool,
    user_id: i64,
    work_id: i64,
    favorite: bool,
    now: i64,
) -> sqlx::Result<bool> {
    let mut tx = pool.begin().await?;
    let exists = sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM works WHERE id = $1) AS "e!: bool""#,
        work_id
    )
    .fetch_one(&mut *tx)
    .await?;
    if !exists {
        return Ok(false);
    }
    if favorite {
        // A repeated PUT keeps the original time, so the favorites order
        // does not jump.
        sqlx::query!(
            "INSERT INTO favorites (user_id, work_id, created_at) VALUES ($1, $2, $3)
             ON CONFLICT (user_id, work_id) DO NOTHING",
            user_id,
            work_id,
            now
        )
        .execute(&mut *tx)
        .await?;
    } else {
        sqlx::query!(
            "DELETE FROM favorites WHERE user_id = $1 AND work_id = $2",
            user_id,
            work_id
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(true)
}

pub async fn circles(pool: &crate::Pool) -> Result<Vec<CircleCount>, LibraryError> {
    sqlx::query!(
        r#"SELECT c.id AS "id!", c.name, COUNT(*) AS "work_count!: i64"
           FROM circles c JOIN works w ON w.circle_id = c.id AND w.missing_at IS NULL
           GROUP BY c.id ORDER BY c.name, c.id"#
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| {
        Ok(CircleCount {
            id: Id::try_from(r.id).map_err(invalid(r.id))?,
            name: r.name,
            work_count: SafeInteger::try_from(r.work_count).map_err(invalid(r.id))?,
        })
    })
    .collect()
}

pub async fn vas(pool: &crate::Pool) -> Result<Vec<VaCount>, LibraryError> {
    sqlx::query!(
        r#"SELECT v.id AS "id!", v.name, COUNT(*) AS "work_count!: i64"
           FROM vas v
           JOIN work_vas wv ON wv.va_id = v.id
           JOIN works w ON w.id = wv.work_id AND w.missing_at IS NULL
           GROUP BY v.id ORDER BY v.name, v.id"#
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| {
        Ok(VaCount {
            work_count: SafeInteger::try_from(r.work_count).map_err(invalid(&r.id))?,
            id: r.id,
            name: r.name,
        })
    })
    .collect()
}

pub async fn tags(pool: &crate::Pool) -> Result<Vec<TagCount>, LibraryError> {
    sqlx::query!(
        r#"SELECT t.id AS "id!", t.name, COUNT(*) AS "work_count!: i64"
           FROM tags t
           JOIN work_tags wt ON wt.tag_id = t.id
           JOIN works w ON w.id = wt.work_id AND w.missing_at IS NULL
           GROUP BY t.id ORDER BY t.name, t.id"#
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| {
        Ok(TagCount {
            id: Id::try_from(r.id).map_err(invalid(r.id))?,
            name: r.name,
            work_count: SafeInteger::try_from(r.work_count).map_err(invalid(r.id))?,
        })
    })
    .collect()
}
