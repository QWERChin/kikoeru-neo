//! Writes to `play_progress`, following "收听状态怎么算" and "什么算播完"
//! in book/src/data-model.md.

#[derive(Debug, Clone, Copy)]
pub struct ReportOrder<'a> {
    pub session_id: &'a str,
    pub sequence: i64,
}

#[derive(Debug, Clone, Copy)]
pub struct ReportInput<'a> {
    pub track_id: i64,
    pub position_ms: i64,
    pub group_path: &'a str,
    pub report_order: Option<ReportOrder<'a>>,
}

/// Where a track sits, for checking a progress report against its work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackPlace {
    pub work_id: i64,
    pub display_path: String,
    pub kind: String,
}
