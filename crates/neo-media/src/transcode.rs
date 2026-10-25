//! The AAC fallback: one ffmpeg run per file, published into the cache
//! only after the output checks out.

use std::{
    future::Future,
    process::Stdio,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
    sync::watch,
};
use tokio_util::sync::CancellationToken;

use crate::{CacheKey, CachedFile, Input, MediaError, TranscodeCache, probe::tail};

/// The `params` part of the cache key for this transcode.
pub const AAC_PARAMS: &str = "aac-192k";
/// How long ffmpeg gets to exit after SIGTERM before it is killed.
#[cfg(unix)]
const TERMINATE_GRACE: Duration = Duration::from_secs(5);
const VERIFY_TIMEOUT: Duration = Duration::from_secs(30);

/// One transcode request.
pub struct Job<'a> {
    pub input: &'a Input,
    pub key: &'a CacheKey,
    /// From the source's probe, to turn ffmpeg's position into progress.
    pub duration_ms: Option<i64>,
    pub timeout: Duration,
}

/// Transcodes `job.input` to AAC and publishes it under `job.key`:
///
/// 1. writes into `cache/tmp/`,
/// 2. checks ffmpeg's exit status and that the output has a duration,
/// 3. calls `restat` and requires the source's size and mtime to still
///    match the key,
/// 4. renames the file into `cache/transcode/`.
///
/// Progress from 0 to 1 is sent on `progress`. On cancellation or timeout
/// ffmpeg gets SIGTERM, then SIGKILL after a grace period, and the
/// temporary file is removed.
///
/// # Errors
/// Low disk space, ffmpeg failure, timeout, cancellation, bad output, or a
/// source that changed meanwhile.
pub async fn transcode_aac<F>(
    cache: &TranscodeCache,
    job: Job<'_>,
    progress: &watch::Sender<f64>,
    cancel: &CancellationToken,
    restat: F,
) -> Result<CachedFile, MediaError>
where
    F: Future<Output = Result<(u64, Option<i64>), MediaError>>,
{
    // Through `lookup`, so an expired entry is not handed out and a hit
    // counts as an access.
    if let Some(cached) = cache.lookup(job.key)? {
        return Ok(cached);
    }
    let pin = cache.pin(job.key);
    cache.check_space()?;
    let deadline = tokio::time::Instant::now() + job.timeout;
    let tmp = TempFile::new(cache.tmp_dir().join(temp_name(job.key, "m4a")));
    let mut output: Vec<std::ffi::OsString> = [
        "-map",
        "0:a:0",
        "-vn",
        "-c:a",
        "aac",
        "-b:a",
        "192k",
        "-movflags",
        "+faststart",
        "-f",
        "mp4",
        "-y",
    ]
    .map(Into::into)
    .to_vec();
    output.push(tmp.0.clone().into());
    run_ffmpeg(
        cache.tools(),
        job.input,
        &output,
        None,
        job.duration_ms,
        progress,
        cancel,
        deadline,
    )
    .await?;

    // Cancellation and the deadline stay in force until the rename.
    let verify = async {
        let len = tokio::fs::metadata(&tmp.0).await?.len();
        let output = Input::File(tmp.0.clone());
        let verified = crate::probe(cache.tools(), &output, VERIFY_TIMEOUT).await?;
        let has_audio = verified.audio.as_ref().is_some_and(|a| a.codec == "aac");
        if len == 0 || !has_audio || verified.duration_ms.unwrap_or(0) <= 0 {
            return Err(MediaError::BadOutput);
        }
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
    tokio::fs::rename(&tmp.0, cache.final_path(job.key)).await?;
    tmp.disarm();
    let _ = progress.send(1.0);
    if let Err(error) = cache.evict() {
        tracing::warn!(%error, "cache eviction failed");
    }
    Ok(cache.cached_file(job.key, pin)?)
}

/// Runs ffmpeg on `input` with `output` (the options after the input and
/// the output path), reporting progress against `duration_ms` and stopping
/// it on cancellation or at the deadline. `cwd` is the directory relative
/// output names resolve in.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_ffmpeg(
    tools: &crate::Tools,
    input: &Input,
    output: &[std::ffi::OsString],
    cwd: Option<&std::path::Path>,
    duration_ms: Option<i64>,
    progress: &watch::Sender<f64>,
    cancel: &CancellationToken,
    deadline: tokio::time::Instant,
) -> Result<(), MediaError> {
    if cancel.is_cancelled() {
        return Err(MediaError::Cancelled);
    }
    let input_stdin = input.stdin()?;
    let mut command = Command::new(&tools.ffmpeg);
    command
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostats",
            "-progress",
            "pipe:1",
        ])
        // AVI packets can lack presentation timestamps, which the HLS muxer
        // refuses even when the streams are only copied. Generating missing
        // ones changes nothing for inputs that have them.
        .args(["-fflags", "+genpts"])
        .args(input.args())
        .args(output)
        .stdin(input_stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Only a last resort. The normal path terminates ffmpeg explicitly.
        .kill_on_drop(true);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let mut child = command.spawn().map_err(|source| MediaError::Spawn {
        program: "ffmpeg",
        source,
    })?;
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let stderr_task = tokio::spawn(async move {
        crate::probe::read_tail(&mut stderr, crate::probe::STDERR_TAIL)
            .await
            .unwrap_or_default()
    });
    let mut lines = BufReader::new(child.stdout.take().expect("stdout is piped")).lines();
    let deadline = tokio::time::sleep_until(deadline);
    tokio::pin!(deadline);
    let mut stdout_open = true;
    let status = loop {
        tokio::select! {
            status = child.wait() => break status?,
            line = lines.next_line(), if stdout_open => match line {
                Ok(Some(line)) => report(&line, duration_ms, progress),
                _ => stdout_open = false,
            },
            () = cancel.cancelled() => {
                terminate(&mut child).await;
                return Err(MediaError::Cancelled);
            }
            () = &mut deadline => {
                terminate(&mut child).await;
                return Err(MediaError::Timeout("ffmpeg"));
            }
        }
    };
    if !status.success() {
        let stderr = stderr_task.await.unwrap_or_default();
        return Err(MediaError::Failed {
            program: "ffmpeg",
            status: status.to_string(),
            stderr: tail(&stderr),
        });
    }
    Ok(())
}

