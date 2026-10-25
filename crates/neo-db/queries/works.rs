use neo_core::db::works::*;
use sqlx::{FromRow, QueryBuilder};

use neo_core::{
    dto::{CircleRef, Id, SafeInteger, WorkSummary},
    progress::ProgressRow,
};

#[derive(FromRow)]
struct Row {
    id: i64,
    code: String,
    title: String,
    circle_id: Option<i64>,
    circle_name: Option<String>,
    release_date: Option<String>,
    nsfw: bool,
    rate_average: Option<f64>,
    rate_count: Option<i64>,
    has_cover: bool,
    cover_version: i64,
    missing: bool,
    favorite: bool,
    has_progress: bool,
    track_id: Option<i64>,
    position_ms: Option<i64>,
    finished_at: Option<i64>,
    replaying: Option<bool>,
    track_duration_ms: Option<i64>,
    track_present: bool,
    group_path: Option<String>,
    my_rating: Option<i64>,
}

impl Row {
    fn into_dto(self) -> Result<WorkSummary, &'static str> {
        let circle = match (self.circle_id, self.circle_name) {
            (Some(id), Some(name)) => Some(CircleRef {
                id: Id::try_from(id)?,
                name,
            }),
            _ => None,
        };
        let progress = if self.has_progress {
            Some(
                ProgressRow {
                    track_id: self.track_id,
                    position_ms: self.position_ms.unwrap_or(0),
                    finished_at: self.finished_at,
                    replaying: self.replaying.unwrap_or(false),
                    track_duration_ms: self.track_duration_ms,
                    track_present: self.track_present,
                    group_path: self.group_path,
                }
                .to_dto()?,
            )
        } else {
            None
        };
        Ok(WorkSummary {
            id: Id::try_from(self.id)?,
            code: self.code,
            title: self.title,
            circle,
            release_date: self.release_date,
            nsfw: self.nsfw,
            rate_average: self.rate_average,
            rate_count: self
                .rate_count
                .map(u32::try_from)
                .transpose()
                .map_err(|_| "rate_count is out of range")?,
            has_cover: self.has_cover,
            cover_version: SafeInteger::try_from(self.cover_version)?,
            missing: self.missing,
            favorite: self.favorite,
            progress,
            my_rating: self
                .my_rating
                .map(u8::try_from)
                .transpose()
                .map_err(|_| "my_rating is out of range")?,
        })
    }
}

