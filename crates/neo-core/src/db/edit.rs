//! Admin edits: manual metadata, and the list and cleanup of missing works
//! and locations (book/src/scanning.md, "缺失和冲突").

/// Circles created by hand get IDs above every DLsite circle code (the
/// RG/BG/VG encoding stays below 3 * 10^9) and above the import's range for
/// the old server's hand-made circles.
pub const CUSTOM_CIRCLE_BASE: i64 = 9_000_000_000;

/// The old server derived voice actor IDs from their names with this UUID
/// namespace, so a voice actor added by hand gets the ID a scrape or the
/// import would give the same name.
pub const VA_NAMESPACE: uuid::Uuid = uuid::uuid!("699d9c07-b965-4399-bafd-18a3cacf073c");

/// Validated input for [`set_metadata`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataEdit {
    pub title: String,
    pub release_date: Option<String>,
    pub nsfw: bool,
    pub circle_name: Option<String>,
    pub va_names: Vec<String>,
    pub tag_names: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum MissingError {
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error("row {id} has an invalid value: {reason}")]
    InvalidRow { id: i64, reason: &'static str },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cleanup {
    /// IDs of the works deleted, so the caller can remove their covers.
    pub works: Vec<i64>,
    pub locations: u64,
}
