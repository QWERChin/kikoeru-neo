//! Settings for the whole server (`server_settings`, book/src/data-model.md).

/// The stored metadata proxy, `None` when none is set.
///
/// # Errors
/// Any database error.
pub async fn metadata_proxy(read: &crate::Pool) -> Result<Option<String>, sqlx::Error> {
    let row = sqlx::query_scalar!("SELECT metadata_proxy FROM server_settings WHERE id = 1")
        .fetch_optional(read)
        .await?;
    Ok(row.flatten())
}

/// Replaces the metadata proxy. The caller normalises it.
///
/// # Errors
/// Any database error.
pub async fn set_metadata_proxy(
    write: &crate::Pool,
    proxy: Option<&str>,
    now: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO server_settings (id, metadata_proxy, updated_at) VALUES (1, $1, $2)
         ON CONFLICT (id) DO UPDATE SET
             metadata_proxy = excluded.metadata_proxy,
             updated_at = excluded.updated_at",
        proxy,
        now
    )
    .execute(write)
    .await?;
    Ok(())
}
