//! Lookups for the media endpoints: where a track's file is, and which
//! subtitle belongs to it.

/// A track with everything needed to open its file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackFile {
    pub id: i64,
    pub work_id: i64,
    pub location_id: i64,
    pub kind: String,
    pub display_path: String,
    /// Raw path below the work directory.
    pub rel_path: Vec<u8>,
    pub size: Option<i64>,
    pub mtime: Option<i64>,
    pub duration_ms: Option<i64>,
    pub probe: Option<String>,
    /// The track or its whole location is marked missing.
    pub missing: bool,
    pub root_id: String,
    /// Changes when an administrator points the root at another source.
    pub source_version: i64,
    /// Raw path of the work directory below the root.
    pub location_path: Vec<u8>,
}

/// Subtitle extensions in the order `/api/tracks/{id}/subtitle` picks them.
pub const SUBTITLE_EXTENSIONS: [&str; 5] = ["vtt", "srt", "lrc", "ass", "ssa"];

/// The extensions `subtitle?format=ass` returns as they are.
pub const ASS_EXTENSIONS: [&str; 2] = ["ass", "ssa"];

/// More fonts than this in one work are not listed, so a font collection
/// shipped with a work cannot make JASSUB fetch hundreds of files.
pub const MAX_FONTS: usize = 64;

/// Whether a work has a cover, and its version for cache keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoverState {
    pub has_cover: bool,
    pub cover_version: i64,
}
