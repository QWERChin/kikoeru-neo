//! Admin edits: manual metadata, and the list and cleanup of missing works
//! and locations (book/src/scanning.md, "缺失和冲突").

use neo_core::db::edit::*;

use super::tags;

use neo_core::dto::{Id, MissingLocation, MissingReport, MissingWork, SafeInteger};

/// Replaces a work's metadata and marks it `manual`, so ordinary refreshes
/// leave it alone. Returns false when the work does not exist.
pub async fn set_metadata(
    pool: &crate::Pool,
    work_id: i64,
    edit: &MetadataEdit,
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
    let circle_id = match &edit.circle_name {
        Some(name) => Some(circle_id(&mut tx, name).await?),
        None => None,
    };
    sqlx::query!(
        "UPDATE works SET title = $1, release_date = $2, nsfw = $3, circle_id = $4,
             metadata_src = 'manual', updated_at = $5
         WHERE id = $6",
        edit.title,
        edit.release_date,
        edit.nsfw,
        circle_id,
        now,
        work_id
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!("DELETE FROM work_vas WHERE work_id = $1", work_id)
        .execute(&mut *tx)
        .await?;
    for name in &edit.va_names {
        let va = va_id(&mut tx, name).await?;
        sqlx::query!(
            "INSERT INTO work_vas (work_id, va_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            work_id,
            va
        )
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query!("DELETE FROM work_tags WHERE work_id = $1", work_id)
        .execute(&mut *tx)
        .await?;
    for name in &edit.tag_names {
        let tag = tags::resolve(&mut tx, name, None).await?;
        sqlx::query!(
            "INSERT INTO work_tags (work_id, tag_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            work_id,
            tag
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(true)
}

/// An existing circle with this name, or a new one in the custom range.
async fn circle_id(tx: &mut crate::Tx<'_>, name: &str) -> sqlx::Result<i64> {
    if let Some(id) = sqlx::query_scalar!(
        r#"SELECT id AS "id!" FROM circles WHERE name = $1 ORDER BY id LIMIT 1"#,
        name
    )
    .fetch_optional(&mut **tx)
    .await?
    {
        return Ok(id);
    }
    let used = sqlx::query_scalar!(r#"SELECT COALESCE(MAX(id), 0) AS "n!: i64" FROM circles"#)
        .fetch_one(&mut **tx)
        .await?;
    let id = used.max(CUSTOM_CIRCLE_BASE - 1) + 1;
    sqlx::query!("INSERT INTO circles (id, name) VALUES ($1, $2)", id, name)
        .execute(&mut **tx)
        .await?;
    Ok(id)
}

/// An existing voice actor with this name, or a new one under the ID the
/// old server would have given them.
pub async fn va_id(tx: &mut crate::Tx<'_>, name: &str) -> sqlx::Result<String> {
    if let Some(id) = sqlx::query_scalar!(
        r#"SELECT id AS "id!" FROM vas WHERE name = $1 ORDER BY id LIMIT 1"#,
        name
    )
    .fetch_optional(&mut **tx)
    .await?
    {
        return Ok(id);
    }
    let id = uuid::Uuid::new_v5(&VA_NAMESPACE, name.as_bytes()).to_string();
    sqlx::query!(
        "INSERT INTO vas (id, name) VALUES ($1, $2) ON CONFLICT (id) DO NOTHING",
        id,
        name
    )
    .execute(&mut **tx)
    .await?;
    Ok(id)
}

/// Works missing everywhere, and missing locations of works that are not.
pub async fn missing(pool: &crate::Pool) -> Result<MissingReport, MissingError> {
    let mut tx = pool.begin().await?;
    let works = sqlx::query!(
        r#"SELECT id AS "id!", code, title, missing_at AS "missing_at!"
           FROM works w WHERE missing_at IS NOT NULL AND NOT EXISTS (SELECT 1 FROM work_locations l
                              WHERE l.work_id = w.id AND l.missing_at IS NULL)
           ORDER BY missing_at DESC, id DESC"#
    )
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|r| {
        let invalid = |reason| MissingError::InvalidRow { id: r.id, reason };
        Ok(MissingWork {
            id: Id::try_from(r.id).map_err(invalid)?,
            code: r.code,
            title: r.title,
            missing_at: SafeInteger::try_from(r.missing_at).map_err(invalid)?,
        })
    })
    .collect::<Result<_, MissingError>>()?;
    let locations = sqlx::query!(
        r#"SELECT l.id AS "id!", l.work_id, w.code, l.root_id, l.display_path,
                  l.missing_at AS "missing_at!"
           FROM work_locations l JOIN works w ON w.id = l.work_id
           WHERE l.missing_at IS NOT NULL AND w.missing_at IS NULL
           ORDER BY l.missing_at DESC, l.id DESC"#
    )
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|r| {
        let invalid = |reason| MissingError::InvalidRow { id: r.id, reason };
        Ok(MissingLocation {
            id: Id::try_from(r.id).map_err(invalid)?,
            work_id: Id::try_from(r.work_id).map_err(invalid)?,
            code: r.code,
            root_id: r.root_id,
            path: r.display_path,
            missing_at: SafeInteger::try_from(r.missing_at).map_err(invalid)?,
        })
    })
    .collect::<Result<_, MissingError>>()?;
    Ok(MissingReport { works, locations })
}

/// Deletes the given works and locations that are still missing. Anything
/// that came back in the meantime is skipped. One transaction, so a
/// failure deletes nothing.
pub async fn cleanup(
    pool: &crate::Pool,
    work_ids: &[i64],
    location_ids: &[i64],
) -> sqlx::Result<Cleanup> {
    let mut done = Cleanup::default();
    let mut tx = pool.begin().await?;
    for &id in location_ids {
        let missing = sqlx::query_scalar!(
            r#"SELECT EXISTS (SELECT 1 FROM work_locations
                              WHERE id = $1 AND missing_at IS NOT NULL) AS "e!: bool""#,
            id
        )
        .fetch_one(&mut *tx)
        .await?;
        if !missing {
            continue;
        }
        // A set track_id is how import matching recognises newer playback,
        // so pending imports it would have stopped are settled first.
        sqlx::query!(
            "UPDATE import_pending SET state = 'superseded'
             WHERE state = 'pending' AND (user_id, work_id) IN (
               SELECT user_id, work_id FROM play_progress
               WHERE track_id IN (SELECT id FROM tracks WHERE location_id = $1))",
            id
        )
        .execute(&mut *tx)
        .await?;
        // Progress and copies elsewhere must stop pointing at the tracks
        // about to go; play_progress.track_id has no cascade.
        sqlx::query!(
            "UPDATE play_progress SET track_id = NULL
             WHERE track_id IN (SELECT id FROM tracks WHERE location_id = $1)",
            id
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!(
            "UPDATE tracks SET duplicate_of = NULL
             WHERE duplicate_of IN (SELECT id FROM tracks WHERE location_id = $1)",
            id
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!("DELETE FROM tracks WHERE location_id = $1", id)
            .execute(&mut *tx)
            .await?;
        sqlx::query!("DELETE FROM work_locations WHERE id = $1", id)
            .execute(&mut *tx)
            .await?;
        done.locations += 1;
    }
    for &id in work_ids {
        let missing = sqlx::query_scalar!(
            // A scan clears a location's flag before the work's, in a
            // later transaction, so a present location also means the
            // work is back.
            r#"SELECT EXISTS (SELECT 1 FROM works w
                              WHERE id = $1 AND missing_at IS NOT NULL
                                AND NOT EXISTS (SELECT 1 FROM work_locations l
                                                WHERE l.work_id = w.id AND l.missing_at IS NULL))
               AS "e!: bool""#,
            id
        )
        .fetch_one(&mut *tx)
        .await?;
        if !missing {
            continue;
        }
        // Progress goes first: it references tracks without a cascade.
        // Favorites, imports and the rest go with the work.
        sqlx::query!("DELETE FROM play_progress WHERE work_id = $1", id)
            .execute(&mut *tx)
            .await?;
        sqlx::query!(
            "UPDATE tracks SET duplicate_of = NULL WHERE work_id = $1",
            id
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query!("DELETE FROM works WHERE id = $1", id)
            .execute(&mut *tx)
            .await?;
        done.works.push(id);
    }
    tx.commit().await?;
    Ok(done)
}
