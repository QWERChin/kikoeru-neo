//! `GET /api/works/{id}/cover`: the original cover or a 480 px wide
//! thumbnail, generated on first request and cached by cover version, as
//! "封面" in book/src/scanning.md describes.

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use image::{ImageReader, codecs::jpeg::JpegEncoder, imageops::FilterType};
use serde::Deserialize;

use super::{
    AppState, HttpError,
    extract::{ApiPath, ApiQuery, CoverUser},
    web::etag_matches,
    works::work_id,
};
use crate::datadir::DataDir;

pub const THUMB_WIDTH: u32 = 480;
const THUMB_QUALITY: u8 = 85;

/// A neutral square shown for works without a cover. The frontend draws
/// its own placeholder too, but an `<img>` needs some image to show.
const PLACEHOLDER: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 480 480"><rect width="480" height="480" fill="#d9d9de"/><path d="M200 170v118a34 34 0 1 1-16-29v-89l112-22v104a34 34 0 1 1-16-29v-75z" fill="#a3a3ab"/></svg>"##;

/// Covers are private to signed-in users, and their URLs carry no token,
/// so a browser that kept one could show it after its user signs out.
/// Every load is therefore revalidated: the request carries the media token
/// cookie, and an unchanged cover answers 304 without a body, so a cached
/// cover still costs no image bytes.
const CACHE: &str = "private, no-cache";

#[derive(Deserialize)]
pub struct Params {
    size: Option<String>,
}

pub async fn cover(
    State(state): State<AppState>,
    CoverUser(_): CoverUser,
    ApiPath(id): ApiPath<String>,
    ApiQuery(params): ApiQuery<Params>,
    headers: HeaderMap,
) -> Result<Response, HttpError> {
    let id = work_id(&id)?;
    let thumb = match params.size.as_deref() {
        None => false,
        Some("thumb") => true,
        Some(v) => {
            return Err(HttpError::invalid_request(format!(
                "size 只能是 thumb，收到 {v:?}"
            )));
        }
    };
    // A metadata refresh replaces the file and bumps cover_version in two
    // steps. Holding this until the file is open pairs the version with
    // the file it belongs to.
    let reading = match state.cover_lock() {
        Some(lock) => Some(lock.read_owned().await),
        None => None,
    };
    let row = neo_db::media::cover_state(state.db(), id)
        .await?
        .ok_or_else(HttpError::work_not_found)?;
    // The `v` parameter, ignored here, only gives a changed cover a new
    // URL. The ETag below is what the server checks.
    let unsettled = reading.as_ref().is_some_and(|r| r.contains(&id));
    if !row.has_cover || unsettled {
        return Ok(placeholder());
    }
    // A metadata refresh bumps cover_version whenever it replaces the file,
    // so the version names the bytes.
    let etag = format!(
        "\"{id}-{}-{}\"",
        row.cover_version,
        if thumb { "thumb" } else { "full" }
    );
    let data_dir = state.data_dir()?.clone();
    let original = data_dir.root().join(format!("covers/{id}.jpg"));
    let opened = open_regular(&original).await;
    drop(reading);
    // Only once the file is there: a database restored without its covers
    // must hand the browser the placeholder, not a 304 for a stale picture.
    if opened.is_ok() && etag_matches(&headers, &etag) {
        return Ok(not_modified(&etag));
    }
    let bytes = match opened {
        Err(e) => Err(e),
        // Streamed, so a large original costs no more memory than a small
        // one.
        Ok(file) if !thumb => match file.metadata().await {
            Ok(meta) => return Ok(original_response(file, meta.len(), &etag)),
            Err(e) => Err(e),
        },
        Ok(file) => {
            // The permit moves into the blocking task: if the request is
            // cancelled, the task still runs to the end and must keep
            // counting against the limit until it does.
            let permit = state
                .thumbnails()
                .acquire_owned()
                .await
                .map_err(HttpError::internal)?;
            let version = row.cover_version;
            let file = file.into_std().await;
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                thumbnail(&data_dir, file, id, version)
            })
            .await
            .map_err(HttpError::internal)?
        }
    };
    match bytes {
        Ok(bytes) => Ok((
            [
                (header::CONTENT_TYPE, "image/jpeg"),
                (header::CACHE_CONTROL, CACHE),
                (header::ETAG, &etag),
            ],
            bytes,
        )
            .into_response()),
        // The row says there is a cover but the file is gone or unreadable,
        // for example after restoring only the database. The work still
        // shows, with the placeholder.
        Err(e) => {
            tracing::warn!(work = id, path = %original.display(), error = %e, "cover not readable");
            Ok(placeholder())
        }
    }
}

