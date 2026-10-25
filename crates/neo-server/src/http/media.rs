//! Track bytes: `stream`, `download` and `subtitle` (contract "媒体字节").

use std::{io::SeekFrom, sync::Arc, time::Duration};

use axum::body::Bytes;
use axum::{
    Json,
    body::Body,
    extract::State,
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use futures_util::{Stream, StreamExt, stream::BoxStream};
use neo_core::dto::{ErrorCode, Id, TrackFont};
use neo_db::media::{self, TrackFile};
use neo_media::{
    Probe, mime_type,
    range::{self, RangePlan, Validators},
    subtitle::{self, MAX_SUBTITLE_BYTES, SubtitleFormat},
};
use neo_storage::{Access, LocalFile, RawPath, RemoteFile, Source, Storage, StorageError};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

use super::{
    AppState, HttpError,
    extract::{ApiPath, ApiQuery, CurrentUser, MediaUser},
};

/// `GET/HEAD /api/tracks/{id}/stream`: the original file with Range support.
pub async fn stream(
    State(state): State<AppState>,
    _user: MediaUser,
    ApiPath(id): ApiPath<i64>,
    method: Method,
    headers: HeaderMap,
) -> Result<Response, HttpError> {
    named(&state, id, async {
        let track = playable_track(&state, id).await?;
        serve_track(&state, &track, &method, &headers, None).await
    })
    .await
}

/// `GET/HEAD /api/tracks/{id}/download`: like `stream`, as an attachment.
pub async fn download(
    State(state): State<AppState>,
    _user: MediaUser,
    ApiPath(id): ApiPath<i64>,
    method: Method,
    headers: HeaderMap,
) -> Result<Response, HttpError> {
    named(&state, id, async {
        let track = playable_track(&state, id).await?;
        let name = track
            .display_path
            .rsplit('/')
            .next()
            .unwrap_or(&track.display_path)
            .to_owned();
        serve_track(&state, &track, &method, &headers, Some(&name)).await
    })
    .await
}

#[derive(serde::Deserialize)]
pub struct SubtitleParams {
    format: Option<String>,
}

/// `GET /api/tracks/{id}/subtitle`: the track's subtitle as WebVTT, or with
/// `format=ass` its ass subtitle as it is, for JASSUB (contract "M6 接口").
pub async fn subtitle(
    State(state): State<AppState>,
    _user: MediaUser,
    ApiPath(id): ApiPath<i64>,
    ApiQuery(params): ApiQuery<SubtitleParams>,
) -> Result<Response, HttpError> {
    let raw_ass = match params.format.as_deref() {
        None => false,
        Some("ass") => true,
        Some(_) => return Err(HttpError::invalid_request("format 只能是 ass")),
    };
    let extensions: &[&str] = if raw_ass {
        &media::ASS_EXTENSIONS
    } else {
        &media::SUBTITLE_EXTENSIONS
    };
    named(&state, id, async {
        let track = playable_track(&state, id).await?;
        let subtitle_id = media::subtitle_for(state.db(), &track, extensions)
            .await?
            .ok_or_else(|| {
                HttpError::new(
                    StatusCode::NOT_FOUND,
                    ErrorCode::NotFound,
                    "这条轨道没有字幕",
                )
            })?;
        let file = media::track_file(state.db(), subtitle_id)
            .await?
            .ok_or_else(HttpError::not_found)?;
        let format = file
            .display_path
            .rsplit_once('.')
            .and_then(|(_, ext)| SubtitleFormat::from_extension(ext))
            .ok_or_else(HttpError::not_found)?;
        let bytes = match source(&state, &file).await? {
            Source::LocalFile(local) => read_capped(&local, MAX_SUBTITLE_BYTES).await?,
            Source::Remote(remote) => read_remote_capped(&remote, MAX_SUBTITLE_BYTES).await?,
        };
        if raw_ass {
            // JASSUB reads UTF-8 only, and library ass files are often
            // Shift_JIS or GBK, so the text is decoded but not converted.
            let text = tokio::task::spawn_blocking(move || subtitle::decode(&bytes))
                .await
                .map_err(HttpError::internal)?;
            return Ok((
                [
                    (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
                    (header::CACHE_CONTROL, "private, no-cache"),
                ],
                text,
            )
                .into_response());
        }
        // Conversion is CPU work on up to 4 MiB, so it stays off the async
        // workers.
        let vtt = tokio::task::spawn_blocking(move || {
            subtitle::to_webvtt(format, &subtitle::decode(&bytes))
        })
        .await
        .map_err(HttpError::internal)?;
        Ok((
            [
                (header::CONTENT_TYPE, "text/vtt; charset=utf-8"),
                (header::CACHE_CONTROL, "private, no-cache"),
            ],
            vtt,
        )
            .into_response())
    })
    .await
}

/// `GET /api/tracks/{id}/fonts`: the work's font files for JASSUB.
pub async fn fonts(
    State(state): State<AppState>,
    _user: CurrentUser,
    ApiPath(id): ApiPath<i64>,
) -> Result<Json<Vec<TrackFont>>, HttpError> {
    let track = media::track_file(state.db(), id)
        .await?
        .ok_or_else(HttpError::not_found)?;
    let fonts = media::work_fonts(state.db(), track.work_id).await?;
    fonts
        .into_iter()
        .map(|(font_id, path)| {
            Ok(TrackFont {
                id: Id::try_from(font_id).map_err(HttpError::internal)?,
                name: path.rsplit('/').next().unwrap_or(&path).to_owned(),
                url: format!("/api/tracks/{font_id}/stream"),
            })
        })
        .collect::<Result<_, HttpError>>()
        .map(Json)
}

pub(crate) async fn playable_track(state: &AppState, id: i64) -> Result<TrackFile, HttpError> {
    let track = media::track_file(state.db(), id)
        .await?
        .ok_or_else(HttpError::not_found)?;
    if track.missing {
        return Err(missing());
    }
    Ok(track)
}

fn missing() -> HttpError {
    HttpError::new(StatusCode::GONE, ErrorCode::Missing, "这个文件已经找不到了")
}

fn remote_not_supported() -> HttpError {
    HttpError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        ErrorCode::StorageUnavailable,
        "这个媒体库暂时不支持播放",
    )
}

/// The message of a storage outage before [`name_root`] adds the library.
const UNAVAILABLE: &str = "媒体库暂时无法访问";

/// Runs a track handler and, when it fails because the library cannot be
/// reached, names that library in the message (book/src/api.md "错误").
/// Storage code deep inside a request does not know the root's display
/// name, so it is looked up here, and only on that error.
pub(crate) async fn named<T>(
    state: &AppState,
    track_id: i64,
    handler: impl Future<Output = Result<T, HttpError>>,
) -> Result<T, HttpError> {
    match handler.await {
        Ok(value) => Ok(value),
        Err(error) => Err(name_root(state, track_id, error).await),
    }
}

pub(crate) async fn name_root(state: &AppState, track_id: i64, error: HttpError) -> HttpError {
    if error.code != ErrorCode::StorageUnavailable || error.message != UNAVAILABLE {
        return error;
    }
    match media::root_name(state.db(), track_id).await {
        Ok(Some(name)) => HttpError {
            message: format!("媒体库 {name} 暂时无法访问"),
            ..error
        },
        _ => error,
    }
}

/// Storage answers as HTTP errors. The provider's own 401, 403 and 404
/// never reach the browser as such, so it cannot mistake them for its own
/// login expiring or a deleted work (book/src/api.md).
pub(crate) fn storage_error(error: StorageError) -> HttpError {
    match error {
        StorageError::NotFound => missing(),
        StorageError::Unavailable(cause) => {
            tracing::warn!(%cause, "storage unavailable");
            HttpError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::StorageUnavailable,
                UNAVAILABLE,
            )
        }
        StorageError::Upstream(cause) => {
            tracing::warn!(%cause, "storage error");
            HttpError::new(
                StatusCode::BAD_GATEWAY,
                ErrorCode::UpstreamError,
                "媒体库返回了错误",
            )
        }
    }
}

