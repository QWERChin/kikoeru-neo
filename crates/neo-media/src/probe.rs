//! ffprobe: what a file contains, stored as JSON in `tracks.probe`.

use std::{process::Stdio, time::Duration};

use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
};

use crate::{Input, MediaError, Tools};

/// The parts of ffprobe's answer that playback decisions need.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Probe {
    /// ffprobe's `format_name`, such as `mp3` or `mov,mp4,m4a,3gp,3g2,mj2`.
    pub format: String,
    pub duration_ms: Option<i64>,
    pub audio: Option<AudioStream>,
    /// The first real video stream. Cover art embedded in audio files is
    /// not counted.
    pub video: Option<VideoStream>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioStream {
    pub codec: String,
    pub profile: Option<String>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoStream {
    pub codec: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// Runs ffprobe on `input`. The child is killed if the call is dropped or
/// the timeout passes.
///
/// # Errors
/// ffprobe could not start, failed, timed out, or printed something that
/// is not its JSON.
pub async fn probe(tools: &Tools, input: &Input, timeout: Duration) -> Result<Probe, MediaError> {
    let input_stdin = input.stdin()?;
    let mut command = Command::new(&tools.ffprobe);
    command
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            // Only the fields `Probe` uses. Tags are left out because a
            // file can carry megabytes of them.
            "-show_entries",
            "format=format_name,duration:stream=codec_type,codec_name,profile,sample_rate,\
             channels,width,height,duration:stream_disposition=attached_pic",
        ])
        .args(input.args())
        .stdin(input_stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let run = async {
        let mut child = command.spawn().map_err(|source| MediaError::Spawn {
            program: "ffprobe",
            source,
        })?;
        let mut stdout = child.stdout.take().expect("stdout is piped");
        let mut stderr = child.stderr.take().expect("stderr is piped");
        let (out, err, status) = tokio::join!(
            read_head(&mut stdout, MAX_PROBE_OUTPUT),
            read_tail(&mut stderr, STDERR_TAIL),
            child.wait()
        );
        let (out, overflowed) = out?;
        let err = err?;
        let status = status?;
        if overflowed {
            return Err(MediaError::Parse("ffprobe printed too much".into()));
        }
        if !status.success() {
            return Err(MediaError::Failed {
                program: "ffprobe",
                status: status.to_string(),
                stderr: tail(&err),
            });
        }
        parse(&out)
    };
    tokio::time::timeout(timeout, run)
        .await
        .map_err(|_| MediaError::Timeout("ffprobe"))?
}

/// ffprobe's JSON for the requested fields is a few KiB.
const MAX_PROBE_OUTPUT: usize = 1 << 20;
/// How much of a child's stderr is kept for error messages.
pub(crate) const STDERR_TAIL: usize = 4096;

/// Drains `reader`, keeping at most `limit` bytes from the start. The flag
/// says whether anything was dropped. The pipe is always read to the end so
/// the child never blocks on a full pipe.
pub(crate) async fn read_head<R: AsyncRead + Unpin>(
    reader: &mut R,
    limit: usize,
) -> std::io::Result<(Vec<u8>, bool)> {
    let mut kept = Vec::new();
    let mut overflowed = false;
    let mut chunk = [0u8; 8192];
    loop {
        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            return Ok((kept, overflowed));
        }
        let room = limit.saturating_sub(kept.len());
        kept.extend_from_slice(&chunk[..n.min(room)]);
        overflowed |= n > room;
    }
}

/// Drains `reader`, keeping only its last `limit` bytes.
pub(crate) async fn read_tail<R: AsyncRead + Unpin>(
    reader: &mut R,
    limit: usize,
) -> std::io::Result<Vec<u8>> {
    let mut kept = std::collections::VecDeque::with_capacity(limit);
    let mut chunk = [0u8; 8192];
    loop {
        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            return Ok(kept.into());
        }
        for &byte in &chunk[..n] {
            if kept.len() == limit {
                kept.pop_front();
            }
            kept.push_back(byte);
        }
    }
}

/// The last 2 KiB of a process's stderr, on one line, with anything that
/// looks like a URL replaced, because ffmpeg quotes its inputs and a URL
/// can carry a signature.
pub(crate) fn tail(stderr: &[u8]) -> String {
    let start = stderr.len().saturating_sub(2048);
    String::from_utf8_lossy(&stderr[start..])
        .split_whitespace()
        .map(|word| if word.contains("://") { "<url>" } else { word })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Deserialize)]
