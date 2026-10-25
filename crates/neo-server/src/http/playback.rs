//! Preparing playable media: `prepare`, `playable`, the transcode job and
//! on-demand ffprobe (book/src/api.md "播放", book/src/media.md).

use std::{io, sync::Arc, time::Duration};

use axum::{
    Json,
    body::Body,
    extract::State,
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
};
use futures_util::{FutureExt, StreamExt};
use neo_core::dto::{ErrorCode, PrepareRequest, PrepareResponse};
use neo_db::{
    media::{self, TrackFile},
    tasks::{self, TaskRow},
};
use neo_media::{
    AAC_PARAMS, CacheKey, CachedFile, HlsJob, HlsMode, Input, Job as TranscodeJob, MediaError,
    Playback, Probe, TranscodeCache, VideoPlayback, decide, decide_video, mime_type, probe,
    transcode_aac, transcode_hls,
};
use neo_storage::{LocalFile, RemoteFile, Source};
use serde_json::json;
use tokio::sync::Semaphore;

use super::{
    AppState, HttpError,
    extract::{ApiJson, ApiPath, CurrentUser, MediaUser},
    media::{name_root, named, root_path, serve_local, source_in, storage_error, storage_of},
};
use crate::tasks::{Job, JobError, Lane, SubmitError, TaskSpec};

/// `prepare` probes a track it has never probed before answering, but no
/// longer than this (book/src/api.md).
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const PLAYABLE_MIME: &str = "audio/mp4";

/// What a transcode task produces: the AAC fallback for audio, or HLS for
/// video (book/src/media.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    Aac,
    Hls(HlsMode),
}

impl Output {
    pub(crate) fn params(self) -> &'static str {
        match self {
            Self::Aac => AAC_PARAMS,
            Self::Hls(mode) => mode.params(),
        }
    }

    /// Stored in the task payload, so a restart rebuilds the same job.
    fn name(self) -> &'static str {
        match self {
            Self::Aac => "aac",
            Self::Hls(HlsMode::Copy) => "hls-copy",
            Self::Hls(HlsMode::Transcode) => "hls-transcode",
        }
    }

    fn from_name(name: Option<&str>) -> Option<Self> {
        match name {
            // Tasks queued before HLS existed carry no output.
            None | Some("aac") => Some(Self::Aac),
            Some("hls-copy") => Some(Self::Hls(HlsMode::Copy)),
            Some("hls-transcode") => Some(Self::Hls(HlsMode::Transcode)),
            Some(_) => None,
        }
    }

    /// The URL and MIME type a finished task points the player at.
    fn result(self, track_id: i64) -> (String, &'static str) {
        match self {
            Self::Aac => (format!("/api/tracks/{track_id}/playable"), PLAYABLE_MIME),
            Self::Hls(_) => (format!("/api/tracks/{track_id}/hls/index.m3u8"), HLS_MIME),
        }
    }
}

pub(crate) const HLS_MIME: &str = "application/vnd.apple.mpegurl";

fn cached_output(cache: &TranscodeCache, key: &CacheKey) -> io::Result<Option<CachedFile>> {
    // The playlist prefers a published transcode over a remux, so either
    // result can satisfy a request that only needs remuxing.
    if key.params == HlsMode::Copy.params() {
        let transcoded = CacheKey {
            params: HlsMode::Transcode.params(),
            ..key.clone()
        };
        if let Some(cached) = cache.lookup(&transcoded)? {
            return Ok(Some(cached));
        }
    }
    cache.lookup(key)
}

/// Media settings and shared resources, from `media.*` and `cache.*`.
pub struct Media {
    pub cache: TranscodeCache,
    /// `media.light_jobs`: ffprobe runs started by requests.
    pub probe_slots: Arc<Semaphore>,
    pub queue_limit: usize,
    pub job_timeout: Duration,
    /// How long an analysis of a remote file that plays directly waits
    /// before it copies the file, so the player buffers first.
    pub remote_analysis_delay: Duration,
    /// `media.video_transcode`. A remux and a transcode already in the
    /// cache are still served when it is off.
    pub video_transcode: bool,
}

