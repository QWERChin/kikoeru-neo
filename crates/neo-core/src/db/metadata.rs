//! Rows for fetching and storing a work's metadata (book/src/scanning.md,
//! "元数据").

/// What decides whether a fetch may write a work and its cover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sources {
    pub code: String,
    pub metadata_src: String,
    pub cover_src: String,
}

/// A work as the storing transaction reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    pub metadata_src: String,
    pub cover_src: String,
    pub cover_version: i64,
    pub extra: String,
}

/// The columns a fetch writes on `works`. `extra` is the whole JSON object
/// to store, and `source` the new `metadata_src`.
#[derive(Debug, Clone, Copy)]
pub struct Fields<'a> {
    pub title: &'a str,
    pub circle_id: Option<i64>,
    pub release_date: Option<&'a str>,
    pub nsfw: bool,
    pub dl_count: Option<i64>,
    pub price: Option<i64>,
    pub rate_average: Option<f64>,
    pub rate_count: Option<i64>,
    pub extra: &'a str,
    pub source: &'a str,
}

/// An image in a work's directory that may stand in for a missing cover
/// (book/src/scanning.md, "封面"). Paths are raw bytes, like `tracks`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderImage {
    pub work_id: i64,
    pub code: String,
    pub track_id: i64,
    pub location_path: Vec<u8>,
    pub rel_path: Vec<u8>,
    pub display_path: String,
    pub size: Option<i64>,
}
