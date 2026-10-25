//! Rows the scan reads and writes (book/src/scanning.md). The scan's own
//! logic stays in neo-scanner; these are only the shapes of its queries.

/// A location of a root as the last scan left it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocationRow {
    pub id: i64,
    pub work_id: i64,
    pub rel_path: Vec<u8>,
    pub missing: bool,
}

/// Whether a work is marked missing, and whether any of its locations is
/// present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkState {
    pub missing: bool,
    pub present: bool,
}

/// A track of a location as the last scan left it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackRow {
    pub id: i64,
    pub rel_path: Vec<u8>,
    pub size: Option<i64>,
    pub mtime: Option<i64>,
    pub missing: bool,
    pub probed: bool,
}

/// A file found in a listing. `kind` is `names::Kind::as_str()`.
#[derive(Debug, Clone, Copy)]
pub struct ListedFile<'a> {
    pub display: &'a str,
    pub sort_key: &'a str,
    pub kind: &'a str,
    pub size: Option<i64>,
    pub mtime: Option<i64>,
}

/// One copy of a file of a work, for picking the copy that is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyRow {
    pub id: i64,
    pub location_id: i64,
    pub root_id: String,
    pub rel_path: Vec<u8>,
    pub size: Option<i64>,
    pub missing: bool,
    pub duplicate_of: Option<i64>,
}

/// An imported play position still waiting for its track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRow {
    pub user_id: i64,
    pub source: String,
}
