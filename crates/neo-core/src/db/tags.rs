//! Canonical tags with every raw name seen for them as an alias
//! (book/src/scanning.md, "标签合并").

/// Tags created by hand or for a name DLsite gave without a genre ID get
/// IDs from here up, so they never collide with a DLsite genre ID.
pub const CUSTOM_TAG_BASE: i64 = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeOutcome {
    Merged,
    SameTag,
    NotFound,
}