/// The track's path below its root.
pub(crate) fn root_path(track: &TrackFile) -> Result<RawPath, HttpError> {
    let location = RawPath::new(track.location_path.clone()).map_err(HttpError::internal)?;
    let rel = RawPath::new(track.rel_path.clone()).map_err(HttpError::internal)?;
    Ok(location.join_path(&rel))
}

pub(crate) async fn source(state: &AppState, track: &TrackFile) -> Result<Source, HttpError> {
    source_in(&*storage_of(state, &track.root_id)?, track).await
}

/// The storage backend of a root, or the error for a root without one.
pub(crate) fn storage_of(state: &AppState, root_id: &str) -> Result<Arc<dyn Storage>, HttpError> {
    state.storage(root_id).ok_or_else(remote_not_supported)
}

/// [`source`] from a storage the caller already holds.
pub(crate) async fn source_in(
    storage: &dyn Storage,
    track: &TrackFile,
) -> Result<Source, HttpError> {
    storage
        .source(&root_path(track)?)
        .await
        .map_err(storage_error)
}

async fn serve_track(
    state: &AppState,
    track: &TrackFile,
    method: &Method,
    headers: &HeaderMap,
    attachment: Option<&str>,
) -> Result<Response, HttpError> {
    let storage = state
        .storage(&track.root_id)
        .ok_or_else(remote_not_supported)?;
    let access = storage
        .access(&root_path(track)?)
        .await
        .map_err(storage_error)?;
    let probe: Option<Probe> = track
        .probe
        .as_deref()
        .and_then(|p| serde_json::from_str(p).ok());
    let name = track.display_path.rsplit('/').next().unwrap_or_default();
    let mime = mime_type(name, probe.as_ref());
    match access {
        Access::ServeLocal(local) => serve_local(&local, mime, method, headers, attachment).await,
        Access::Remote(remote) => {
            let stall = state.transfer_stall();
            proxy(&remote, mime, method, headers, attachment, stall).await
        }
    }
}