/// `POST /api/tracks/{id}/prepare`.
pub async fn prepare(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiPath(id): ApiPath<i64>,
    ApiJson(body): ApiJson<PrepareRequest>,
) -> Result<Response, HttpError> {
    named(&state, id, async {
        let media = state.media()?;
        let track = media::track_file(state.db(), id)
            .await?
            .ok_or_else(HttpError::not_found)?;
        if track.kind != "audio" && track.kind != "video" {
            return Err(HttpError::invalid_request("只有音频和视频可以播放"));
        }
        if track.missing {
            return Err(missing());
        }
        // The live file decides, not the row: it may have changed since the
        // last scan, and then the stored probe and cache key would be stale.
        let opened = open_source(&state, &track).await?;
        let probed = match parse_probe(&track).filter(|_| matches_row(&track, &opened)) {
            Some(probe) => Some(probe),
            None => probe_track(&state, &track, &opened).await?,
        };
        // A file ffprobe cannot read is handed to the transcode, whose failure
        // then reaches the user as the task message.
        let unknown = Probe {
            format: String::new(),
            duration_ms: None,
            audio: None,
            video: None,
        };
        let probe_ref = probed.as_ref().unwrap_or(&unknown);
        let name = file_name(&track);
        let output = if track.kind == "video" {
            match decide_video(probe_ref, name, &body.can_play, body.force_fallback) {
                VideoPlayback::Direct => {
                    return Ok(ready(
                        format!("/api/tracks/{id}/stream"),
                        mime_type(name, probed.as_ref()),
                    ));
                }
                VideoPlayback::Remux => Output::Hls(HlsMode::Copy),
                VideoPlayback::Transcode => Output::Hls(HlsMode::Transcode),
            }
        } else if decide(probe_ref, &body.can_play, body.force_fallback) == Playback::Direct {
            super::analysis::queue_quietly(&state, &track, &opened, Some(user.id));
            return Ok(ready(
                format!("/api/tracks/{id}/stream"),
                mime_type(name, probed.as_ref()),
            ));
        } else {
            Output::Aac
        };
        let key = cache_key(&track, &opened, output.params())?;
        let published = cached_output(&media.cache, &key).map_err(HttpError::internal)?;
        // A remote file that still has to be transcoded is analysed from
        // the transcode's output, which `transcode_job` queues, so it is
        // downloaded once. Video is never analysed.
        if published.is_some() || matches!(opened, Opened::Local(_)) {
            super::analysis::queue_quietly(&state, &track, &opened, Some(user.id));
        }
        if published.is_some() {
            let (url, mime) = output.result(id);
            return Ok(ready(url, mime));
        }
        if output == Output::Hls(HlsMode::Transcode) && !media.video_transcode {
            return Err(video_transcode_off());
        }
        let dedupe_key = format!("transcode:{}", key.digest());
        // Joining a running transcode needs no disk space, so it is checked
        // before the space limit refuses new work.
        let existing = tasks::active_id(state.db(), &dedupe_key).await?;
        if existing.is_none() {
            // A remote source is copied whole before it is transcoded, so the
            // job would fail on a disk that cannot hold the copy.
            let copy = match &opened {
                Opened::Remote(remote) => remote.size.unwrap_or(0),
                Opened::Local(_) => 0,
            };
            if let Err(error) = media.cache.check_space_for(copy) {
                // The previous task may have published and ended while
                // active_id was waiting for the database.
                if matches!(error, MediaError::InsufficientStorage)
                    && let Some(_cached) =
                        cached_output(&media.cache, &key).map_err(HttpError::internal)?
                {
                    let (url, mime) = output.result(id);
                    return Ok(ready(url, mime));
                }
                return Err(media_error(error));
            }
        }
        let spec = TaskSpec {
            kind: "transcode",
            dedupe_key: Some(dedupe_key),
            payload: json!({
                "track_id": id,
                "key": key.digest(),
                "output": output.name(),
                "params": output.params(),
            }),
            lane: Lane::Heavy,
            timeout: Some(media.job_timeout),
            queue_limit: Some(media.queue_limit),
        };
        let submitted = state
            .scheduler()
            .submit(spec, transcode_job(state.clone(), id, key.digest(), output))
            .await
            .map_err(|error| match error {
                SubmitError::QueueFull => HttpError::new(
                    StatusCode::TOO_MANY_REQUESTS,
                    ErrorCode::QueueFull,
                    "转码队列已满，请稍后再试",
                ),
                SubmitError::Db(error) => HttpError::from(error),
            })?;
        state.scheduler().add_requester(submitted.id, user.id);
        let task_id = submitted.id.try_into().map_err(HttpError::internal)?;
        Ok((
            StatusCode::ACCEPTED,
            Json(PrepareResponse::Processing { task_id }),
        )
            .into_response())
    })
    .await
}

