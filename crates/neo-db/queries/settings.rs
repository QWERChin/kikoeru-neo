//! Per-user player settings (`user_settings`, book/src/data-model.md).

use neo_core::db::settings::*;

use neo_core::dto::{Accent, UserSettings};

/// # Errors
/// Any database error.
pub async fn get(read: &crate::Pool, user_id: i64) -> Result<Settings, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT normalize_loudness AS "normalize_loudness: bool", target_lufs, accent,
                  seek_back_seconds, seek_forward_seconds
           FROM user_settings WHERE user_id = $1"#,
        user_id
    )
    .fetch_optional(read)
    .await?;
    // The columns' CHECKs only allow the DTO's names and 1 to 99 seconds,
    // so parsing never drops a stored value.
    Ok(row.map_or_else(Settings::default, |r| Settings {
        normalize_loudness: r.normalize_loudness,
        target_lufs: r.target_lufs,
        accent: r.accent.as_deref().and_then(Accent::parse),
        seek_back_seconds: u32::try_from(r.seek_back_seconds)
            .unwrap_or(UserSettings::DEFAULT_SEEK_BACK),
        seek_forward_seconds: u32::try_from(r.seek_forward_seconds)
            .unwrap_or(UserSettings::DEFAULT_SEEK_FORWARD),
    }))
}

/// Replaces the user's settings. The caller checks the ranges.
///
/// # Errors
/// Any database error.
pub async fn put(
    write: &crate::Pool,
    user_id: i64,
    settings: Settings,
    now: i64,
) -> Result<(), sqlx::Error> {
    let accent = settings.accent.map(Accent::as_str);
    // PostgreSQL has no unsigned integers.
    let (back, forward) = (
        i64::from(settings.seek_back_seconds),
        i64::from(settings.seek_forward_seconds),
    );
    sqlx::query!(
        "INSERT INTO user_settings (user_id, normalize_loudness, target_lufs, accent,
                                    seek_back_seconds, seek_forward_seconds, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (user_id) DO UPDATE SET
             normalize_loudness = excluded.normalize_loudness,
             target_lufs = excluded.target_lufs,
             accent = excluded.accent,
             seek_back_seconds = excluded.seek_back_seconds,
             seek_forward_seconds = excluded.seek_forward_seconds,
             updated_at = excluded.updated_at",
        user_id,
        settings.normalize_loudness,
        settings.target_lufs,
        accent,
        back,
        forward,
        now
    )
    .execute(write)
    .await?;
    Ok(())
}
