//! Video as HLS: a whole playlist with 6-second MPEG-TS segments, written
//! into one cache directory and published together (book/src/media.md).

use std::{ffi::OsString, future::Future, path::Path, time::Duration};

use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::{
    CacheKey, CachedFile, Input, MediaError, TranscodeCache,
    cache::create_private_dir,
    transcode::{TempFile, run_ffmpeg, temp_name},
};

/// Cache key `params` for H.264 + AAC copied into HLS without re-encoding.
pub const HLS_COPY_PARAMS: &str = "hls-copy-6s";
/// Cache key `params` for a transcode to H.264 + AAC. The limits are in the
/// key, so a set made before them, or under other limits, is never served.
pub const HLS_TRANSCODE_PARAMS: &str = "hls-h264-aac-6s-1080p-8m";
/// The playlist inside an HLS cache directory.
pub const PLAYLIST: &str = "index.m3u8";
/// The ffmpeg pattern for segment names, which [`is_segment_name`] accepts.
const SEGMENT_PATTERN: &str = "seg-%05d.ts";
const SEGMENT_SECONDS: &str = "6";
/// Fits the picture inside 1920x1080, or 1080x1920 when it is taller than
/// wide, without enlarging it and keeping its shape. yuv420p needs even
/// dimensions, which an odd-sized source does not have, so both are rounded
/// down to even. Losing at most one pixel row or column is invisible.
const SCALE_TO_1080P: &str = "scale=w='min(iw,if(gte(iw,ih),1920,1080))'\
    :h='min(ih,if(gte(iw,ih),1080,1920))'\
    :force_original_aspect_ratio=decrease:force_divisible_by=2";
/// The bitrate ceiling. CRF 23 alone lets a busy 1080p scene go far above
/// what a phone on a remote connection can stream, so x264 keeps a VBV
/// buffer of two seconds at this rate.
const MAX_VIDEO_BITRATE: &str = "8M";
const VIDEO_BUFFER: &str = "16M";
const VERIFY_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HlsMode {
    /// The source is already H.264 + AAC, so only the container changes.
    Copy,
    Transcode,
}

impl HlsMode {
    pub fn params(self) -> &'static str {
        match self {
            Self::Copy => HLS_COPY_PARAMS,
            Self::Transcode => HLS_TRANSCODE_PARAMS,
        }
    }
}

pub struct HlsJob<'a> {
    pub input: &'a Input,
    pub key: &'a CacheKey,
    pub mode: HlsMode,
    pub duration_ms: Option<i64>,
    pub timeout: Duration,
}

