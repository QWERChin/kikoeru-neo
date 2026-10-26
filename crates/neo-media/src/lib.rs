//! Media handling for kikoeru-neo: byte ranges, ffprobe, the AAC fallback
//! and HLS for video with their cache, and streamed ZIP downloads. All decoding happens in
//! ffmpeg and ffprobe subprocesses.

mod analysis;
mod cache;
mod decide;
mod diagnostics;
mod error;
mod hls;
mod input;
mod mime;
mod probe;
pub mod range;
pub mod subtitle;
mod tools;
mod transcode;
pub mod zip;

pub use analysis::{AnalysisJob, Loudness, WAVEFORM_BUCKETS, WAVEFORM_PARAMS, analyze};
pub use cache::{CacheKey, CachedFile, HlsSegment, TranscodeCache};
pub use decide::{Playback, VideoPlayback, decide, decide_video};
pub use error::MediaError;
pub use hls::{
    HLS_COPY_PARAMS, HLS_TRANSCODE_PARAMS, HlsJob, HlsMode, PLAYLIST, is_segment_name,
    sign_playlist, transcode_hls,
};
pub use input::Input;
pub use mime::mime_type;
pub use probe::{AudioStream, Probe, VideoStream, probe};
pub use tools::Tools;
pub use transcode::{AAC_PARAMS, Job, transcode_aac};
