//! Rows `import-legacy` writes (book/src/migration.md).

/// A work from the old library, with its old metadata.
#[derive(Debug, Clone, Copy)]
pub struct ImportedWork<'a> {
    pub code: &'a str,
    pub title: &'a str,
    pub circle_id: Option<i64>,
    pub release_date: Option<&'a str>,
    pub nsfw: bool,
    pub dl_count: Option<i64>,
    pub price: Option<i64>,
    pub rate_average: Option<f64>,
    pub rate_count: Option<i64>,
    pub metadata_src: &'a str,
    pub cover_src: &'a str,
    pub extra: &'a str,
    pub created_at: i64,
    pub updated_at: i64,
}