/// `GET/HEAD /api/tracks/{id}/playable`: the prepared transcode, or 409.
/// It never starts a transcode.
pub async fn playable(
    State(state): State<AppState>,
    _user: MediaUser,
    ApiPath(id): ApiPath<i64>,
    method: Method,
    headers: HeaderMap,
) -> Result<Response, HttpError> {
    named(&state, id, async {
        let media = state.media()?;
        let track = media::track_file(state.db(), id)
            .await?
            .ok_or_else(HttpError::not_found)?;
        if track.missing {
            return Err(missing());
        }
        let opened = open_source(&state, &track).await?;
        let key = cache_key(&track, &opened, AAC_PARAMS)?;
        let cached = media
            .cache
            .lookup(&key)
            .map_err(HttpError::internal)?
            .ok_or_else(|| {
                HttpError::new(
                    StatusCode::CONFLICT,
                    ErrorCode::MediaNotReady,
                    "这个文件还在转码，请稍后再试",
                )
            })?;
        let file = std::fs::File::open(&cached.path).map_err(HttpError::internal)?;
        let published = file
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|d| i64::try_from(d.as_millis()).ok());
        let local = LocalFile {
            file: Arc::new(file),
            path: cached.path.clone(),
            size: cached.len,
            mtime_ms: published,
        };
        let response = serve_local(&local, PLAYABLE_MIME, &method, &headers, None).await?;
        // The body holds the pin, so eviction leaves the file alone until the
        // last byte is sent or the client goes away.
        let (parts, body) = response.into_parts();
        let body = body.into_data_stream().map(move |chunk| {
            let _pin = &cached;
            chunk
        });
        Ok(Response::from_parts(parts, Body::from_stream(body)))
    })
    .await
}

fn ready(url: String, mime_type: &str) -> Response {
    let body = PrepareResponse::Ready {
        url,
        mime_type: mime_type.to_owned(),
    };
    (StatusCode::OK, Json(body)).into_response()
}

fn video_transcode_off() -> HttpError {
    HttpError::new(
        StatusCode::FORBIDDEN,
        ErrorCode::Forbidden,
        "服务器关闭了视频转码，这个视频要转码才能播放",
    )
}

pub(super) fn missing() -> HttpError {
    HttpError::new(StatusCode::GONE, ErrorCode::Missing, "这个文件已经找不到了")
}

fn file_name(track: &TrackFile) -> &str {
    track.display_path.rsplit('/').next().unwrap_or_default()
}

pub(super) fn parse_probe(track: &TrackFile) -> Option<Probe> {
    track
        .probe
        .as_deref()
        .and_then(|p| serde_json::from_str(p).ok())
}

/// A track's file as it is now, local or remote.
pub(crate) enum Opened {
    Local(LocalFile),
    Remote(RemoteFile),
}

impl Opened {
    pub(super) fn size(&self) -> Option<u64> {
        match self {
            Self::Local(local) => Some(local.size),
            Self::Remote(remote) => remote.size,
        }
    }

    pub(super) fn mtime_ms(&self) -> Option<i64> {
        match self {
            Self::Local(local) => local.mtime_ms,
            Self::Remote(remote) => remote.mtime_ms,
        }
    }
}

pub(crate) async fn open_source(state: &AppState, track: &TrackFile) -> Result<Opened, HttpError> {
    // The registry and source version change together under this lock.
    // Keep the selected backend after releasing it, including during I/O.
    let storage = {
        let _changes = state.lock_root_changes().await;
        if media::source_version(state.db(), &track.root_id).await? != Some(track.source_version) {
            return Err(unavailable("the library location changed"));
        }
        storage_of(state, &track.root_id)?
    };
    Ok(match source_in(&*storage, track).await? {
        Source::LocalFile(local) => Opened::Local(local),
        Source::Remote(remote) => Opened::Remote(remote),
    })
}

