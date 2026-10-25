//! The statements `import-legacy` runs: a fresh import fills an empty
//! library, a merge adds old user data to one that has works.

use neo_core::db::import::*;

/// # Errors
/// Any database error.
pub async fn insert_root(tx: &mut crate::Tx<'_>, id: &str, display_name: &str) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO roots (id, display_name) VALUES ($1, $2)",
        id,
        display_name
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// # Errors
/// Any database error.
pub async fn insert_circle(tx: &mut crate::Tx<'_>, id: i64, name: &str) -> sqlx::Result<()> {
    sqlx::query!("INSERT INTO circles (id, name) VALUES ($1, $2)", id, name)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// # Errors
/// Any database error.
pub async fn insert_va(tx: &mut crate::Tx<'_>, id: &str, name: &str) -> sqlx::Result<()> {
    sqlx::query!("INSERT INTO vas (id, name) VALUES ($1, $2)", id, name)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// The tag and its name as an alias, so the dictionary sync finds it.
///
/// # Errors
/// Any database error.
pub async fn insert_tag(tx: &mut crate::Tx<'_>, id: i64, name: &str) -> sqlx::Result<()> {
    sqlx::query!("INSERT INTO tags (id, name) VALUES ($1, $2)", id, name)
        .execute(&mut **tx)
        .await?;
    sqlx::query!(
        "INSERT INTO tag_aliases (alias, tag_id) VALUES ($1, $2) ON CONFLICT (alias) DO NOTHING",
        name,
        id
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// # Errors
/// Any database error.
pub async fn insert_work(tx: &mut crate::Tx<'_>, work: ImportedWork<'_>) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"INSERT INTO works (code, title, circle_id, release_date, nsfw, dl_count, price,
                              rate_average, rate_count, metadata_src, cover_src, extra,
                              created_at, updated_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
           RETURNING id AS "id!""#,
        work.code,
        work.title,
        work.circle_id,
        work.release_date,
        work.nsfw,
        work.dl_count,
        work.price,
        work.rate_average,
        work.rate_count,
        work.metadata_src,
        work.cover_src,
        work.extra,
        work.created_at,
        work.updated_at,
    )
    .fetch_one(&mut **tx)
    .await
}

/// # Errors
/// Any database error.
pub async fn insert_work_tag(
    tx: &mut crate::Tx<'_>,
    work_id: i64,
    tag_id: i64,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO work_tags (work_id, tag_id) VALUES ($1, $2)",
        work_id,
        tag_id
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// # Errors
/// Any database error.
pub async fn insert_work_va(tx: &mut crate::Tx<'_>, work_id: i64, va_id: &str) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO work_vas (work_id, va_id) VALUES ($1, $2)",
        work_id,
        va_id
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// # Errors
/// Any database error.
pub async fn user_id(tx: &mut crate::Tx<'_>, name: &str) -> sqlx::Result<Option<i64>> {
    sqlx::query_scalar!("SELECT id FROM users WHERE name = $1", name)
        .fetch_optional(&mut **tx)
        .await
}

/// An account without a password. Old password hashes are not kept
/// (book/src/auth.md), so it cannot log in before `user set-password`.
///
/// # Errors
/// Any database error, such as a name that is taken.
pub async fn insert_user(
    tx: &mut crate::Tx<'_>,
    name: &str,
    is_admin: bool,
    token_version: i64,
    now: i64,
) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        r#"INSERT INTO users (name, password_hash, is_admin, token_version, created_at)
           VALUES ($1, NULL, $2, $3, $4)
           RETURNING id AS "id!""#,
        name,
        is_admin,
        token_version,
        now
    )
    .fetch_one(&mut **tx)
    .await
}

/// # Errors
/// Any database error.
pub async fn insert_favorite(
    tx: &mut crate::Tx<'_>,
    user_id: i64,
    work_id: i64,
    created_at: i64,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO favorites (user_id, work_id, created_at) VALUES ($1, $2, $3)",
        user_id,
        work_id,
        created_at
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// # Errors
/// Any database error.
pub async fn insert_progress(
    tx: &mut crate::Tx<'_>,
    user_id: i64,
    work_id: i64,
    position_ms: i64,
    finished_at: Option<i64>,
    updated_at: i64,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO play_progress (user_id, work_id, position_ms, finished_at, updated_at)
         VALUES ($1, $2, $3, $4, $5)",
        user_id,
        work_id,
        position_ms,
        finished_at,
        updated_at
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// # Errors
/// Any database error.
pub async fn insert_legacy_review(
    tx: &mut crate::Tx<'_>,
    user_id: i64,
    work_id: i64,
    rating: Option<i64>,
    review_text: Option<&str>,
    progress: Option<&str>,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO legacy_reviews (user_id, work_id, rating, review_text, progress)
         VALUES ($1, $2, $3, $4, $5)",
        user_id,
        work_id,
        rating,
        review_text,
        progress
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// # Errors
/// Any database error.
pub async fn insert_pending(
    tx: &mut crate::Tx<'_>,
    user_id: i64,
    work_id: i64,
    source: &str,
    durations: Option<&str>,
    state: &str,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO import_pending (user_id, work_id, source, durations, state)
         VALUES ($1, $2, $3, $4, $5)",
        user_id,
        work_id,
        source,
        durations,
        state
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// The work codes merged user data still waits for.
///
/// # Errors
/// Any database error.
pub async fn waiting_codes(tx: &mut crate::Tx<'_>) -> sqlx::Result<Vec<String>> {
    sqlx::query_scalar!("SELECT DISTINCT code FROM legacy_user_state")
        .fetch_all(&mut **tx)
        .await
}

/// # Errors
/// Any database error.
pub async fn waiting_count(tx: &mut crate::Tx<'_>) -> sqlx::Result<i64> {
    sqlx::query_scalar!(r#"SELECT COUNT(DISTINCT code) AS "n!: i64" FROM legacy_user_state"#)
        .fetch_one(&mut **tx)
        .await
}

/// # Errors
/// Any database error.
pub async fn imported_covers(read: &crate::Pool) -> sqlx::Result<i64> {
    sqlx::query_scalar!(r#"SELECT COUNT(*) AS "n!: i64" FROM works WHERE cover_src = 'imported'"#)
        .fetch_one(read)
        .await
}
