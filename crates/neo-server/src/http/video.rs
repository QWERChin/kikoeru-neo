//! HLS for video: the playlist, signed per request, and its segments
//! (contract "M6 接口").

use std::sync::Arc;

use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use futures_util::StreamExt;
use neo_core::dto::ErrorCode;
use neo_media::{CachedFile, HlsMode, PLAYLIST, is_segment_name, sign_playlist};
use neo_storage::LocalFile;

use super::{
    AppState, HttpError,
    extract::{ApiPath, ApiQuery, MediaUser, TokenParam, bearer_present},
    media::{named, playable_track, serve_local},
    playback::{HLS_MIME, cache_key, open_source},
};
use crate::token::{self, Audience};

/// `GET/HEAD /api/tracks/{id}/hls/index.m3u8`.
pub async fn playlist(
    State(state): State<AppState>,
    MediaUser(user): MediaUser,
    ApiPath(id): ApiPath<i64>,
    ApiQuery(query): ApiQuery<TokenParam>,
    request_headers: HeaderMap,
) -> Result<Response, HttpError> {
    named(&state, id, async {
        let (set, cached) = published(&state, id).await?;
        // A media token from the query is reused. A request authorized by an
        // API token gets a new media token, so the API token never ends up
        // in a URL.
        let media_token = match query.token.filter(|_| !bearer_present(&request_headers)) {
            Some(token) => token,
            None => {
                state
                    .tokens()
                    .issue(
                        user.id,
                        user.token_version,
                        Audience::Media,
                        token::now_secs(),
                    )
                    .map_err(HttpError::internal)?
                    .token
            }
        };
        let text = tokio::fs::read_to_string(cached.path.join(PLAYLIST))
            .await
            .map_err(HttpError::internal)?;
        Ok((
            [
                (header::CONTENT_TYPE, HLS_MIME),
                // The segment URIs carry a token, so nothing may store it.
                (header::CACHE_CONTROL, "private, no-store"),
            ],
            sign_playlist(&text, &set, &media_token),
        )
            .into_response())
    })
    .await
}

/// `GET/HEAD /api/tracks/{id}/hls/{set}/{segment}`, with Range support.
/// `set` comes from the playlist, so a set published later, for the other
/// rendition or a changed source, never answers for this one.
pub async fn segment(
    State(state): State<AppState>,
    _user: MediaUser,
    ApiPath((id, set, name)): ApiPath<(i64, String, String)>,
    method: Method,
    headers: HeaderMap,
) -> Result<Response, HttpError> {
    if !is_segment_name(&name) {
        return Err(HttpError::not_found());
    }
    named(&state, id, async {
        let track = playable_track(&state, id).await?;
        if track.kind != "video" {
            return Err(HttpError::not_found());
        }
        let segment = match state.media()?.cache.open_hls_segment(&set, &name) {
            Ok(Some(segment)) => segment,
            Ok(None) => return Err(not_ready()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(HttpError::not_found());
            }
            Err(error) => return Err(HttpError::internal(error)),
        };
        let local = LocalFile {
            file: Arc::new(segment.file.try_clone().map_err(HttpError::internal)?),
            path: name.clone().into(),
            size: segment.len,
            mtime_ms: segment
                .modified
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .and_then(|d| i64::try_from(d.as_millis()).ok()),
        };
        let response = serve_local(&local, "video/mp2t", &method, &headers, None).await?;
        // The body holds the pin, so eviction leaves the set alone while a
        // segment is being sent.
        let (parts, body) = response.into_parts();
        let body = body.into_data_stream().map(move |chunk| {
            let _pin = &segment;
            chunk
        });
        Ok(Response::from_parts(parts, Body::from_stream(body)))
    })
    .await
}

/// The published HLS set of the track as it is now, with its key digest.
/// A transcode is preferred over a copy, since it exists only after the copy
/// failed to play and the client asked for the fallback.
async fn published(state: &AppState, id: i64) -> Result<(String, CachedFile), HttpError> {
    let media = state.media()?;
    let track = playable_track(state, id).await?;
    if track.kind != "video" {
        return Err(HttpError::not_found());
    }
    let opened = open_source(state, &track).await?;
    for mode in [HlsMode::Transcode, HlsMode::Copy] {
        let key = cache_key(&track, &opened, mode.params())?;
        if let Some(cached) = media.cache.lookup(&key).map_err(HttpError::internal)? {
            let id = media
                .cache
                .hls_set_id(&key, &cached)
                .map_err(HttpError::internal)?;
            return Ok((id, cached));
        }
    }
    Err(not_ready())
}

fn not_ready() -> HttpError {
    HttpError::new(
        StatusCode::CONFLICT,
        ErrorCode::MediaNotReady,
        "视频还没准备好，请重新播放",
    )
}