/// The cache key from the file as it is now.
pub(crate) fn cache_key(
    track: &TrackFile,
    opened: &Opened,
    params: &'static str,
) -> Result<CacheKey, HttpError> {
    Ok(CacheKey {
        root_id: track.root_id.clone(),
        source_version: track.source_version,
        path: root_path(track)?.into_bytes(),
        size: opened.size().unwrap_or_default(),
        mtime: opened.mtime_ms(),
        params,
    })
}

/// Whether the row still describes the file as it is now.
fn matches_row(track: &TrackFile, opened: &Opened) -> bool {
    track.size == opened.size().and_then(|s| i64::try_from(s).ok())
        && track.mtime == opened.mtime_ms()
}

/// How much of a remote file is fetched to probe it. Headers of the
/// library's formats fit easily, and 115 counts every byte.
const REMOTE_PROBE_BYTES: u64 = 2 << 20;

/// How much of the end of a remote file is fetched when the start was not
/// enough. MP4 files often keep their index (`moov`) at the end, and the
/// index of a few hours of AAC is a few MiB.
const REMOTE_TAIL_BYTES: u64 = 4 << 20;

/// How often a remote copy rechecks the free space while it writes.
const SPACE_CHECK_BYTES: u64 = 32 << 20;

/// Something ffmpeg can read. A remote file is first copied to `cache/tmp/`,
/// because ffmpeg must not follow provider URLs itself (book/src/media.md).
/// The copy is removed when this is dropped.
pub(super) struct Readable {
    pub(super) input: Input,
    /// Whether only parts of a remote file were copied, so durations
    /// ffprobe reports are estimates.
    partial: bool,
    /// For a partial copy of known size: the file is sparse at full length,
    /// and its tail from this offset can still be filled in.
    tail_from: Option<u64>,
    _temp: Option<TempPath>,
}

struct TempPath(std::path::PathBuf);

