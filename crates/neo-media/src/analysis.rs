//! Loudness and waveform from one ffmpeg pass (book/src/media.md, "响度和
//! 波形"). One branch measures EBU R128 loudness on the original audio, the
//! other streams 8 kHz mono samples that are folded into peaks as they
//! arrive, so the PCM is never held in memory.

use std::{future::Future, process::Stdio, time::Duration};

use tokio::{io::AsyncReadExt, process::Command, sync::watch};
use tokio_util::sync::CancellationToken;

use crate::{
    CacheKey, CachedFile, Input, MediaError, TranscodeCache,
    probe::tail,
    transcode::{TempFile, temp_name, terminate},
};

/// The `params` part of a waveform's cache key.
pub const WAVEFORM_PARAMS: &str = "waveform-1000-v1";
/// Bytes in a waveform: one peak per thousandth of the track.
pub const WAVEFORM_BUCKETS: usize = 1000;
/// The sample rate the waveform branch is downmixed to.
const WAVEFORM_RATE: u64 = 8000;
/// Samples per fine peak: 10 ms at 8 kHz. An hour is 360 000 of them.
const FINE_SAMPLES: usize = 80;
/// ffmpeg reports -70 LUFS for silence and for audio entirely below the
/// gate, which is not a measurement.
const UNMEASURED_LUFS: f64 = -70.0;
/// ebur128's summary is short, but ffmpeg's other info lines come too.
const STDERR_KEEP: usize = 64 * 1024;

/// A track's measured loudness. `None` where the track is too quiet to
/// measure.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Loudness {
    pub integrated_lufs: Option<f64>,
    pub true_peak_dbtp: Option<f64>,
}

pub struct AnalysisJob<'a> {
    pub input: &'a Input,
    /// The waveform's key, built with [`WAVEFORM_PARAMS`].
    pub key: &'a CacheKey,
    /// From the source's probe, to turn samples read into progress.
    pub duration_ms: Option<i64>,
    pub timeout: Duration,
}

/// Measures loudness and writes the waveform under `job.key`. Like a
/// transcode, the waveform goes to `cache/tmp/` first and is renamed into
/// place only after `restat` confirms the source did not change. On
/// cancellation or timeout ffmpeg is stopped and nothing is published.
///
/// # Errors
/// Low disk space, ffmpeg failure, timeout, cancellation, output that is
/// not a loudness summary, or a source that changed meanwhile.
pub async fn analyze<F>(
    cache: &TranscodeCache,
    job: AnalysisJob<'_>,
    progress: &watch::Sender<f64>,
    cancel: &CancellationToken,
    restat: F,
) -> Result<(Loudness, CachedFile), MediaError>
where
    F: Future<Output = Result<(u64, Option<i64>), MediaError>>,
{
    let pin = cache.pin(job.key);
    cache.check_space()?;
    let deadline = tokio::time::Instant::now() + job.timeout;
    let (loudness, peaks) = run(cache.tools(), &job, progress, cancel, deadline).await?;
    let tmp = TempFile::new(cache.tmp_dir().join(temp_name(job.key, "wave")));
    tokio::fs::write(&tmp.0, fold(&peaks)).await?;
    let (size, mtime) = tokio::select! {
        result = restat => result?,
        () = cancel.cancelled() => return Err(MediaError::Cancelled),
        () = tokio::time::sleep_until(deadline) => return Err(MediaError::Timeout("ffmpeg")),
    };
    if size != job.key.size || mtime != job.key.mtime {
        return Err(MediaError::SourceChanged);
    }
    tokio::fs::rename(&tmp.0, cache.final_path(job.key)).await?;
    tmp.disarm();
    let _ = progress.send(1.0);
    if let Err(error) = cache.evict() {
        tracing::warn!(%error, "cache eviction failed");
    }
    Ok((loudness, cache.cached_file(job.key, pin)?))
}