/// Upstream headers a browser needs to play and seek. Anything else, such
/// as cookies or the provider's own caching rules, stays upstream.
const FORWARDED: [header::HeaderName; 6] = [
    header::CONTENT_LENGTH,
    // The upstream is asked for identity, but if it encodes anyway the
    // browser must know, or it would play compressed bytes.
    header::CONTENT_ENCODING,
    header::CONTENT_RANGE,
    header::ETAG,
    header::LAST_MODIFIED,
    header::ACCEPT_RANGES,
];

/// Forwards a remote file (book/src/storage.md "访问方式"). 200, 206 and
/// 416 pass through with their range headers, so an upstream that ignores Range
/// stays a 200. Anything else becomes 502 before any body is sent. The
/// body streams at the client's pace, and dropping it, when the browser
/// goes away, cancels the upstream request.
async fn proxy(
    remote: &RemoteFile,
    mime: &str,
    method: &Method,
    headers: &HeaderMap,
    attachment: Option<&str>,
    stall: Duration,
) -> Result<Response, HttpError> {
    let mut response = Response::builder().header(header::CACHE_CONTROL, "private, no-cache");
    if let Some(name) = attachment {
        response = response.header(header::CONTENT_DISPOSITION, content_disposition(name));
    }
    // HEAD is answered from what the listing knows, without opening the
    // file upstream. A listing may have no size. axum would then label an
    // empty body as length 0, so the body is a stream of unknown length.
    if method != Method::GET {
        response = response
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, mime);
        let body = match remote.size {
            Some(size) => {
                response = response
                    .header(header::CONTENT_LENGTH, size)
                    .header(header::ACCEPT_RANGES, "bytes");
                Body::empty()
            }
            None => {
                Body::from_stream(futures_util::stream::empty::<Result<Bytes, std::io::Error>>())
            }
        };
        return response.body(body).map_err(HttpError::internal);
    }
    // A multi-range request gets the whole file, as for local files: an
    // upstream multipart answer could not be relabelled with one type.
    let mut forward = HeaderMap::new();
    let single_range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|range| !range.contains(','));
    if method == Method::GET && single_range {
        for name in [header::RANGE, header::IF_RANGE] {
            if let Some(value) = headers.get(&name) {
                forward.insert(name, value.clone());
            }
        }
    }
    let upstream = remote.opener.open(&forward).await.map_err(storage_error)?;
    if !matches!(upstream.status.as_u16(), 200 | 206 | 416) {
        tracing::warn!(status = %upstream.status, "upstream refused a file");
        return Err(HttpError::new(
            StatusCode::BAD_GATEWAY,
            ErrorCode::UpstreamError,
            "媒体库返回了错误",
        ));
    }
    response = response.status(upstream.status);
    for name in FORWARDED {
        if let Some(value) = upstream.headers.get(&name) {
            response = response.header(&name, value);
        }
    }
    // The type is always ours, from the extension and probe. The
    // provider's could be text/html or SVG, which a browser would run as
    // script on this origin, where the API token lives.
    response = response
        .header(header::CONTENT_TYPE, mime)
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    response
        .body(Body::from_stream(paced(upstream.body, stall)))
        .map_err(HttpError::internal)
}

