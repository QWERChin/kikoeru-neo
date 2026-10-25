//! `legacy_user_state`: old favorites, reviews and play positions that wait
//! for their work to be scanned. "合并到已有的库" in book/src/migration.md
//! has the rules.

/// One old user's data for one work code.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct State {
    pub user_id: i64,
    pub code: String,
    pub favorite_at: Option<i64>,
    pub review: Option<Review>,
    pub progress: Option<Progress>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Review {
    pub rating: Option<i64>,
    pub review_text: Option<String>,
    pub progress: Option<String>,
    /// The old row's update time, when it had a valid one.
    pub updated_at: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Progress {
    pub updated_at: i64,
    pub position_ms: i64,
    pub finished_at: Option<i64>,
    /// The `import_pending` row that matches the position to a track after
    /// the scan: its source, durations and state.
    pub pending: Option<(String, Option<String>, String)>,
}

/// What `claim` did, for scan summaries and the merge report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Claimed {
    pub favorites: u64,
    pub reviews: u64,
    pub progress: u64,
    /// Old positions dropped because the library has a newer one.
    pub progress_kept: u64,
}
