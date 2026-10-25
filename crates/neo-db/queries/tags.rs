//! Canonical tags with every raw name seen for them as an alias
//! (book/src/scanning.md, "标签合并").

use neo_core::db::tags::*;

/// The canonical tag for a raw name, creating what is missing. A known
/// alias wins. Otherwise a DLsite genre ID names the tag, and a name with
/// neither becomes a new tag in the custom range.
pub async fn resolve(
    tx: &mut crate::Tx<'_>,
    name: &str,
    dlsite_id: Option<i64>,
) -> sqlx::Result<i64> {
    if let Some(id) = sqlx::query_scalar!("SELECT tag_id FROM tag_aliases WHERE alias = $1", name)
        .fetch_optional(&mut **tx)
        .await?
    {
        return Ok(id);
    }
    let id = match dlsite_id.filter(|id| *id > 0 && *id < CUSTOM_TAG_BASE) {
        Some(id) => {
            let id = follow_merges(tx, id).await?;
            sqlx::query!(
                "INSERT INTO tags (id, name) VALUES ($1, $2) ON CONFLICT (id) DO NOTHING",
                id,
                name
            )
            .execute(&mut **tx)
            .await?;
            id
        }
        None => {
            // Above every ID ever used, merged-away ones included, so a new
            // tag cannot take over an old merge record.
            let used = sqlx::query_scalar!(
                r#"SELECT COALESCE(MAX(id), 0) AS "n!: i64" FROM (
                       SELECT id FROM tags
                       UNION ALL SELECT from_id FROM tag_merges
                       UNION ALL SELECT into_id FROM tag_merges
                   ) AS used"#
            )
            .fetch_one(&mut **tx)
            .await?;
            let next = used.max(CUSTOM_TAG_BASE - 1) + 1;
            sqlx::query!("INSERT INTO tags (id, name) VALUES ($1, $2)", next, name)
                .execute(&mut **tx)
                .await?;
            next
        }
    };
    sqlx::query!(
        "INSERT INTO tag_aliases (alias, tag_id) VALUES ($1, $2)",
        name,
        id
    )
    .execute(&mut **tx)
    .await?;
    Ok(id)
}

/// The tag an ID was merged into, following chains of merges, or the ID
/// itself. A cycle, possible only in hand-edited data, stops the walk.
async fn follow_merges(tx: &mut crate::Tx<'_>, mut id: i64) -> sqlx::Result<i64> {
    let mut seen = std::collections::HashSet::new();
    while seen.insert(id) {
        match sqlx::query_scalar!("SELECT into_id FROM tag_merges WHERE from_id = $1", id)
            .fetch_optional(&mut **tx)
            .await?
        {
            Some(into) => id = into,
            None => break,
        }
    }
    Ok(id)
}

/// Moves `from`'s works and aliases to `into` and deletes `from`.
pub async fn merge(
    pool: &crate::Pool,
    from: i64,
    into: i64,
    now: i64,
) -> sqlx::Result<MergeOutcome> {
    if from == into {
        return Ok(MergeOutcome::SameTag);
    }
    let mut tx = pool.begin().await?;
    let found = sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "n!: i64" FROM tags WHERE id IN ($1, $2)"#,
        from,
        into
    )
    .fetch_one(&mut *tx)
    .await?;
    if found != 2 {
        return Ok(MergeOutcome::NotFound);
    }
    sqlx::query!(
        "INSERT INTO work_tags (work_id, tag_id)
         SELECT work_id, $1 FROM work_tags WHERE tag_id = $2
         ON CONFLICT (work_id, tag_id) DO NOTHING",
        into,
        from
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "UPDATE tag_aliases SET tag_id = $1 WHERE tag_id = $2",
        into,
        from
    )
    .execute(&mut *tx)
    .await?;
    // The merged tag's own name stays findable as an alias of the target,
    // unless a third tag already owns that name.
    sqlx::query!(
        "INSERT INTO tag_aliases (alias, tag_id) SELECT name, $1 FROM tags WHERE id = $2
         ON CONFLICT (alias) DO NOTHING",
        into,
        from
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!("DELETE FROM tags WHERE id = $1", from)
        .execute(&mut *tx)
        .await?;
    sqlx::query!(
        "INSERT INTO tag_merges (from_id, into_id, merged_at) VALUES ($1, $2, $3)
         ON CONFLICT (from_id) DO UPDATE SET into_id = excluded.into_id,
             merged_at = excluded.merged_at",
        from,
        into,
        now
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(MergeOutcome::Merged)
}