/// Runs the two-branch ffmpeg and returns the loudness and the 10 ms
/// peaks. Cancelling stops ffmpeg the same way as a transcode.
async fn run(
    tools: &crate::Tools,
    job: &AnalysisJob<'_>,
    progress: &watch::Sender<f64>,
    cancel: &CancellationToken,
    deadline: tokio::time::Instant,
) -> Result<(Loudness, Vec<u16>), MediaError> {
    if cancel.is_cancelled() {
        return Err(MediaError::Cancelled);
    }
    let mut command = Command::new(&tools.ffmpeg);
    command
        // ebur128 writes its summary at the info level.
        .args(["-nostdin", "-hide_banner", "-nostats", "-loglevel", "info"])
        .args(job.input.args())
        .args([
            "-filter_complex",
            "[0:a:0]asplit=2[l][w];\
             [l]ebur128=peak=true:framelog=quiet,anullsink;\
             [w]aresample=8000,aformat=sample_fmts=s16:channel_layouts=mono[out]",
            "-map",
            "[out]",
            "-f",
            "s16le",
            "pipe:1",
        ])
        .stdin(job.input.stdin()?)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|source| MediaError::Spawn {
        program: "ffmpeg",
        source,
    })?;
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let stderr_task = tokio::spawn(async move {
        crate::probe::read_tail(&mut stderr, STDERR_KEEP)
            .await
            .unwrap_or_default()
    });
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let expected = job
        .duration_ms
        .filter(|d| *d > 0)
        .map(|d| d as f64 / 1000.0 * WAVEFORM_RATE as f64);
    let mut peaks = Peaks::default();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        tokio::select! {
            read = stdout.read(&mut buf) => match read? {
                0 => break,
                n => {
                    peaks.feed(&buf[..n]);
                    if let Some(total) = expected {
                        let done = peaks.samples as f64 / total;
                        let _ = progress.send(done.clamp(0.0, 0.99));
                    }
                }
            },
            () = cancel.cancelled() => {
                terminate(&mut child).await;
                return Err(MediaError::Cancelled);
            }
            () = tokio::time::sleep_until(deadline) => {
                terminate(&mut child).await;
                return Err(MediaError::Timeout("ffmpeg"));
            }
        }
    }
    // ffmpeg can close its output and still not exit, so the wait is
    // bounded by the same cancel and deadline.
    let status = tokio::select! {
        status = child.wait() => status?,
        () = cancel.cancelled() => {
            terminate(&mut child).await;
            return Err(MediaError::Cancelled);
        }
        () = tokio::time::sleep_until(deadline) => {
            terminate(&mut child).await;
            return Err(MediaError::Timeout("ffmpeg"));
        }
    };
    let stderr = stderr_task.await.unwrap_or_default();
    // A video without sound has nothing to measure. Treating it as silence
    // stores a result, so it is not analysed again on every play.
    if !status.success() && has_no_audio(&stderr) {
        let silent = Loudness {
            integrated_lufs: None,
            true_peak_dbtp: None,
        };
        return Ok((silent, Vec::new()));
    }
    if !status.success() {
        return Err(MediaError::Failed {
            program: "ffmpeg",
            status: status.to_string(),
            stderr: tail(&stderr),
        });
    }
    let loudness = parse_summary(&String::from_utf8_lossy(&stderr))
        .ok_or_else(|| MediaError::Parse("no ebur128 summary in ffmpeg's output".into()))?;
    Ok((loudness, peaks.finish()))
}

/// Whether ffmpeg failed because the input has no audio stream for
/// `[0:a:0]` to select.
fn has_no_audio(stderr: &[u8]) -> bool {
    String::from_utf8_lossy(stderr).contains("matches no streams")
}

/// The running fold of little-endian s16 samples into 10 ms peaks. A
/// sample split across two reads is carried over.
#[derive(Default)]
struct Peaks {
    fine: Vec<u16>,
    current: u16,
    in_bucket: usize,
    samples: u64,
    carry: Option<u8>,
}

impl Peaks {
    fn feed(&mut self, mut bytes: &[u8]) {
        if let Some(low) = self.carry.take() {
            let Some((&high, rest)) = bytes.split_first() else {
                self.carry = Some(low);
                return;
            };
            self.sample(i16::from_le_bytes([low, high]));
            bytes = rest;
        }
        let (pairs, rest) = bytes.as_chunks::<2>();
        for pair in pairs {
            self.sample(i16::from_le_bytes(*pair));
        }
        self.carry = rest.first().copied();
    }