fn report(line: &str, duration_ms: Option<i64>, progress: &watch::Sender<f64>) {
    let (Some(micros), Some(duration)) = (line.strip_prefix("out_time_us="), duration_ms) else {
        return;
    };
    if let Ok(micros) = micros.trim().parse::<i64>()
        && duration > 0
    {
        // Held below 1 until the file is published.
        let _ = progress.send((micros as f64 / 1000.0 / duration as f64).clamp(0.0, 0.99));
    }
}

/// SIGTERM, a grace period, then SIGKILL, always reaping the child.
pub(crate) async fn terminate(child: &mut Child) {
    #[cfg(unix)]
    if let Some(pid) = child
        .id()
        .and_then(|id| rustix::process::Pid::from_raw(id as i32))
    {
        let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
        if tokio::time::timeout(TERMINATE_GRACE, child.wait())
            .await
            .is_ok()
        {
            return;
        }
    }
    // Windows has no SIGTERM, so ffmpeg is stopped at once there.
    let _ = child.kill().await;
}

pub(crate) fn temp_name(key: &CacheKey, ext: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}.{}.{n}.{ext}", key.digest(), std::process::id())
}

/// Removes the temporary file on every early return.
pub(crate) struct TempFile(pub(crate) std::path::PathBuf, bool);

impl TempFile {
    pub(crate) fn new(path: std::path::PathBuf) -> Self {
        Self(path, true)
    }

    pub(crate) fn disarm(mut self) {
        self.1 = false;
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if self.1 {
            // A directory for HLS output, a file otherwise.
            let _ = std::fs::remove_dir_all(&self.0).or_else(|_| std::fs::remove_file(&self.0));
        }
    }
}