fn not_modified(etag: &str) -> Response {
    (
        StatusCode::NOT_MODIFIED,
        [
            (header::CACHE_CONTROL, CACHE.to_owned()),
            (header::ETAG, etag.to_owned()),
        ],
    )
        .into_response()
}

fn original_response(file: tokio::fs::File, len: u64, etag: &str) -> Response {
    (
        [
            (header::CONTENT_TYPE, "image/jpeg".to_owned()),
            (header::CACHE_CONTROL, CACHE.to_owned()),
            (header::ETAG, etag.to_owned()),
            (header::CONTENT_LENGTH, len.to_string()),
        ],
        axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(file)),
    )
        .into_response()
}

/// Open flags that refuse a symlink in the last component: `O_NOFOLLOW`,
/// plus `O_NONBLOCK` so a FIFO cannot hang the open. On Windows,
/// `FILE_FLAG_OPEN_REPARSE_POINT` opens a link itself, which then fails
/// the regular-file check.
#[cfg(unix)]
const NO_FOLLOW: i32 = rustix::fs::OFlags::NOFOLLOW
    .union(rustix::fs::OFlags::NONBLOCK)
    .bits() as i32;
#[cfg(windows)]
const NO_FOLLOW: u32 = 0x0020_0000;

fn open_std_no_follow(path: &Path) -> io::Result<fs::File> {
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    #[cfg(windows)]
    use std::os::windows::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(NO_FOLLOW)
        .open(path)
}

/// Opens a file only if it is a regular file reached without following a
/// symlink in its last component. Anyone who can write to `covers/` or the
/// thumbnail cache could otherwise point a cover at the signing key or the
/// database. Non-blocking, so a FIFO planted there cannot hang the open.
async fn open_regular(path: &Path) -> io::Result<tokio::fs::File> {
    let file = tokio::fs::OpenOptions::new()
        .read(true)
        .custom_flags(NO_FOLLOW)
        .open(path)
        .await?;
    if !file.metadata().await?.is_file() {
        return Err(io::Error::other("not a regular file"));
    }
    Ok(file)
}

/// Never cached for long: once a cover appears, the next request must see
/// it even if its version did not change.
fn placeholder() -> Response {
    (
        [
            (header::CONTENT_TYPE, "image/svg+xml"),
            (header::CACHE_CONTROL, CACHE),
        ],
        PLACEHOLDER,
    )
        .into_response()
}

/// Returns the cached thumbnail, generating it first if needed. The file
/// is written under `cache/tmp` and renamed into place, so a reader never
/// sees half of it and a crash leaves nothing behind that startup does not
/// clear.
fn thumbnail(data_dir: &DataDir, original: fs::File, id: i64, version: i64) -> io::Result<Vec<u8>> {
    let path = data_dir
        .root()
        .join(format!("cache/thumbs/{id}-{version}.jpg"));
    // Anything that is not a plain cached file, a planted symlink
    // included, is rebuilt; the rename below replaces the link itself.
    let cached = open_std_no_follow(&path).and_then(|mut f| {
        if !f.metadata()?.is_file() {
            return Err(io::Error::other("not a regular file"));
        }
        let mut bytes = Vec::new();
        io::Read::read_to_end(&mut f, &mut bytes)?;
        Ok(bytes)
    });
    if let Ok(bytes) = cached {
        return Ok(bytes);
    }
    let bytes = render(original)?;
    let tmp = tmp_path(data_dir, id, version);
    let written = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, &path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    // A failed cache write still serves the thumbnail just made.
    if let Err(e) = written {
        tracing::warn!(path = %path.display(), error = %e, "cannot cache thumbnail");
    }
    Ok(bytes)
}

fn tmp_path(data_dir: &DataDir, id: i64, version: i64) -> PathBuf {
    let nonce: u64 = getrandom::u64().unwrap_or_default();
    data_dir
        .root()
        .join(format!("cache/tmp/thumb-{id}-{version}-{nonce:016x}.jpg"))
}

/// Narrower images are re-encoded as they are, so every thumbnail is a
/// JPEG of bounded size.
fn render(original: fs::File) -> io::Result<Vec<u8>> {
    let image = ImageReader::new(io::BufReader::new(original))
        .with_guessed_format()?
        .decode()
        .map_err(io::Error::other)?;
    let image = if image.width() > THUMB_WIDTH {
        image.resize(THUMB_WIDTH, u32::MAX, FilterType::Triangle)
    } else {
        image
    };
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, THUMB_QUALITY)
        .encode_image(&image.to_rgb8())
        .map_err(io::Error::other)?;
    Ok(out)
}
