//! Loudness and waveform: the `analysis` task, the two endpoints that read
//! its results and the user settings that turn normalisation on
//! (book/src/contract.md "响度和波形", book/src/media.md).

use std::time::Duration;

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use futures_util::FutureExt;
use neo_core::dto::{Decibels, ErrorCode, Loudness, PrepareResponse, UserSettings};
use neo_db::{
    analysis::{self, StoredLoudness},
    media::{self, TrackFile},
    now_ms, settings, tasks,
};
use neo_media::{AAC_PARAMS, AnalysisJob, CachedFile, Input, MediaError, WAVEFORM_PARAMS, analyze};
use serde_json::json;

use super::{
    AppState, HttpError,
    extract::{ApiJson, ApiPath, CurrentUser},
    media::{name_root, named},
    playback::{
        Opened, Readable, cache_key, media_error, missing, open_source, parse_probe, readable,
        restat, space_failed, step_failed,
    },
    web::etag_matches,
};
use crate::tasks::{Job, JobError, Lane, SubmitError, TaskSpec};

const KIND: &str = "analysis";
/// The only kind of track that is analysed.
const AUDIO: &str = "audio";
const WAVEFORM_CACHE: &str = "private, max-age=604800";
/// `PUT /api/me/settings` accepts target levels in this range.
const TARGET_LUFS: std::ops::RangeInclusive<f64> = -30.0..=-10.0;

/// `GET /api/tracks/{id}/loudness`.
pub async fn loudness(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiPath(id): ApiPath<i64>,
) -> Result<Response, HttpError> {
    named(&state, id, async {
        let (track, opened) = open_playable(&state, id).await?;
        match stored(&state, id, &opened).await? {
            Some(stored) => Ok(Json(to_dto(stored)).into_response()),
            None => queue(&state, &track, &opened, user.id).await,
        }
    })
    .await
}

/// `GET /api/tracks/{id}/waveform`. The ETag is the cache key, which
/// changes with the file, and the time the waveform was written.
pub async fn waveform(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiPath(id): ApiPath<i64>,
    headers: HeaderMap,
) -> Result<Response, HttpError> {
    named(&state, id, async {
        let (track, opened) = open_playable(&state, id).await?;
        let media = state.media()?;
        let key = cache_key(&track, &opened, WAVEFORM_PARAMS)?;
        let Some(cached) = media.cache.lookup(&key).map_err(HttpError::internal)? else {
            return queue(&state, &track, &opened, user.id).await;
        };
        // The key alone stays the same when a file without an mtime is
        // regenerated, so the time the waveform was written is part of it.
        let written = tokio::fs::metadata(&cached.path)
            .await
            .and_then(|m| m.modified())
            .map_err(HttpError::internal)?
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let etag = format!("\"{}-{written:x}\"", key.digest());
        let mut response = if etag_matches(&headers, &etag) {
            StatusCode::NOT_MODIFIED.into_response()
        } else {
            let body = tokio::fs::read(&cached.path)
                .await
                .map_err(HttpError::internal)?;
            (
                [(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/octet-stream"),
                )],
                body,
            )
                .into_response()
        };
        let h = response.headers_mut();
        h.insert(
            header::ETAG,
            HeaderValue::from_str(&etag).map_err(HttpError::internal)?,
        );
        h.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static(WAVEFORM_CACHE),
        );
        Ok(response)
    })
    .await
}

/// Queues an analysis for an audio track that is about to play, unless it
/// is measured already. It runs on its own task, so the caller neither
/// waits for it nor fails with it.
pub(super) fn queue_quietly(
    state: &AppState,
    track: &TrackFile,
    opened: &Opened,
    user_id: Option<i64>,
) {
    if track.kind != AUDIO {
        return;
    }
    let (state, track_id) = (state.clone(), track.id);
    let (size, mtime) = (size_of(opened), opened.mtime_ms());
    tokio::spawn(async move {
        match analysis::loudness(state.db(), track_id, size, mtime, now_ms()).await {
            Ok(None) => {}
            Ok(Some(_)) => return,
            Err(error) => {
                tracing::warn!(track = track_id, %error, "cannot check for a stored loudness");
                return;
            }
        }
        let Ok(media) = state.media() else { return };
        match state
            .scheduler()
            .submit(spec(track_id, media), job(state.clone(), track_id))
            .await
        {
            Ok(submitted) => {
                if let Some(user_id) = user_id {
                    state.scheduler().add_requester(submitted.id, user_id);
                }
            }
            Err(error) => tracing::debug!(track = track_id, ?error, "analysis not queued"),
        }
    });
}

