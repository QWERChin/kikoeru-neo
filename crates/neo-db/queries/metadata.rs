//! Queries for fetching and storing a work's metadata. neo-scanner decides
//! what may be written; these run the statements.

use neo_core::db::metadata::*;

use super::{edit::va_id, tags};

/// # Errors
/// Any database error.
pub async fn sources(read: &crate::Pool, work_id: i64) -> sqlx::Result<Option<Sources>> {
    sqlx::query_as!(
        Sources,
        "SELECT code, metadata_src, cover_src FROM works WHERE id = $1",
        work_id
    )
    .fetch_optional(read)
    .await
}

/// # Errors
/// Any database error.
pub async fn cover_version(read: &crate::Pool, work_id: i64) -> sqlx::Result<Option<i64>> {
    sqlx::query_scalar!("SELECT cover_version FROM works WHERE id = $1", work_id)
        .fetch_optional(read)
        .await
}

/// # Errors
/// Any database error.
pub async fn stored(tx: &mut crate::Tx<'_>, work_id: i64) -> sqlx::Result<Option<Stored>> {
    sqlx::query_as!(
        Stored,
        "SELECT metadata_src, cover_src, cover_version, extra FROM works WHERE id = $1",
        work_id
    )
    .fetch_optional(&mut **tx)
    .await
}

/// A new cover version gives the thumbnail and cover URLs a new cache key.
///
/// # Errors
/// Any database error.
pub async fn bump_cover(
    tx: &mut crate::Tx<'_>,
    work_id: i64,
    cover_src: &str,
    now: i64,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE works SET cover_src = $1, cover_version = cover_version + 1,
         updated_at = $2 WHERE id = $3",
        cover_src,
        now,
        work_id
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Adds the circle, or renames it unless a manually edited work other than
/// `work_id` links to it: renaming it would change that work's metadata.
///
/// # Errors
/// Any database error.
pub async fn upsert_circle(
    tx: &mut crate::Tx<'_>,
    circle_id: i64,
    name: &str,
    work_id: i64,
) -> sqlx::Result<()> {
    sqlx::query!(
        "INSERT INTO circles (id, name) VALUES ($1, $2)
         ON CONFLICT (id) DO UPDATE SET name = excluded.name
         WHERE excluded.name <> ''
           AND NOT EXISTS (SELECT 1 FROM works
                           WHERE circle_id = circles.id AND metadata_src = 'manual'
                             AND id <> $3)",
        circle_id,
        name,
        work_id
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// # Errors
/// Any database error.
pub async fn update_work(
    tx: &mut crate::Tx<'_>,
    work_id: i64,
    fields: Fields<'_>,
    now: i64,
) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE works SET title = $1, circle_id = $2, release_date = $3, nsfw = $4, dl_count = $5,
             price = $6, rate_average = $7, rate_count = $8, extra = $9,
             metadata_src = $10, updated_at = $11
         WHERE id = $12",
        fields.title,
        fields.circle_id,
        fields.release_date,
        fields.nsfw,
        fields.dl_count,
        fields.price,
        fields.rate_average,
        fields.rate_count,
        fields.extra,
        fields.source,
        now,
        work_id
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Replaces the work's voice actors and tags. Tags go through the
/// dictionary's aliases and merges (`tags::resolve`).
///
/// # Errors
/// Any database error.
pub async fn replace_credits(
    tx: &mut crate::Tx<'_>,
    work_id: i64,
    vas: &[String],
    tag_names: &[(String, Option<i64>)],
) -> sqlx::Result<()> {
    sqlx::query!("DELETE FROM work_vas WHERE work_id = $1", work_id)
        .execute(&mut **tx)
        .await?;
    for name in vas {
        let id = va_id(tx, name).await?;
        sqlx::query!(
            "INSERT INTO work_vas (work_id, va_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            work_id,
            id
        )
        .execute(&mut **tx)
        .await?;
    }
    sqlx::query!("DELETE FROM work_tags WHERE work_id = $1", work_id)
        .execute(&mut **tx)
        .await?;
    for (name, dlsite_id) in tag_names {
        let id = tags::resolve(tx, name, *dlsite_id).await?;
        sqlx::query!(
            "INSERT INTO work_tags (work_id, tag_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            work_id,
            id
        )
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// Present works of the root that have no metadata yet.
///
/// # Errors
/// Any database error.
pub async fn works_without_metadata(read: &crate::Pool, root_id: &str) -> sqlx::Result<Vec<i64>> {
    sqlx::query_scalar!(
        r#"SELECT w.id AS "id!" FROM works w
           WHERE w.metadata_src = 'none' AND w.missing_at IS NULL
             AND EXISTS (SELECT 1 FROM work_locations l
                         WHERE l.work_id = w.id AND l.root_id = $1 AND l.missing_at IS NULL)
           ORDER BY w.id"#,
        root_id
    )
    .fetch_all(read)
    .await
}

/// The images of every present work in `root_id` that has no cover, from
/// that root's present locations only, since only its storage reads them.
/// Copies of a file in another root stay in: that root may be the one that
/// cannot be read.
///
/// # Errors
/// Any database error.
pub async fn folder_images(read: &crate::Pool, root_id: &str) -> sqlx::Result<Vec<FolderImage>> {
    sqlx::query_as!(
        FolderImage,
        r#"SELECT w.id AS "work_id!", w.code, t.id AS "track_id!", l.rel_path AS location_path,
                  t.rel_path, t.display_path, t.size
           FROM works w
           JOIN tracks t ON t.work_id = w.id
           JOIN work_locations l ON l.id = t.location_id
           WHERE w.cover_src = 'none' AND w.missing_at IS NULL
             AND l.root_id = $1 AND l.missing_at IS NULL
             AND t.kind = 'image' AND t.missing_at IS NULL
           ORDER BY w.id, t.sort_key, t.id"#,
        root_id
    )
    .fetch_all(read)
    .await
}