struct Raw {
    #[serde(default)]
    streams: Vec<RawStream>,
    format: Option<RawFormat>,
}

#[derive(Deserialize)]
struct RawFormat {
    #[serde(default)]
    format_name: String,
    duration: Option<String>,
}

#[derive(Deserialize)]
struct RawStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    profile: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u32>,
    width: Option<u32>,
    height: Option<u32>,
    duration: Option<String>,
    #[serde(default)]
    disposition: RawDisposition,
}

#[derive(Deserialize, Default)]
struct RawDisposition {
    #[serde(default)]
    attached_pic: u8,
}

fn parse(json: &[u8]) -> Result<Probe, MediaError> {
    let raw: Raw =
        serde_json::from_slice(json).map_err(|error| MediaError::Parse(error.to_string()))?;
    let format = raw
        .format
        .ok_or_else(|| MediaError::Parse("no format section".into()))?;
    let audio_raw = raw
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("audio"));
    let video_raw = raw
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("video") && s.disposition.attached_pic == 0);
    // Some containers only report the duration on the stream.
    let duration_ms = seconds_to_ms(format.duration.as_deref())
        .or_else(|| audio_raw.and_then(|s| seconds_to_ms(s.duration.as_deref())));
    Ok(Probe {
        format: format.format_name,
        duration_ms,
        audio: audio_raw.map(|s| AudioStream {
            codec: s.codec_name.clone().unwrap_or_default(),
            profile: s.profile.clone(),
            sample_rate: s.sample_rate.as_deref().and_then(|r| r.parse().ok()),
            channels: s.channels,
        }),
        video: video_raw.map(|s| VideoStream {
            codec: s.codec_name.clone().unwrap_or_default(),
            width: s.width,
            height: s.height,
        }),
    })
}

fn seconds_to_ms(text: Option<&str>) -> Option<i64> {
    let seconds: f64 = text?.parse().ok()?;
    (seconds.is_finite() && seconds >= 0.0).then(|| (seconds * 1000.0).round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_audio_and_ignores_cover_art() {
        let json = br#"{
            "streams": [
                {"codec_type": "audio", "codec_name": "mp3", "sample_rate": "44100", "channels": 2},
                {"codec_type": "video", "codec_name": "mjpeg", "width": 500, "height": 500,
                 "disposition": {"attached_pic": 1}}
            ],
            "format": {"format_name": "mp3", "duration": "1995.123456"}
        }"#;
        let probe = parse(json).unwrap();
        assert_eq!(probe.format, "mp3");
        assert_eq!(probe.duration_ms, Some(1_995_123));
        let audio = probe.audio.unwrap();
        assert_eq!(
            (audio.codec.as_str(), audio.sample_rate, audio.channels),
            ("mp3", Some(44100), Some(2))
        );
        assert_eq!(probe.video, None, "cover art is not video");
    }

    #[test]
    fn falls_back_to_the_stream_duration() {
        let json =
            br#"{"streams": [{"codec_type": "audio", "codec_name": "opus", "duration": "2.5"}],
            "format": {"format_name": "ogg"}}"#;
        assert_eq!(parse(json).unwrap().duration_ms, Some(2500));
    }

    #[test]
    fn rejects_output_without_a_format() {
        assert!(parse(b"{}").is_err());
        assert!(parse(b"not json").is_err());
    }

    #[tokio::test]
    async fn bounded_readers_drain_everything_but_keep_little() {
        let data = vec![7u8; 100_000];
        let (head, overflowed) = read_head(&mut data.as_slice(), 10).await.unwrap();
        assert_eq!((head.len(), overflowed), (10, true));
        let (all, overflowed) = read_head(&mut &data[..5], 10).await.unwrap();
        assert_eq!((all.len(), overflowed), (5, false));
        let mut numbered: Vec<u8> = (0..=255u8).cycle().take(1000).collect();
        numbered.push(42);
        let tail = read_tail(&mut numbered.as_slice(), 3).await.unwrap();
        assert_eq!(tail, [230, 231, 42]);
    }

    #[test]
    fn stderr_tail_is_bounded_and_single_line() {
        let long = format!("{}\nlast line\n", "x".repeat(5000));
        let tail = tail(long.as_bytes());
        assert!(tail.len() <= 2048);
        assert!(tail.ends_with("last line"));
        assert_eq!(
            super::tail(b"https://cdn.example/x?sig=1: 403"),
            "<url> 403"
        );
    }
}