impl Drop for TempPath {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// ffmpeg reads a local file through the open handle, as `/dev/fd/0`.
/// Windows has no such path, so there it gets the file's path, which the
/// storage backend checked against the root when it opened the handle.
#[cfg(unix)]
fn local_input(local: &LocalFile) -> Input {
    Input::Handle(local.file.clone())
}

#[cfg(not(unix))]
fn local_input(local: &LocalFile) -> Input {
    Input::File(local.path.clone())
}

/// `limit` copies only the first bytes of a remote file. A local file is
/// read in place.
pub(super) async fn readable(
    media: &Media,
    track: &TrackFile,
    opened: &Opened,
    limit: Option<u64>,
) -> Result<Readable, HttpError> {
    let remote = match opened {
        Opened::Local(local) => {
            return Ok(Readable {
                input: local_input(local),
                partial: false,
                tail_from: None,
                _temp: None,
            });
        }
        Opened::Remote(remote) => remote,
    };
    let partial = match (limit, remote.size) {
        (Some(limit), Some(size)) => size > limit,
        (Some(_), None) => true,
        (None, _) => false,
    };
    let mut headers = HeaderMap::new();
    if let (true, Some(limit)) = (partial, limit) {
        let range = format!("bytes=0-{}", limit - 1);
        headers.insert(
            axum::http::header::RANGE,
            range.parse().map_err(HttpError::internal)?,
        );
    }
    let upstream = remote.opener.open(&headers).await.map_err(storage_error)?;
    if !matches!(upstream.status.as_u16(), 200 | 206) {
        return Err(upstream_refused());
    }
    if !partial {
        media
            .cache
            .check_space_for(remote.size.unwrap_or(0))
            .map_err(media_error)?;
    }
    // Created without an await, so a cancel cannot fall between creating
    // the file and `TempPath` taking ownership of its removal.
    let temp = TempPath(media.cache.temp_path(&format!("track-{}", track.id)));
    let mut file =
        tokio::fs::File::from_std(std::fs::File::create(&temp.0).map_err(HttpError::internal)?);
    let written = if partial {
        // An upstream that ignores Range still stops at the limit.
        copy_body(
            &mut file,
            upstream.body,
            limit.unwrap_or(u64::MAX),
            true,
            &media.cache,
        )
        .await?
    } else {
        copy_body(
            &mut file,
            upstream.body,
            remote.size.unwrap_or(u64::MAX),
            false,
            &media.cache,
        )
        .await?
    };
    if !partial && remote.size.is_some_and(|size| size != written) {
        return Err(unavailable("the remote file ended early"));
    }
    // A sparse file of full length lets ffprobe seek to the end, where the
    // tail can be filled in later.
    let tail_from = match (partial, remote.size) {
        // A size the filesystem cannot hold only costs the tail retry.
        (true, Some(size)) if file.set_len(size).await.is_ok() => {
            Some(size.saturating_sub(REMOTE_TAIL_BYTES).max(written))
        }
        _ => None,
    };
    Ok(Readable {
        input: Input::File(temp.0.clone()),
        partial,
        tail_from,
        _temp: Some(temp),
    })
}

impl Readable {
    /// Copies the end of the remote file into the sparse copy. Returns
    /// `false` when there is no tail to add or the upstream will not serve
    /// just the end.
    async fn add_tail(
        &mut self,
        remote: &RemoteFile,
        cache: &TranscodeCache,
    ) -> Result<bool, HttpError> {
        use tokio::io::AsyncSeekExt;
        let (Some(from), Some(size), Some(temp)) = (self.tail_from, remote.size, &self._temp)
        else {
            return Ok(false);
        };
        if from >= size {
            return Ok(false);
        }
        self.tail_from = None;
        let mut headers = HeaderMap::new();
        let range = format!("bytes={from}-{}", size - 1);
        headers.insert(
            axum::http::header::RANGE,
            range.parse().map_err(HttpError::internal)?,
        );
        let upstream = remote.opener.open(&headers).await.map_err(storage_error)?;
        // A whole-file answer would have to be read to its end, which is
        // the transfer the partial probe exists to avoid.
        let starts_at_tail = upstream
            .headers
            .get(axum::http::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with(&format!("bytes {from}-")));
        if upstream.status != StatusCode::PARTIAL_CONTENT || !starts_at_tail {
            return Ok(false);
        }
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .open(&temp.0)
            .await
            .map_err(HttpError::internal)?;
        file.seek(std::io::SeekFrom::Start(from))
            .await
            .map_err(HttpError::internal)?;
        let written = copy_body(&mut file, upstream.body, size - from, false, cache).await?;
        if written != size - from {
            return Err(unavailable("the remote file ended early"));
        }
        Ok(true)
    }
}

/// Copies `body` into `file` up to `cap` bytes. At the cap, a partial copy
/// stops, and a full copy fails if any byte follows, because a file longer than its listing
/// has changed and a truncated copy would be published as complete.
///
/// The free space is checked again every [`SPACE_CHECK_BYTES`], because a
/// size checked up front can be wrong or unknown, and other writers share
/// the disk with the database.
async fn copy_body(
    file: &mut tokio::fs::File,
    mut body: futures_util::stream::BoxStream<'static, std::io::Result<axum::body::Bytes>>,
    cap: u64,
    partial: bool,
    cache: &TranscodeCache,
) -> Result<u64, HttpError> {
    use tokio::io::AsyncWriteExt;
    let mut written = 0u64;
    let mut next_check = SPACE_CHECK_BYTES;
    while let Some(chunk) = body.next().await {
        if written >= next_check {
            cache.check_space().map_err(media_error)?;
            next_check = written + SPACE_CHECK_BYTES;
        }
        let chunk = chunk.map_err(|e| unavailable(&e.to_string()))?;
        let room = cap - written;
        if chunk.len() as u64 > room && !partial {
            return Err(unavailable("the remote file is longer than its listing"));
        }
        let take = usize::try_from(room).unwrap_or(usize::MAX).min(chunk.len());
        file.write_all(&chunk[..take])
            .await
            .map_err(HttpError::internal)?;
        written += take as u64;
        // A full copy reads on to the end, so a byte past the cap in a
        // later chunk still fails it.
        if partial && written >= cap {
            break;
        }
    }
    file.flush().await.map_err(HttpError::internal)?;
    Ok(written)
}

fn unavailable(cause: &str) -> HttpError {
    storage_error(neo_storage::StorageError::Unavailable(cause.to_owned()))
}

async fn within<T>(
    deadline: tokio::time::Instant,
    work: impl Future<Output = Result<T, HttpError>>,
) -> Result<T, HttpError> {
    tokio::time::timeout_at(deadline, work)
        .await
        .unwrap_or_else(|_| Err(unavailable("the remote file took too long to read")))
}

fn left(deadline: tokio::time::Instant) -> Duration {
    deadline.saturating_duration_since(tokio::time::Instant::now())
}

fn upstream_refused() -> HttpError {
    HttpError::new(
        StatusCode::BAD_GATEWAY,
        ErrorCode::UpstreamError,
        "媒体库返回了错误",
    )
}

pub(super) fn media_error(error: MediaError) -> HttpError {
    match error {
        MediaError::InsufficientStorage => HttpError::new(
            StatusCode::INSUFFICIENT_STORAGE,
            ErrorCode::InsufficientStorage,
            "服务器磁盘空间不足",
        ),
        other => HttpError::internal(other),
    }
}

/// A failed job step. The task message, which users see, names the step.
/// The database or file system error behind it goes to the log, or nobody
/// could ever tell why the step failed.
pub(super) fn step_failed(
    task: i64,
    track: i64,
    step: &str,
    cause: impl std::fmt::Display,
) -> JobError {
    tracing::warn!(task, track, step, %cause, "task step failed");
    JobError(step.to_owned())
}

/// Like `step_failed` for the free-space check, whose usual failure is
/// simply a full disk.
pub(super) fn space_failed(task: i64, track: i64, error: MediaError) -> JobError {
    match error {
        MediaError::InsufficientStorage => JobError("服务器磁盘空间不足".to_owned()),
        other => step_failed(task, track, "检查磁盘空间失败", other),
    }
}

/// Probes a track now and stores the result. Returns `None` when ffprobe
/// cannot read the file, so the caller can fall back to transcoding.
///
/// The result is stored only while the row still matches the file, so a
/// probe of a newer version never sits next to the old size and time.
///
/// A remote file is probed from its first bytes. Its duration is then an
/// estimate and is not stored.
async fn probe_track(
    state: &AppState,
    track: &TrackFile,
    opened: &Opened,
) -> Result<Option<Probe>, HttpError> {
    let media = state.media()?;
    let _slot = media
        .probe_slots
        .acquire()
        .await
        .map_err(HttpError::internal)?;
    // The copy and ffprobe share one deadline, so a slow upstream cannot
    // hold a probe slot that local tracks are waiting for.
    let deadline = tokio::time::Instant::now() + PROBE_TIMEOUT;
    let mut readable = within(
        deadline,
        readable(media, track, opened, Some(REMOTE_PROBE_BYTES)),
    )
    .await?;
    let mut result = probe(media.cache.tools(), &readable.input, left(deadline)).await;
    if let (Err(_), Opened::Remote(remote)) = (&result, opened)
        && within(deadline, readable.add_tail(remote, &media.cache)).await?
    {
        result = probe(media.cache.tools(), &readable.input, left(deadline)).await;
    }
    let mut probed = match result {
        Ok(probed) => probed,
        Err(error) => {
            tracing::warn!(track = track.id, %error, "ffprobe failed");
            return Ok(None);
        }
    };
    if readable.partial {
        probed.duration_ms = None;
    }
    if matches_row(track, opened) {
        let json = serde_json::to_string(&probed).map_err(HttpError::internal)?;
        media::save_probe(state.db(), track, &json, probed.duration_ms).await?;
    }
    Ok(Some(probed))
}

/// The scheduler job that transcodes track `track_id` into the cache.
///
/// `expected_key` is the cache digest the task was deduplicated on. If the
/// source changed since, the job fails instead of transcoding under a key
/// that another task may own.
pub fn transcode_job(state: AppState, track_id: i64, expected_key: String, output: Output) -> Job {
    Box::new(move |ctx| {
        async move {
            let fail = |message: &str| JobError(message.to_owned());
            let task = ctx.task_id;
            let media = state.media().map_err(|_| fail("服务没有配置转码缓存"))?;
            let track = media::track_file(state.db(), track_id)
                .await
                .map_err(|e| step_failed(task, track_id, "读取轨道失败", e))?
                .ok_or_else(|| fail("轨道已经不存在"))?;
            if track.missing {
                return Err(fail("这个文件已经找不到了"));
            }
            let opened = match open_source(&state, &track).await {
                Ok(opened) => opened,
                Err(e) => return Err(JobError(name_root(&state, track_id, e).await.message)),
            };
            let key =
                cache_key(&track, &opened, output.params()).map_err(|_| fail("轨道路径无效"))?;
            if key.digest() != expected_key {
                return Err(fail("源文件已经变了，请重新播放"));
            }
            // A task can finish between prepare's lookup and enqueue.
            // Hold its published output before budgeting or copying a source.
            let _published = if let Some(cached) = cached_output(&media.cache, &key)
                .map_err(|e| step_failed(task, track_id, "读取转码缓存失败", e))?
            {
                cached
            } else {
                // A remote source is also checked against its own size before
                // it is copied (see `readable`).
                media
                    .cache
                    .check_space()
                    .map_err(|e| space_failed(task, track_id, e))?;
                let source = tokio::select! {
                    source = readable(media, &track, &opened, None) => match source {
                        Ok(source) => source,
                        Err(e) => return Err(JobError(name_root(&state, track_id, e).await.message)),
                    },
                    () = ctx.cancel.cancelled() => return Err(fail("已取消")),
                };
                let duration_ms = parse_probe(&track)
                    .and_then(|p| p.duration_ms)
                    .or(track.duration_ms);
                let restat = restat(&state, &track);
                let published = match output {
                    Output::Aac => {
                        let job = TranscodeJob {
                            input: &source.input,
                            key: &key,
                            duration_ms,
                            timeout: media.job_timeout,
                        };
                        transcode_aac(&media.cache, job, ctx.progress(), &ctx.cancel, restat).await
                    }
                    Output::Hls(mode) => {
                        let job = HlsJob {
                            input: &source.input,
                            key: &key,
                            mode,
                            duration_ms,
                            timeout: media.job_timeout,
                        };
                        transcode_hls(&media.cache, job, ctx.progress(), &ctx.cancel, restat).await
                    }
                };
                published.map_err(|error| match error {
                    MediaError::InsufficientStorage => fail("服务器磁盘空间不足"),
                    MediaError::SourceChanged => fail("源文件在转码期间变了，请重试"),
                    MediaError::Cancelled => fail("已取消"),
                    other => {
                        tracing::warn!(track = track_id, error = %other, "transcode failed");
                        fail("转码失败")
                    }
                })?
            };
            // The analysis reads this output instead of downloading a
            // remote file again.
            super::analysis::queue_quietly(&state, &track, &opened, None);
            let (url, mime_type) = output.result(track_id);
            Ok(Some(json!({ "url": url, "mime_type": mime_type })))
        }
        .boxed()
    })
}

/// The source's size and mtime as they are now, for a job to check before
/// it publishes. Any failure counts as a changed source.
pub(super) async fn restat(
    state: &AppState,
    track: &TrackFile,
) -> Result<(u64, Option<i64>), MediaError> {
    if media::source_version(state.db(), &track.root_id)
        .await
        .map_err(|_| MediaError::SourceChanged)?
        != Some(track.source_version)
    {
        return Err(MediaError::SourceChanged);
    }
    let storage = state
        .storage(&track.root_id)
        .ok_or(MediaError::SourceChanged)?;
    let path = root_path(track).map_err(|_| MediaError::SourceChanged)?;
    let entry = storage.stat(&path).await.map_err(|error| {
        tracing::warn!(track = track.id, error = %storage_error(error).message, "re-stat failed");
        MediaError::SourceChanged
    })?;
    Ok((entry.size.unwrap_or_default(), entry.mtime_ms))
}

/// How long a background probe may take, ffprobe and the database write.
const PROBE_JOB_TIMEOUT: Duration = Duration::from_secs(60);

/// The task for probing one track in the background, for example after a
/// scan: light lane, one per track at a time.
pub fn probe_spec(track_id: i64) -> TaskSpec {
    TaskSpec {
        kind: "probe",
        dedupe_key: Some(format!("probe:{track_id}")),
        payload: json!({ "track_id": track_id }),
        lane: Lane::Light,
        timeout: Some(PROBE_JOB_TIMEOUT),
        queue_limit: None,
    }
}

/// The scheduler job that probes track `track_id` and stores the result.
/// Submit it with [`probe_spec`]. A track that is missing, already probed
/// or unreadable by ffprobe ends the task without an error, because a
/// later `prepare` still decides playback.
pub fn probe_job(state: AppState, track_id: i64) -> Job {
    Box::new(move |ctx| {
        async move {
            let track = media::track_file(state.db(), track_id)
                .await
                .map_err(|e| step_failed(ctx.task_id, track_id, "读取轨道失败", e))?;
            let Some(track) = track else { return Ok(None) };
            if track.missing || track.probe.is_some() {
                return Ok(None);
            }
            // Dropping the probe on cancel kills ffprobe and skips the write.
            let opened = open_source(&state, &track)
                .await
                .map_err(|e| JobError(e.message))?;
            tokio::select! {
                result = probe_track(&state, &track, &opened) => {
                    result.map_err(|e| JobError(e.message))?;
                }
                () = ctx.cancel.cancelled() => return Err(JobError("已取消".to_owned())),
            }
            Ok(None)
        }
        .boxed()
    })
}

/// Rebuilds a queued task left by the previous process (see
/// `Scheduler::recover`). Transcodes, probes and analyses can resume.
pub fn rebuild_task(state: &AppState, row: &TaskRow) -> Option<(Lane, Option<Duration>, Job)> {
    let payload: serde_json::Value = serde_json::from_str(&row.payload).ok()?;
    let track_id = payload.get("track_id")?.as_i64()?;
    match row.kind.as_str() {
        "transcode" => {
            let media = state.media().ok()?;
            let output = Output::from_name(payload.get("output").and_then(|o| o.as_str()))?;
            // Queued before video transcoding was turned off: it ends as
            // interrupted instead of running.
            if output == Output::Hls(HlsMode::Transcode) && !media.video_transcode {
                return None;
            }
            // Queued under other processing settings, such as a video
            // transcode from before its 1080p limit: the key no longer
            // matches, so it would fail as if the source had changed. It ends
            // as interrupted, and the player prepares it again. Tasks older
            // than this field kept their settings, except that transcode.
            let params = payload.get("params").and_then(|p| p.as_str());
            let stale = match params {
                Some(params) => params != output.params(),
                None => output == Output::Hls(HlsMode::Transcode),
            };
            if stale {
                return None;
            }
            Some((
                Lane::Heavy,
                Some(media.job_timeout),
                transcode_job(
                    state.clone(),
                    track_id,
                    payload.get("key")?.as_str()?.to_owned(),
                    output,
                ),
            ))
        }
        "probe" => Some((
            Lane::Light,
            Some(PROBE_JOB_TIMEOUT),
            probe_job(state.clone(), track_id),
        )),
        "analysis" => super::analysis::rebuild(state, track_id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logging::captured;

    #[test]
    fn failed_step_logs_its_cause_and_shows_only_the_step() {
        let log = captured::start();
        let JobError(message) = step_failed(7, 42, "保存响度失败", "database is locked");
        assert_eq!(message, "保存响度失败");
        let lines = log.lines();
        assert_eq!(lines.len(), 1, "{lines:?}");
        for part in [
            "WARN",
            "task=7",
            "track=42",
            "step=\"保存响度失败\"",
            "cause=database is locked",
        ] {
            assert!(lines[0].contains(part), "{part} in {lines:?}");
        }
    }

    #[test]
    fn full_disk_is_not_logged_but_a_failed_check_is() {
        let log = captured::start();
        let JobError(message) = space_failed(7, 42, MediaError::InsufficientStorage);
        assert_eq!(message, "服务器磁盘空间不足");
        assert!(log.lines().is_empty(), "{:?}", log.lines());
        let error = MediaError::Io(std::io::Error::other("statvfs failed"));
        let JobError(message) = space_failed(7, 42, error);
        assert_eq!(message, "检查磁盘空间失败");
        assert!(
            log.lines()[0].contains("statvfs failed"),
            "{:?}",
            log.lines()
        );
    }
}
