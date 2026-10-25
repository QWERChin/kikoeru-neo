//! Measured loudness per track (`track_analysis`, book/src/data-model.md).
//! A row describes the file at the size and mtime it was measured at, so
//! a changed file reads as not measured.

/// A file without an mtime can change without its size changing, so its
/// measurement expires like the cache's entries for such files.
pub const UNKNOWN_MTIME_TTL_MS: i64 = 7 * 24 * 3600 * 1000;

/// A stored measurement. Both values are `None` for a track too quiet to
/// measure.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StoredLoudness {
    pub integrated_lufs: Option<f64>,
    pub true_peak_dbtp: Option<f64>,
}
