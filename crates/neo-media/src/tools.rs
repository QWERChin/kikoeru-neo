//! Which ffmpeg and ffprobe to run.

use std::path::PathBuf;

/// The ffmpeg and ffprobe executables. The server package puts both on
/// `PATH`, so the default is the bare names. The desktop app passes the
/// binaries it bundles (book/src/desktop.md, `media.ffmpeg` and
/// `media.ffprobe`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

impl Default for Tools {
    fn default() -> Self {
        Self {
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
        }
    }
}
