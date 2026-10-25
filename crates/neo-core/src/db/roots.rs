/// A root added through the API, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRoot {
    pub id: String,
    pub display_name: String,
    pub kind: String,
    /// JSON settings without the password.
    pub settings: String,
}

/// A root's row for the settings page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootRow {
    pub display_name: String,
    pub status: String,
    pub last_scan_at: Option<i64>,
    /// Works with a present location in the root.
    pub work_count: i64,
}