/// Turns a search text into a LIKE pattern that matches it literally.
fn like_pattern(q: &str) -> String {
    let mut pattern = String::from("%");
    for c in q.chars() {
        if matches!(c, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern.push('%');
    pattern
}

/// The WHERE clause shared by the page and its count. The summary query
/// joins `circles c` and `play_progress p`, and so does the count query.
fn push_where(sql: &mut QueryBuilder<crate::Database>, query: &ListQuery) {
    sql.push(" WHERE 1 = 1");
    let f = &query.filter;
    // Favorites, history and reviews are the user's own records, so they
    // keep works that went missing.
    if query.scope == Scope::Library && !f.include_missing {
        sql.push(" AND w.missing_at IS NULL");
    }
    match query.scope {
        Scope::Library => {}
        Scope::Favorites => {
            sql.push(
                " AND EXISTS (SELECT 1 FROM favorites fs WHERE fs.work_id = w.id AND fs.user_id = ",
            );
            sql.push_bind(query.user_id);
            sql.push(")");
        }
        Scope::History => {
            sql.push(" AND p.work_id IS NOT NULL");
        }
        Scope::Reviews => {
            sql.push(
                " AND EXISTS (SELECT 1 FROM reviews rs WHERE rs.work_id = w.id AND rs.user_id = ",
            );
            sql.push_bind(query.user_id);
            sql.push(")");
        }
    }
    if let Some(q) = &f.q {
        // SQLite's LIKE ignores ASCII case and PostgreSQL's does not, so
        // the columns go through lower(), which folds only ASCII on both
        // (SQLite's lower() and PostgreSQL's under COLLATE "C"). The search
        // text is folded the same way here: lower($n) would use
        // PostgreSQL's default collation, which may fold other letters too.
        let pattern = like_pattern(q).to_ascii_lowercase();
        sql.push(" AND (lower(w.code) LIKE ");
        sql.push_bind(pattern.clone());
        sql.push(" ESCAPE '\\' OR lower(w.title) LIKE ");
        sql.push_bind(pattern.clone());
        sql.push(" ESCAPE '\\' OR lower(c.name) LIKE ");
        sql.push_bind(pattern.clone());
        sql.push(
            " ESCAPE '\\' OR EXISTS (SELECT 1 FROM work_vas qv JOIN vas qa ON qa.id = qv.va_id
                                    WHERE qv.work_id = w.id AND lower(qa.name) LIKE ",
        );
        sql.push_bind(pattern);
        sql.push(" ESCAPE '\\'))");
    }
    if let Some(tag) = f.tag {
        sql.push(
            " AND EXISTS (SELECT 1 FROM work_tags ft WHERE ft.work_id = w.id AND ft.tag_id = ",
        );
        sql.push_bind(tag);
        sql.push(")");
    }
    if let Some(circle) = f.circle {
        sql.push(" AND w.circle_id = ");
        sql.push_bind(circle);
    }
    if let Some(va) = &f.va {
        sql.push(" AND EXISTS (SELECT 1 FROM work_vas fv WHERE fv.work_id = w.id AND fv.va_id = ");
        sql.push_bind(va.clone());
        sql.push(")");
    }
    if let Some(root) = &f.root {
        sql.push(
            " AND EXISTS (SELECT 1 FROM work_locations fl
                          WHERE fl.work_id = w.id AND fl.missing_at IS NULL AND fl.root_id = ",
        );
        sql.push_bind(root.clone());
        sql.push(")");
    }
}

/// How many works the query matches, for the page's `total`.
pub async fn count(pool: &crate::Pool, query: &ListQuery) -> sqlx::Result<i64> {
    let mut sql = QueryBuilder::<crate::Database>::new(
        "SELECT count(*) FROM works w
         LEFT JOIN circles c ON c.id = w.circle_id
         LEFT JOIN play_progress p ON p.work_id = w.id AND p.user_id = ",
    );
    sql.push_bind(query.user_id);
    push_where(&mut sql, query);
    sql.build_query_scalar().fetch_one(pool).await
}

/// Joins `rr.rnd`, a 32-bit hash of each work ID mixed with the seed, for
/// `Sort::Random`. It is the integer hash `x = ((x >> 16) ^ x) * 0x45d9f3b`
/// applied twice, written with `|`, `&` and `-` because SQLite has no XOR.
/// Each round is its own derived table so the expression never repeats.
/// Products stay below 2^59, inside SQLite's integer range.
fn push_random_join(sql: &mut QueryBuilder<crate::Database>, seed: u32) {
    const ROUND: &str =
        "SELECT rid, ((((x >> 16) | x) - ((x >> 16) & x)) * 73244475) & 4294967295 AS x FROM ";
    sql.push(" JOIN (SELECT rid, ((x >> 16) | x) - ((x >> 16) & x) AS rnd FROM (");
    sql.push(ROUND);
    sql.push("(");
    sql.push(ROUND);
    sql.push("(SELECT id AS rid, ((id & 4294967295) | ");
    sql.push_bind(i64::from(seed));
    sql.push(") - ((id & 4294967295) & ");
    sql.push_bind(i64::from(seed));
    // PostgreSQL before 16 needs a name for every subquery in FROM.
    sql.push(") AS x FROM works) AS h0) AS h1) AS h2) rr ON rr.rid = w.id ");
}

fn push_order(sql: &mut QueryBuilder<crate::Database>, query: &ListQuery) {
    // The direction cannot be a bound parameter. It comes from the `Order`
    // enum, never from the request text.
    let dir = query.order.sql();
    match query.scope {
        Scope::Favorites => {
            sql.push(" ORDER BY (SELECT fo.created_at FROM favorites fo WHERE fo.work_id = w.id AND fo.user_id = ");
            sql.push_bind(query.user_id);
            sql.push(") DESC, w.id DESC");
            return;
        }
        Scope::History => {
            sql.push(" ORDER BY p.updated_at DESC, w.id DESC");
            return;
        }
        // Imported reviews may have no time and go last.
        Scope::Reviews => {
            sql.push(" ORDER BY (SELECT ro.updated_at FROM reviews ro WHERE ro.work_id = w.id AND ro.user_id = ");
            sql.push_bind(query.user_id);
            sql.push(") IS NULL, (SELECT ro.updated_at FROM reviews ro WHERE ro.work_id = w.id AND ro.user_id = ");
            sql.push_bind(query.user_id);
            sql.push(") DESC, w.id DESC");
            return;
        }
        Scope::Library => {}
    }
    match query.sort {
        Sort::Added => {
            sql.push(format!(" ORDER BY w.created_at {dir}, w.id {dir}"));
        }
        // Works without the value sort last in both directions.
        Sort::Release => {
            sql.push(format!(
                " ORDER BY w.release_date IS NULL, w.release_date {dir}, w.id {dir}"
            ));
        }
        Sort::Rating => {
            sql.push(format!(
                " ORDER BY w.rate_average IS NULL, w.rate_average {dir}, w.id {dir}"
            ));
        }
        Sort::Random { .. } => {
            sql.push(" ORDER BY rr.rnd, w.id");
        }
    }
}

/// The `WorkSummary` columns for one user, ending after the joins so the
/// caller adds the WHERE clause.
fn summary_select(user_id: i64) -> QueryBuilder<crate::Database> {
    let mut sql = QueryBuilder::<crate::Database>::new(
        "SELECT w.id, w.code, w.title, w.circle_id, c.name AS circle_name,
                w.release_date, w.nsfw, w.rate_average, w.rate_count,
                w.cover_src <> 'none' AS has_cover, w.cover_version,
                w.missing_at IS NOT NULL AS missing,
                EXISTS (SELECT 1 FROM favorites f
                        WHERE f.user_id = p_user.id AND f.work_id = w.id) AS favorite,
                (SELECT r.rating FROM reviews r
                 WHERE r.user_id = p_user.id AND r.work_id = w.id) AS my_rating,
                p.work_id IS NOT NULL AS has_progress,
                p.track_id, p.position_ms, p.finished_at, p.replaying, p.group_path,
                t.duration_ms AS track_duration_ms,
                (t.id IS NOT NULL AND t.missing_at IS NULL) AS track_present
         FROM works w
         CROSS JOIN (SELECT ",
    );
    sql.push_bind(user_id);
    sql.push(
        " AS id) p_user
         LEFT JOIN circles c ON c.id = w.circle_id
         LEFT JOIN play_progress p ON p.user_id = p_user.id AND p.work_id = w.id
         LEFT JOIN tracks t ON t.id = p.track_id ",
    );
    sql
}

/// One work as the list shows it, missing or not.
pub async fn summary(
    read: impl sqlx::Executor<'_, Database = crate::Database>,
    user_id: i64,
    work_id: i64,
) -> Result<Option<WorkSummary>, ListError> {
    let mut sql = summary_select(user_id);
    sql.push("WHERE w.id = ");
    sql.push_bind(work_id);
    let row: Option<Row> = sql.build_query_as().fetch_optional(read).await?;
    row.map(|row| {
        row.into_dto().map_err(|reason| ListError::InvalidRow {
            id: work_id,
            reason,
        })
    })
    .transpose()
}

pub async fn list(pool: &crate::Pool, query: &ListQuery) -> Result<Vec<WorkSummary>, ListError> {
    let mut sql = summary_select(query.user_id);
    if let (Scope::Library, Sort::Random { seed }) = (query.scope, query.sort) {
        push_random_join(&mut sql, seed);
    }
    push_where(&mut sql, query);
    push_order(&mut sql, query);
    sql.push(" LIMIT ");
    sql.push_bind(i64::from(query.limit));
    sql.push(" OFFSET ");
    sql.push_bind(i64::try_from(query.offset).unwrap_or(i64::MAX));
    let rows: Vec<Row> = sql.build_query_as().fetch_all(pool).await?;
    rows.into_iter()
        .map(|row| {
            let id = row.id;
            row.into_dto()
                .map_err(|reason| ListError::InvalidRow { id, reason })
        })
        .collect()
}

/// # Errors
/// Any database error.
pub async fn exists(pool: &crate::Pool, work_id: i64) -> sqlx::Result<bool> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM works WHERE id = $1) AS "e!: bool""#,
        work_id
    )
    .fetch_one(pool)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_text_is_literal() {
        assert_eq!(like_pattern("a%b_c\\d"), "%a\\%b\\_c\\\\d%");
        assert_eq!(like_pattern("音声"), "%音声%");
    }
}