/// `seg-00000.ts` and the like, the only names served from an HLS
/// directory. Anything else, such as `../x` or the playlist, is refused.
pub fn is_segment_name(name: &str) -> bool {
    name.strip_prefix("seg-")
        .and_then(|rest| rest.strip_suffix(".ts"))
        .is_some_and(|digits| digits.len() == 5 && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// Writes the HLS set for `job.input` into a temporary directory, checks
/// it, and renames the directory into the cache under `job.key`. The steps
/// and failure handling match [`crate::transcode_aac`]: cancellation and
/// the deadline hold until the rename, `restat` must still match the key,
/// and the temporary directory goes away on every error.
///
/// # Errors
/// Low disk space, ffmpeg failure, timeout, cancellation, a playlist that
/// does not check out, or a source that changed meanwhile.
pub async fn transcode_hls<F>(
    cache: &TranscodeCache,
    job: HlsJob<'_>,
    progress: &watch::Sender<f64>,
    cancel: &CancellationToken,
    restat: F,
) -> Result<CachedFile, MediaError>
where
    F: Future<Output = Result<(u64, Option<i64>), MediaError>>,
{
    debug_assert_eq!(job.key.params, job.mode.params());
    if let Some(cached) = cache.lookup(job.key)? {
        return Ok(cached);
    }
    let pin = cache.pin(job.key);
    cache.check_space()?;
    let deadline = tokio::time::Instant::now() + job.timeout;
    let tmp = TempFile::new(cache.tmp_dir().join(temp_name(job.key, "hls")));
    create_private_dir(&tmp.0)?;
    // ffmpeg reads %d, %v and others in the segment and playlist names as
    // patterns, and a data directory may contain them. So it runs in the
    // set's directory and gets fixed relative names.
    run_ffmpeg(
        cache.tools(),
        job.input,
        &output_args(job.mode),
        Some(&tmp.0),
        job.duration_ms,
        progress,
        cancel,
        deadline,
    )
    .await?;

    let verify = async {
        verify_playlist(cache.tools(), &tmp.0).await?;
        let (size, mtime) = restat.await?;
        if size != job.key.size || mtime != job.key.mtime {
            return Err(MediaError::SourceChanged);
        }
        Ok(())
    };
    tokio::select! {
        result = verify => result?,
        () = cancel.cancelled() => return Err(MediaError::Cancelled),
        () = tokio::time::sleep_until(deadline) => return Err(MediaError::Timeout("ffmpeg")),
    }
    if cancel.is_cancelled() {
        return Err(MediaError::Cancelled);
    }
    publish(cache, &tmp, &cache.final_path(job.key)).await?;
    tmp.disarm();
    let _ = progress.send(1.0);
    if let Err(error) = cache.evict() {
        tracing::warn!(%error, "cache eviction failed");
    }
    Ok(cache.cached_file(job.key, pin)?)
}

/// Renames the new set into place. An older set under the same key can
/// still be there: one that expired while a segment of it was being sent
/// stays pinned, and a full directory cannot be replaced by rename. It is
/// moved into `cache/tmp/` first and deleted after, which leaves files
/// already open readable.
async fn publish(cache: &TranscodeCache, tmp: &TempFile, target: &Path) -> Result<(), MediaError> {
    let retired = match tokio::fs::symlink_metadata(target).await {
        Ok(_) => {
            let retired = cache
                .tmp_dir()
                .join(format!("{}.retired", temp_name_for(target)));
            tokio::fs::rename(target, &retired).await?;
            Some(retired)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    tokio::fs::rename(&tmp.0, target).await?;
    if let Some(retired) = retired {
        let _ = tokio::fs::remove_dir_all(retired).await;
    }
    Ok(())
}

fn temp_name_for(target: &Path) -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!("{name}.{}.{n}", std::process::id())
}

fn output_args(mode: HlsMode) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["-map", "0:v:0", "-map", "0:a:0?", "-sn", "-dn"]
        .map(Into::into)
        .to_vec();
    match mode {
        // MPEG-TS needs Annex B H.264, which ffmpeg converts to on its own.
        HlsMode::Copy => args.extend(["-c", "copy"].map(Into::into)),
        HlsMode::Transcode => args.extend(
            [
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-crf",
                "23",
                "-maxrate",
                MAX_VIDEO_BITRATE,
                "-bufsize",
                VIDEO_BUFFER,
                "-vf",
                SCALE_TO_1080P,
                "-pix_fmt",
                "yuv420p",
                // A keyframe at every segment boundary, so segments start
                // where the playlist says.
                "-force_key_frames",
                "expr:gte(t,n_forced*6)",
                "-c:a",
                "aac",
                "-b:a",
                "160k",
                "-ac",
                "2",
            ]
            .map(Into::into),
        ),
    }
    args.extend(
        [
            "-f",
            "hls",
            "-hls_time",
            SEGMENT_SECONDS,
            "-hls_playlist_type",
            "vod",
            "-hls_segment_type",
            "mpegts",
            "-hls_segment_filename",
        ]
        .map(Into::into),
    );
    args.extend([SEGMENT_PATTERN, "-y", PLAYLIST].map(Into::into));
    args
}

/// The playlist must be complete (`#EXT-X-ENDLIST`), name only segments
/// that exist and are not empty, and its first segment must hold H.264.
async fn verify_playlist(tools: &crate::Tools, dir: &Path) -> Result<(), MediaError> {
    let text = tokio::fs::read_to_string(dir.join(PLAYLIST))
        .await
        .map_err(|_| MediaError::BadOutput)?;
    if !text.lines().any(|l| l.trim() == "#EXT-X-ENDLIST") {
        return Err(MediaError::BadOutput);
    }
    let segments: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    if segments.is_empty() || !segments.iter().all(|s| is_segment_name(s)) {
        return Err(MediaError::BadOutput);
    }
    for segment in &segments {
        let len = tokio::fs::metadata(dir.join(segment))
            .await
            .map_err(|_| MediaError::BadOutput)?
            .len();
        if len == 0 {
            return Err(MediaError::BadOutput);
        }
    }
    let first = crate::probe(tools, &Input::File(dir.join(segments[0])), VERIFY_TIMEOUT).await?;
    if first.video.as_ref().is_none_or(|v| v.codec != "h264") {
        return Err(MediaError::BadOutput);
    }
    Ok(())
}

/// The playlist with each segment URI turned into
/// `<set>/<segment>?token=<token>`. Players fetch segments from these URIs
/// and never carry the playlist URL's query over (contract "M6 接口"). `set`
/// is the cache key digest of this exact HLS set, so the segments keep
/// coming from it when another rendition, or a set for a changed source,
/// is published meanwhile. Lines that are not segments pass through.
pub fn sign_playlist(text: &str, set: &str, token: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for line in text.lines() {
        if is_segment_name(line.trim()) {
            out.push_str(set);
            out.push('/');
            out.push_str(line.trim());
            out.push_str("?token=");
            out.push_str(token);
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcodes_keep_under_the_bitrate_ceiling_and_copies_untouched() {
        let transcode = output_args(HlsMode::Transcode);
        let ceiling = ["-maxrate", MAX_VIDEO_BITRATE, "-bufsize", VIDEO_BUFFER];
        assert!(
            transcode
                .windows(4)
                .any(|w| w.iter().zip(ceiling).all(|(a, b)| a == b)),
            "{transcode:?}"
        );
        let copy = output_args(HlsMode::Copy);
        assert!(
            !copy.iter().any(|a| a == "-maxrate" || a == "-vf"),
            "{copy:?}"
        );
    }

    #[test]
    fn only_numbered_segments_are_segment_names() {
        assert!(is_segment_name("seg-00000.ts"));
        assert!(is_segment_name("seg-12345.ts"));
        for bad in [
            "seg-0000.ts",
            "seg-000000.ts",
            "seg-0000a.ts",
            "index.m3u8",
            "../seg-00000.ts",
            "seg-00000.ts/x",
        ] {
            assert!(!is_segment_name(bad), "{bad}");
        }
    }

    #[test]
    fn signing_adds_the_token_to_segments_only() {
        let playlist =
            "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXTINF:6.0,\nseg-00000.ts\n#EXT-X-ENDLIST\n";
        assert_eq!(
            sign_playlist(playlist, "f00d", "abc"),
            "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXTINF:6.0,\nf00d/seg-00000.ts?token=abc\n#EXT-X-ENDLIST\n"
        );
    }
}