    fn sample(&mut self, value: i16) {
        self.current = self.current.max(value.unsigned_abs());
        self.in_bucket += 1;
        self.samples += 1;
        if self.in_bucket == FINE_SAMPLES {
            self.fine.push(self.current);
            self.current = 0;
            self.in_bucket = 0;
        }
    }

    fn finish(mut self) -> Vec<u16> {
        if self.in_bucket > 0 {
            self.fine.push(self.current);
        }
        self.fine
    }
}

/// Folds the fine peaks into [`WAVEFORM_BUCKETS`] bytes, each the largest
/// peak of its share of the track scaled to 0..=255. A short track repeats
/// its peaks; no audio at all gives zeros.
pub(crate) fn fold(fine: &[u16]) -> Vec<u8> {
    let n = fine.len();
    (0..WAVEFORM_BUCKETS)
        .map(|i| {
            if n == 0 {
                return 0;
            }
            let start = i * n / WAVEFORM_BUCKETS;
            let end = ((i + 1) * n / WAVEFORM_BUCKETS).max(start + 1).min(n);
            let peak = fine[start..end].iter().copied().max().unwrap_or(0);
            scale(peak)
        })
        .collect()
}

/// Full scale (32768) is 255; anything audible is at least 1.
fn scale(peak: u16) -> u8 {
    let scaled = (u32::from(peak) * 255).div_ceil(32768);
    u8::try_from(scaled.min(255)).unwrap_or(255)
}

/// Reads `I:` and `Peak:` from ebur128's summary, rounded to 0.1.
pub(crate) fn parse_summary(stderr: &str) -> Option<Loudness> {
    let summary = &stderr[stderr.rfind("Summary:")?..];
    let value = |label: &str| -> Option<Option<f64>> {
        let line = summary
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with(label))?;
        let number = line[label.len()..].split_whitespace().next()?;
        Some(number.parse::<f64>().ok().filter(|v| v.is_finite()))
    };
    let integrated = value("I:")?.filter(|v| *v > UNMEASURED_LUFS);
    let peak = value("Peak:")?;
    let round = |v: f64| (v * 10.0).round() / 10.0;
    Some(Loudness {
        integrated_lufs: integrated.map(round),
        true_peak_dbtp: peak.map(round),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUMMARY: &str = "[Parsed_ebur128_2 @ 0x1] Summary:\n\n  Integrated loudness:\n    I:         -27.14 LUFS\n    Threshold: -37.1 LUFS\n\n  Loudness range:\n    LRA:         0.0 LU\n\n  True peak:\n    Peak:      -24.06 dBFS\n";

    #[test]
    fn the_summary_gives_loudness_and_peak() {
        assert_eq!(
            parse_summary(SUMMARY),
            Some(Loudness {
                integrated_lufs: Some(-27.1),
                true_peak_dbtp: Some(-24.1),
            })
        );
        let silent = SUMMARY
            .replace("-27.14 LUFS", "-70.0 LUFS")
            .replace("-24.06 dBFS", "-inf dBFS");
        assert_eq!(
            parse_summary(&silent),
            Some(Loudness {
                integrated_lufs: None,
                true_peak_dbtp: None,
            }),
            "ffmpeg's -70 LUFS sentinel is no measurement"
        );
        assert_eq!(parse_summary("no summary here"), None);
    }

    #[test]
    fn samples_split_across_reads_are_joined() {
        let samples: Vec<i16> = (0..200)
            .map(|i| if i == 150 { -32768 } else { 100 })
            .collect();
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut peaks = Peaks::default();
        for chunk in bytes.chunks(3) {
            peaks.feed(chunk);
        }
        assert_eq!(peaks.samples, 200);
        assert_eq!(peaks.finish(), [100, 32768, 100]);
    }

    #[test]
    fn peaks_fold_into_a_thousand_scaled_bytes() {
        let folded = fold(&[0, 16384, 32768]);
        assert_eq!(folded.len(), WAVEFORM_BUCKETS);
        assert_eq!(folded[0], 0);
        assert_eq!(folded[500], 128);
        assert_eq!(folded[999], 255);
        assert!(fold(&[]).iter().all(|b| *b == 0));
        assert_eq!(scale(1), 1, "anything audible shows");
        let long: Vec<u16> = (0..360_000).map(|i| (i % 7) as u16).collect();
        assert_eq!(fold(&long).len(), WAVEFORM_BUCKETS);
    }
}