/// Chunks held between the upstream and a client that reads slowly.
const PACED_CHUNKS: usize = 8;

/// Hands `body` to the client through a few buffered chunks. The upstream
/// stream holds one of the root's transfer slots, and a paused player can
/// leave its response unread for good, so a client that takes nothing for
/// `stall` is cut off and the slot freed. The body then ends with an error,
/// so a client that comes back cannot take the cut file for a whole one. A
/// browser asks again with a range when it needs more.
fn paced(
    mut body: BoxStream<'static, std::io::Result<Bytes>>,
    stall: Duration,
) -> impl Stream<Item = std::io::Result<Bytes>> {
    let (sender, receiver) = tokio::sync::mpsc::channel(PACED_CHUNKS);
    let (stalled, stall_seen) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        loop {
            // A client that goes away while the upstream is quiet cancels
            // the upstream request at once (book/src/storage.md).
            let chunk = tokio::select! {
                chunk = body.next() => chunk,
                () = sender.closed() => return,
            };
            let Some(chunk) = chunk else { return };
            let last = chunk.is_err();
            match tokio::time::timeout(stall, sender.send(chunk)).await {
                Ok(Ok(())) if !last => {}
                // An upstream error was passed on, or the client is gone.
                Ok(_) => return,
                Err(_) => {
                    tracing::info!("a client stopped reading a remote file");
                    drop(body);
                    let _ = stalled.send(());
                    return;
                }
            }
        }
    });
    let failure = futures_util::stream::once(async move { stall_seen.await.ok() }).filter_map(
        |stalled| async move {
            stalled.map(|()| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "the client stopped reading",
                ))
            })
        },
    );
    futures_util::stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|chunk| (chunk, receiver))
    })
    .chain(failure)
}