/// The track if it is a present audio file, opened as it is now. Video
/// is not analysed: the player draws no waveform for it, and a remote
/// video would be copied whole just to measure its sound.
async fn open_playable(state: &AppState, id: i64) -> Result<(TrackFile, Opened), HttpError> {
    state.media()?;
    let track = media::track_file(state.db(), id)
        .await?
        .ok_or_else(HttpError::not_found)?;
    if track.kind != AUDIO {
        return Err(HttpError::new(
            StatusCode::NOT_FOUND,
            ErrorCode::NotApplicable,
            "只有音频有响度和波形",
        ));
    }
    if track.missing {
        return Err(missing());
    }
    let opened = open_source(state, &track).await?;
    Ok((track, opened))
}

fn size_of(opened: &Opened) -> Option<i64> {
    opened.size().and_then(|s| i64::try_from(s).ok())
}

async fn stored(
    state: &AppState,
    id: i64,
    opened: &Opened,
) -> Result<Option<StoredLoudness>, HttpError> {
    Ok(analysis::loudness(state.db(), id, size_of(opened), opened.mtime_ms(), now_ms()).await?)
}

fn to_dto(stored: StoredLoudness) -> Loudness {
    Loudness {
        integrated_lufs: stored.integrated_lufs.map(Decibels),
        true_peak_dbtp: stored.true_peak_dbtp.map(Decibels),
    }
}

/// Queues the track's analysis and answers 202 with the task.
///
/// While a remote file is being transcoded, the answer is the transcode
/// task instead: it queues the analysis of its output when it is done, so
/// the file is downloaded once. The client asks again either way.
async fn queue(
    state: &AppState,
    track: &TrackFile,
    opened: &Opened,
    user_id: i64,
) -> Result<Response, HttpError> {
    let media = state.media()?;
    let transcoded = transcoded(media, track, opened);
    if matches!(opened, Opened::Remote(_)) && transcoded.is_none() {
        let key = cache_key(track, opened, AAC_PARAMS)?;
        let transcode =
            tasks::active_id(state.db(), &format!("transcode:{}", key.digest())).await?;
        if let Some(id) = transcode {
            state.scheduler().add_requester(id, user_id);
            return processing(id);
        }
    }
    // Joining a running analysis needs no disk space, so only new work is
    // checked. A remote source without a transcode is copied whole.
    if tasks::active_id(state.db(), &dedupe_key(track.id))
        .await?
        .is_none()
    {
        let copy = match (opened, &transcoded) {
            (Opened::Remote(remote), None) => remote.size.unwrap_or(0),
            _ => 0,
        };
        media.cache.check_space_for(copy).map_err(media_error)?;
    }
    drop(transcoded);
    let submitted = state
        .scheduler()
        .submit(spec(track.id, media), job(state.clone(), track.id))
        .await
        .map_err(|error| match error {
            SubmitError::QueueFull => HttpError::new(
                StatusCode::TOO_MANY_REQUESTS,
                ErrorCode::QueueFull,
                "分析队列已满，请稍后再试",
            ),
            SubmitError::Db(error) => HttpError::from(error),
        })?;
    state.scheduler().add_requester(submitted.id, user_id);
    processing(submitted.id)
}

fn processing(task_id: i64) -> Result<Response, HttpError> {
    let task_id = task_id.try_into().map_err(HttpError::internal)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(PrepareResponse::Processing { task_id }),
    )
        .into_response())
}

