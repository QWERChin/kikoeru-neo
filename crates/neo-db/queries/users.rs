use neo_core::db::users::*;

pub async fn find_by_name(pool: &crate::Pool, name: &str) -> sqlx::Result<Option<UserRecord>> {
    sqlx::query_as!(
        UserRecord,
        r#"SELECT id AS "id!", name, password_hash, is_admin AS "is_admin: bool", token_version
           FROM users WHERE name = $1"#,
        name
    )
    .fetch_optional(pool)
    .await
}

pub async fn find_by_id(pool: &crate::Pool, id: i64) -> sqlx::Result<Option<UserRecord>> {
    sqlx::query_as!(
        UserRecord,
        r#"SELECT id AS "id!", name, password_hash, is_admin AS "is_admin: bool", token_version
           FROM users WHERE id = $1"#,
        id
    )
    .fetch_optional(pool)
    .await
}

pub async fn admin_exists(pool: &crate::Pool) -> sqlx::Result<bool> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM users WHERE is_admin = TRUE) AS "e!: bool""#
    )
    .fetch_one(pool)
    .await
}

/// Creates a user. `token_version` should be random: tokens carry it, so a
/// token issued to a deleted account cannot match a later account even if
/// the ID were ever reused (AUTOINCREMENT already prevents that).
pub async fn create(
    pool: &crate::Pool,
    name: &str,
    password_hash: &str,
    is_admin: bool,
    token_version: i64,
    now_ms: i64,
) -> Result<i64, CreateError> {
    let result = sqlx::query_scalar!(
        r#"INSERT INTO users (name, password_hash, is_admin, token_version, created_at)
           VALUES ($1, $2, $3, $4, $5) RETURNING id AS "id!""#,
        name,
        password_hash,
        is_admin,
        token_version,
        now_ms
    )
    .fetch_one(pool)
    .await;
    match result {
        Ok(id) => Ok(id),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            Err(CreateError::NameTaken(name.to_owned()))
        }
        Err(e) => Err(e.into()),
    }
}

/// Replaces the password hash and bumps `token_version`, which revokes every
/// API and media token issued to this user. Returns false if no such user.
pub async fn set_password(pool: &crate::Pool, id: i64, password_hash: &str) -> sqlx::Result<bool> {
    // Another process may change the same row. On PostgreSQL that is a
    // serialization failure, run again here.
    let done = neo_core::db::retry_busy(|| {
        sqlx::query!(
            "UPDATE users SET password_hash = $1, token_version = token_version + 1 WHERE id = $2",
            password_hash,
            id
        )
        .execute(pool)
    })
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Bumps `token_version`, which revokes every API and media token issued to
/// this user. Returns false if no such user.
pub async fn revoke_tokens(pool: &crate::Pool, id: i64) -> sqlx::Result<bool> {
    let done = neo_core::db::retry_busy(|| {
        sqlx::query!(
            "UPDATE users SET token_version = token_version + 1 WHERE id = $1",
            id
        )
        .execute(pool)
    })
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Like [`set_password`], but only if `token_version` still equals
/// `expected_version`. A self-service change must lose to a reset that
/// happened while it was in flight, otherwise a client holding the old
/// password could overwrite the reset. Returns false if the version moved.
pub async fn set_password_if_version(
    pool: &crate::Pool,
    id: i64,
    expected_version: i64,
    password_hash: &str,
) -> sqlx::Result<bool> {
    let done = neo_core::db::retry_busy(|| {
        sqlx::query!(
            "UPDATE users SET password_hash = $1, token_version = token_version + 1
             WHERE id = $2 AND token_version = $3",
            password_hash,
            id,
            expected_version
        )
        .execute(pool)
    })
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Deletes a user unless it is the only admin left. The check and the delete
/// are one statement, so another process cannot delete the other admin in
/// between.
pub async fn delete(pool: &crate::Pool, name: &str) -> sqlx::Result<DeleteOutcome> {
    // On PostgreSQL two processes deleting the last two admins conflict,
    // and the one that loses runs again and sees a single admin left.
    let done = neo_core::db::retry_busy(|| {
        sqlx::query!(
            "DELETE FROM users WHERE name = $1 AND NOT (
                 is_admin = TRUE AND (SELECT count(*) FROM users WHERE is_admin = TRUE) <= 1
             )",
            name
        )
        .execute(pool)
    })
    .await?;
    if done.rows_affected() == 1 {
        return Ok(DeleteOutcome::Deleted);
    }
    Ok(match find_by_name(pool, name).await? {
        Some(_) => DeleteOutcome::LastAdmin,
        None => DeleteOutcome::NotFound,
    })
}

/// Users without a password, administrators first, as `(name, is_admin)`.
///
/// # Errors
/// Any database error.
pub async fn without_password(pool: &crate::Pool) -> sqlx::Result<Vec<(String, bool)>> {
    Ok(sqlx::query!(
        r#"SELECT name, is_admin AS "is_admin: bool" FROM users WHERE password_hash IS NULL
           ORDER BY is_admin DESC, name"#
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| (r.name, r.is_admin))
    .collect())
}