async fn read_remote_capped(remote: &RemoteFile, limit: usize) -> Result<Vec<u8>, HttpError> {
    use futures_util::StreamExt;
    let too_large = || {
        HttpError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            ErrorCode::TooLarge,
            "字幕文件太大",
        )
    };
    if remote.size.is_some_and(|size| size > limit as u64) {
        return Err(too_large());
    }
    let mut upstream = remote
        .opener
        .open(&HeaderMap::new())
        .await
        .map_err(storage_error)?;
    // Encoded bytes would parse as an empty subtitle. Identity is asked for,
    // so anything else is an upstream fault.
    let encoded = upstream
        .headers
        .get(header::CONTENT_ENCODING)
        .is_some_and(|v| !v.as_bytes().eq_ignore_ascii_case(b"identity"));
    if upstream.status != StatusCode::OK || encoded {
        return Err(HttpError::new(
            StatusCode::BAD_GATEWAY,
            ErrorCode::UpstreamError,
            "媒体库返回了错误",
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = upstream.body.next().await {
        let chunk = chunk
            .map_err(|e| storage_error(neo_storage::StorageError::Unavailable(e.to_string())))?;
        if bytes.len() + chunk.len() > limit {
            return Err(too_large());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// Answers a GET or HEAD for an open local file, honouring `Range` and
/// `If-Range`. axum drops the body for HEAD.
pub(crate) async fn serve_local(
    local: &LocalFile,
    mime: &str,
    method: &Method,
    headers: &HeaderMap,
    attachment: Option<&str>,
) -> Result<Response, HttpError> {
    let len = local.size;
    let modified = local
        .mtime_ms
        .and_then(|ms| u64::try_from(ms).ok())
        .map(|ms| std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms));
    let validators = Validators::new(len, modified);
    let header_text = |name| {
        headers
            .get(name)
            .and_then(|v: &HeaderValue| v.to_str().ok())
    };
    // Range only applies to GET (RFC 9110 section 14.2). axum answers HEAD
    // with the GET handler, so the method decides here.
    let range_header = if method == Method::GET {
        header_text(header::RANGE)
    } else {
        None
    };
    let plan = range::plan(
        range_header,
        header_text(header::IF_RANGE),
        len,
        &validators,
    );
    let mut response = Response::builder()
        .status(plan.status())
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CACHE_CONTROL, "private, no-cache")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    if let Some(etag) = &validators.etag {
        response = response.header(header::ETAG, etag);
    }
    if let Some(date) = validators.last_modified_header() {
        response = response.header(header::LAST_MODIFIED, date);
    }
    if let Some(content_range) = plan.content_range(len) {
        response = response.header(header::CONTENT_RANGE, content_range);
    }
    if plan == RangePlan::Unsatisfiable {
        return response
            .header(header::CONTENT_LENGTH, 0)
            .body(Body::empty())
            .map_err(HttpError::internal);
    }
    if let Some(name) = attachment {
        response = response.header(header::CONTENT_DISPOSITION, content_disposition(name));
    }
    let (start, body_len) = match plan {
        RangePlan::Partial { start, end } => (start, end - start + 1),
        _ => (0, len),
    };
    // A private descriptor for this response. `try_clone` shares the offset
    // with the original, but the original is never read elsewhere.
    let file = local.file.try_clone().map_err(HttpError::internal)?;
    let mut file = tokio::fs::File::from_std(file);
    file.seek(SeekFrom::Start(start))
        .await
        .map_err(HttpError::internal)?;
    let body = Body::from_stream(ReaderStream::new(file.take(body_len)));
    response
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_LENGTH, body_len)
        .body(body)
        .map_err(HttpError::internal)
}

/// `attachment` with an ASCII fallback name and the real name in RFC 5987
/// form, which every current browser prefers.
pub(crate) fn content_disposition(name: &str) -> String {
    let fallback: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let encoded: String = name
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    format!("attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

async fn read_capped(local: &LocalFile, limit: usize) -> Result<Vec<u8>, HttpError> {
    if local.size > limit as u64 {
        return Err(HttpError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            ErrorCode::TooLarge,
            "字幕文件太大",
        ));
    }
    let file = local.file.try_clone().map_err(HttpError::internal)?;
    let mut file = tokio::fs::File::from_std(file);
    file.seek(SeekFrom::Start(0))
        .await
        .map_err(HttpError::internal)?;
    let mut bytes = Vec::new();
    file.take(limit as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(HttpError::internal)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_disposition_keeps_the_real_name_and_a_safe_fallback() {
        assert_eq!(
            content_disposition("01 トラック.mp3"),
            "attachment; filename=\"01 ____.mp3\"; \
             filename*=UTF-8''01%20%E3%83%88%E3%83%A9%E3%83%83%E3%82%AF.mp3"
        );
        assert!(
            content_disposition("a\"b\\c\r\n.mp3")
                .starts_with("attachment; filename=\"a_b_c__.mp3\"")
        );
    }
}