/// The published AAC transcode of a remote file. It has the same sound,
/// and reading it costs no download. A file without an mtime is left out:
/// its transcode can be days old, and results derived from it would be
/// dated now and outlive the 7 days such a file is trusted for.
fn transcoded(media: &super::Media, track: &TrackFile, opened: &Opened) -> Option<CachedFile> {
    if !matches!(opened, Opened::Remote(_)) || opened.mtime_ms().is_none() {
        return None;
    }
    let key = cache_key(track, opened, AAC_PARAMS).ok()?;
    media.cache.lookup(&key).ok().flatten()
}

/// What ffmpeg reads. The variants own what keeps their file in place: the
/// temporary copy, or the cache pin.
enum Source {
    Copy(Readable),
    Transcoded(CachedFile),
}

impl Source {
    fn input(&self) -> Input {
        match self {
            Self::Copy(copy) => copy.input.clone(),
            Self::Transcoded(cached) => Input::File(cached.path.clone()),
        }
    }
}

/// Opens what the analysis reads. A remote file is read from its
/// transcode when there is one. Otherwise it is copied whole, but only
/// once `media.remote_analysis_delay` has passed since the task was
/// queued, so a player that just started streaming the file buffers first.
async fn source(
    state: &AppState,
    track: &TrackFile,
    opened: &Opened,
    queued: tokio::time::Instant,
    task: i64,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<Source, JobError> {
    let media = state.media().map_err(|e| JobError(e.message))?;
    if let Opened::Remote(_) = opened {
        if let Some(cached) = transcoded(media, track, opened) {
            return Ok(Source::Transcoded(cached));
        }
        tokio::select! {
            () = tokio::time::sleep_until(queued + media.remote_analysis_delay) => {}
            () = cancel.cancelled() => return Err(JobError("已取消".to_owned())),
        }
        if let Some(cached) = transcoded(media, track, opened) {
            return Ok(Source::Transcoded(cached));
        }
    }
    media
        .cache
        .check_space()
        .map_err(|e| space_failed(task, track.id, e))?;
    tokio::select! {
        copy = readable(media, track, opened, None) => match copy {
            Ok(copy) => Ok(Source::Copy(copy)),
            Err(e) => Err(JobError(name_root(state, track.id, e).await.message)),
        },
        () = cancel.cancelled() => Err(JobError("已取消".to_owned())),
    }
}

fn dedupe_key(track_id: i64) -> String {
    format!("analysis:{track_id}")
}

/// One analysis per track at a time, on the light lane behind probes.
fn spec(track_id: i64, media: &super::Media) -> TaskSpec {
    TaskSpec {
        kind: KIND,
        dedupe_key: Some(dedupe_key(track_id)),
        payload: json!({ "track_id": track_id }),
        lane: Lane::Light,
        timeout: Some(timeout(media)),
        queue_limit: Some(media.queue_limit),
    }
}

/// The scheduler job that measures track `track_id` and writes its
/// waveform. A track measured meanwhile, with its waveform still cached,
/// ends the task at once, and so does a track that is not audio.
pub fn job(state: AppState, track_id: i64) -> Job {
    let queued = tokio::time::Instant::now();
    Box::new(move |ctx| {
        async move {
            let fail = |message: &str| JobError(message.to_owned());
            let task = ctx.task_id;
            let media = state.media().map_err(|_| fail("服务没有配置缓存"))?;
            let track = media::track_file(state.db(), track_id)
                .await
                .map_err(|e| step_failed(task, track_id, "读取轨道失败", e))?
                .ok_or_else(|| fail("轨道已经不存在"))?;
            if track.kind != AUDIO {
                return Ok(None);
            }
            if track.missing {
                return Err(fail("这个文件已经找不到了"));
            }
            let opened = match open_source(&state, &track).await {
                Ok(opened) => opened,
                Err(e) => return Err(JobError(name_root(&state, track_id, e).await.message)),
            };
            let key =
                cache_key(&track, &opened, WAVEFORM_PARAMS).map_err(|_| fail("轨道路径无效"))?;
            let measured = stored(&state, track_id, &opened)
                .await
                .map_err(|_| fail("读取响度失败"))?;
            let cached = media
                .cache
                .lookup(&key)
                .map_err(|e| step_failed(task, track_id, "读取缓存失败", e))?;
            if measured.is_some() && cached.is_some() {
                return Ok(None);
            }
            drop(cached);
            let source = source(&state, &track, &opened, queued, task, &ctx.cancel).await?;
            let input = source.input();
            let duration_ms = parse_probe(&track)
                .and_then(|p| p.duration_ms)
                .or(track.duration_ms);
            let job = AnalysisJob {
                input: &input,
                key: &key,
                duration_ms,
                timeout: media.job_timeout,
            };
            let (loudness, _waveform) = analyze(
                &media.cache,
                job,
                ctx.progress(),
                &ctx.cancel,
                restat(&state, &track),
            )
            .await
            .map_err(|error| match error {
                MediaError::InsufficientStorage => fail("服务器磁盘空间不足"),
                MediaError::SourceChanged => fail("源文件在分析期间变了，请重试"),
                MediaError::Cancelled => fail("已取消"),
                other => {
                    tracing::warn!(track = track_id, error = %other, "analysis failed");
                    fail("分析失败")
                }
            })?;
            let stored = StoredLoudness {
                integrated_lufs: loudness.integrated_lufs,
                true_peak_dbtp: loudness.true_peak_dbtp,
            };
            analysis::save(
                state.db(),
                &track,
                size_of(&opened),
                opened.mtime_ms(),
                stored,
                now_ms(),
            )
            .await
            .map_err(|e| step_failed(task, track_id, "保存响度失败", e))?;
            Ok(None)
        }
        .boxed()
    })
}

/// `media.job_timeout` for the work itself, plus the wait before a
/// remote copy, which is not work and must not use up that budget.
fn timeout(media: &super::Media) -> Duration {
    media.job_timeout + media.remote_analysis_delay
}

/// Rebuilds a queued analysis left by the previous process.
pub fn rebuild(state: &AppState, track_id: i64) -> Option<(Lane, Option<Duration>, Job)> {
    let timeout = timeout(state.media().ok()?);
    Some((Lane::Light, Some(timeout), job(state.clone(), track_id)))
}

/// `GET /api/me/settings`.
pub async fn get_settings(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<UserSettings>, HttpError> {
    let stored = settings::get(state.db(), user.id).await?;
    Ok(Json(settings_dto(stored)))
}

/// `PUT /api/me/settings`: replaces every field.
pub async fn put_settings(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiJson(body): ApiJson<UserSettings>,
) -> Result<Json<UserSettings>, HttpError> {
    let target = body.target_lufs.0;
    if !TARGET_LUFS.contains(&target) {
        return Err(HttpError::invalid_request(
            "target_lufs 要在 -30 到 -10 之间",
        ));
    }
    for seconds in [body.seek_back_seconds, body.seek_forward_seconds] {
        if !UserSettings::SEEK_SECONDS.contains(&seconds) {
            return Err(HttpError::invalid_request(
                "快退和快进的秒数要在 1 到 99 之间",
            ));
        }
    }
    let stored = settings::Settings {
        normalize_loudness: body.normalize_loudness,
        target_lufs: target,
        accent: body.accent,
        seek_back_seconds: body.seek_back_seconds,
        seek_forward_seconds: body.seek_forward_seconds,
    };
    settings::put(state.db(), user.id, stored, now_ms()).await?;
    Ok(Json(settings_dto(stored)))
}

fn settings_dto(stored: settings::Settings) -> UserSettings {
    UserSettings {
        normalize_loudness: stored.normalize_loudness,
        target_lufs: Decibels(stored.target_lufs),
        accent: stored.accent,
        seek_back_seconds: stored.seek_back_seconds,
        seek_forward_seconds: stored.seek_forward_seconds,
    }
}
