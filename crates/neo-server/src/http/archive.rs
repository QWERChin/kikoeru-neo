//! `GET /api/works/{id}/download`: a whole work, or one directory group, as
//! a streamed ZIP (book/src/api.md "播放").

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    io::SeekFrom,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use axum::{
    body::Body,
    extract::State,
    http::{StatusCode, header},
    response::Response,
};
use futures_util::StreamExt;
use neo_core::dto::ErrorCode;
use neo_db::media::{self, TrackFile};
use neo_media::zip::ZipWriter;
use neo_storage::Source;
use serde::Deserialize;
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt, AsyncWrite, AsyncWriteExt},
    sync::{Semaphore, oneshot},
};
use tokio_util::{io::ReaderStream, sync::CancellationToken};

use super::{
    AppState, HttpError,
    extract::{ApiPath, ApiQuery, MediaUser},
    media::{source_in, storage_of},
};

/// Archives being written at once. More would compete for the disk and,
/// for OpenList, for the provider's rate limit.
pub const MAX_ARCHIVES: usize = 2;
/// Buffer between the writer task and the response.
const PIPE_BYTES: usize = 256 * 1024;
/// A client that reads nothing for this long loses its archive slot, or
/// its transfer slot on a remote root. Without it, two stalled downloads
/// would block every other one, and a few paused remote streams a root.
pub const STALL_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Deserialize)]
pub struct Params {
    group: Option<String>,
}

pub async fn download(
    State(state): State<AppState>,
    _user: MediaUser,
    ApiPath(work_id): ApiPath<i64>,
    ApiQuery(params): ApiQuery<Params>,
) -> Result<Response, HttpError> {
    let code = media::work_code(state.db(), work_id)
        .await?
        .ok_or_else(HttpError::not_found)?;
    let group = params
        .group
        .as_deref()
        .map(|g| g.trim_matches('/').to_owned())
        .filter(|g| !g.is_empty());
    let files: Vec<TrackFile> = media::work_files(state.db(), work_id)
        .await?
        .into_iter()
        .filter(|f| match &group {
            Some(group) => f
                .display_path
                .strip_prefix(group.as_str())
                .is_some_and(|rest| rest.starts_with('/')),
            None => true,
        })
        .collect();
    if files.is_empty() {
        return Err(HttpError::new(
            StatusCode::NOT_FOUND,
            ErrorCode::NotFound,
            "这里没有可以下载的文件",
        ));
    }
    let permit = Arc::clone(state.archives())
        .try_acquire_owned()
        .map_err(|_| {
            HttpError::new(
                StatusCode::TOO_MANY_REQUESTS,
                ErrorCode::QueueFull,
                "同时进行的打包下载太多，请稍后再试",
            )
        })?;
    let archive_name = match &group {
        Some(group) => format!("{code}-{}.zip", group.rsplit('/').next().unwrap_or(group)),
        None => format!("{code}.zip"),
    };
    let (reader, writer) = tokio::io::duplex(PIPE_BYTES);
    let (failed, failure) = oneshot::channel::<String>();
    // The body holds the guard, so a client that disconnects ends the task
    // even while it waits on a stalled upstream and writes nothing.
    let disconnected = CancellationToken::new();
    let guard = disconnected.clone().drop_guard();
    tokio::spawn(async move {
        let _permit = permit;
        let written = tokio::select! {
            written = write_archive(&state, files, writer) => written,
            () = disconnected.cancelled() => Err("the client disconnected".to_owned()),
        };
        if let Err(error) = written {
            tracing::warn!(work = work_id, %error, "archive download stopped");
            let _ = failed.send(error);
        }
    });
    // A dropped pipe reads as a normal end, which the browser would save as
    // a complete file. A failure is sent after the bytes instead, so the
    // body ends with an error and the connection is cut.
    let tail = futures_util::stream::once(async move { failure.await.ok() })
        .filter_map(|error| async move { error.map(|e| Err(std::io::Error::other(e))) });
    let body = ReaderStream::new(reader).chain(tail).map(move |chunk| {
        let _ = &guard;
        chunk
    });
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/zip")
        .header(
            header::CONTENT_DISPOSITION,
            super::media::content_disposition(&archive_name),
        )
        .header(header::CACHE_CONTROL, "private, no-store")
        .body(Body::from_stream(body))
        .map_err(HttpError::internal)
}

async fn write_archive(
    state: &AppState,
    files: Vec<TrackFile>,
    writer: tokio::io::DuplexStream,
) -> Result<(), String> {
    let mut zip = ZipWriter::new(StallWriter::new(writer, state.transfer_stall()));
    let mut names = Names::default();
    let mut buffer = vec![0u8; 64 * 1024];
    // Taken once, so a root edited during the download cannot mix files
    // from its old and new location into one archive.
    let mut storages = HashMap::new();
    for file in &files {
        if !storages.contains_key(&file.root_id) {
            let storage = storage_of(state, &file.root_id).map_err(|e| e.message)?;
            storages.insert(file.root_id.clone(), storage);
        }
    }
    for file in files {
        let name = names.add(&file.display_path);
        let storage = &storages[&file.root_id];
        match source_in(&**storage, &file).await.map_err(|e| e.message)? {
            Source::LocalFile(local) => {
                zip.start_file(&name, time_of(local.mtime_ms))
                    .await
                    .map_err(io_message)?;
                let handle = local.file.try_clone().map_err(io_message)?;
                let mut reader = tokio::fs::File::from_std(handle);
                reader.seek(SeekFrom::Start(0)).await.map_err(io_message)?;
                // Exactly the size seen when the file was opened, so a file
                // growing meanwhile cannot make the entry disagree with
                // itself.
                let mut reader = reader.take(local.size);
                loop {
                    let n = reader.read(&mut buffer).await.map_err(io_message)?;
                    if n == 0 {
                        break;
                    }
                    zip.write(&buffer[..n]).await.map_err(io_message)?;
                }
                // A file truncated while it was read must not become a
                // shorter entry with a valid checksum.
                if reader.limit() != 0 {
                    return Err(format!(
                        "{} shrank while it was archived",
                        file.display_path
                    ));
                }
            }
            // Remote files are fetched one at a time, as the book asks, so a
            // download never adds parallel load on the provider.
            Source::Remote(remote) => {
                let upstream = remote
                    .opener
                    .open(&axum::http::HeaderMap::new())
                    .await
                    .map_err(|e| format!("{}: {e}", file.display_path))?;
                if upstream.status != StatusCode::OK {
                    return Err(format!(
                        "{}: upstream answered {}",
                        file.display_path, upstream.status
                    ));
                }
                zip.start_file(&name, time_of(remote.mtime_ms))
                    .await
                    .map_err(io_message)?;
                let mut written = 0u64;
                let mut body = upstream.body;
                while let Some(chunk) = body.next().await {
                    let chunk = chunk.map_err(io_message)?;
                    written += chunk.len() as u64;
                    if remote.size.is_some_and(|size| written > size) {
                        return Err(format!("{} grew while it was archived", file.display_path));
                    }
                    zip.write(&chunk).await.map_err(io_message)?;
                }
                if remote.size.is_some_and(|size| written != size) {
                    return Err(format!("{} ended early", file.display_path));
                }
            }
        }
        zip.finish_file().await.map_err(io_message)?;
    }
    let mut writer = zip.finish().await.map_err(io_message)?;
    writer.shutdown().await.map_err(io_message)
}

fn time_of(mtime_ms: Option<i64>) -> Option<std::time::SystemTime> {
    mtime_ms
        .and_then(|ms| u64::try_from(ms).ok())
        .map(|ms| std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms))
}

fn io_message(error: std::io::Error) -> String {
    error.to_string()
}

/// Fails a write that makes no progress for `stall`. The deadline restarts
/// whenever the pipe accepts bytes, so a slow client that keeps reading is
/// never cut off, while one that stopped reading loses its archive slot.
/// Every write goes through it: headers, data and the central directory.
struct StallWriter<W> {
    inner: W,
    stall: Duration,
    deadline: std::pin::Pin<Box<tokio::time::Sleep>>,
}

impl<W> StallWriter<W> {
    fn new(inner: W, stall: Duration) -> Self {
        Self {
            inner,
            stall,
            deadline: Box::pin(tokio::time::sleep(stall)),
        }
    }

    fn progress<T>(
        &mut self,
        poll: Poll<std::io::Result<T>>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<T>> {
        match poll {
            Poll::Ready(result) => {
                let next = tokio::time::Instant::now() + self.stall;
                self.deadline.as_mut().reset(next);
                Poll::Ready(result)
            }
            Poll::Pending => match self.deadline.as_mut().poll(cx) {
                Poll::Ready(()) => Poll::Ready(Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "the client stopped reading",
                ))),
                Poll::Pending => Poll::Pending,
            },
        }
    }
}

impl<W: AsyncWrite + Unpin> AsyncWrite for StallWriter<W> {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let poll = std::pin::Pin::new(&mut self.inner).poll_write(cx, buf);
        self.progress(poll, cx)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        let poll = std::pin::Pin::new(&mut self.inner).poll_flush(cx);
        self.progress(poll, cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        let poll = std::pin::Pin::new(&mut self.inner).poll_shutdown(cx);
        self.progress(poll, cx)
    }
}

/// Entry names already used, as files and as directories. Two locations
/// can hold different files under one path, or a file where the other has
/// a directory. The later one is renamed with ` (2)`, ` (3)` and so on, so
/// the archive extracts without collisions.
#[derive(Default)]
struct Names {
    files: HashSet<String>,
    dirs: HashSet<String>,
    /// Directory paths, as listed, moved aside because a file took the name.
    moved: HashMap<String, String>,
}

impl Names {
    fn taken(&self, path: &str) -> bool {
        self.files.contains(path) || self.dirs.contains(path)
    }

    fn add(&mut self, path: &str) -> String {
        let path = entry_path(path);
        let (dirs, file) = match path.rsplit_once('/') {
            Some((dirs, file)) => (dirs.split('/').collect::<Vec<_>>(), file),
            None => (Vec::new(), path.as_str()),
        };
        let mut original = String::new();
        let mut current = String::new();
        for dir in dirs {
            original = join(&original, dir);
            let wanted = join(&current, dir);
            current = if let Some(moved) = self.moved.get(&original) {
                moved.clone()
            } else if self.files.contains(&wanted) {
                let moved = self.fresh(&current, dir, "");
                self.moved.insert(original.clone(), moved.clone());
                moved
            } else {
                wanted
            };
            self.dirs.insert(current.clone());
        }
        let wanted = join(&current, file);
        let name = if self.taken(&wanted) {
            // Only the final component's extension moves.
            match file.rfind('.') {
                Some(dot) if dot > 0 => self.fresh(&current, &file[..dot], &file[dot..]),
                _ => self.fresh(&current, file, ""),
            }
        } else {
            wanted
        };
        self.files.insert(name.clone());
        name
    }

    fn fresh(&self, parent: &str, stem: &str, ext: &str) -> String {
        (2..)
            .map(|n| join(parent, &format!("{stem} ({n}){ext}")))
            .find(|candidate| !self.taken(candidate))
            .expect("an unused name exists")
    }
}

/// A display path made safe as a zip entry name. Every component stays
/// inside the extraction directory on any system: Windows extractors split
/// on `\` and read `C:` as a drive, so both become `_`, as do control
/// characters. A component of only dots and spaces, such as `..` or `.. `,
/// gets a `_` in front.
fn entry_path(display: &str) -> String {
    let path = display
        .split('/')
        .filter(|c| !c.is_empty())
        .map(|c| {
            let c: String = c
                .chars()
                .map(|ch| match ch {
                    '\\' | ':' => '_',
                    ch if ch.is_control() => '_',
                    ch => ch,
                })
                .collect();
            // Windows drops trailing dots and spaces, so any name made of
            // only those could resolve to `.` or `..` there.
            if c.chars().all(|ch| ch == '.' || ch == ' ') {
                format!("_{c}")
            } else {
                c
            }
        })
        .collect::<Vec<_>>()
        .join("/");
    if path.is_empty() {
        "_".to_owned()
    } else {
        path
    }
}

fn join(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}/{name}")
    }
}

/// A semaphore for [`MAX_ARCHIVES`].
pub fn archive_slots() -> Arc<Semaphore> {
    Arc::new(Semaphore::new(MAX_ARCHIVES))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_paths_get_numbered_names() {
        let mut names = Names::default();
        assert_eq!(names.add("mp3/01.mp3"), "mp3/01.mp3");
        assert_eq!(names.add("mp3/01.mp3"), "mp3/01 (2).mp3");
        assert_eq!(names.add("mp3/01.mp3"), "mp3/01 (3).mp3");
        assert_eq!(names.add("README"), "README");
        assert_eq!(names.add("README"), "README (2)");
        assert_eq!(names.add("ver.1/README"), "ver.1/README");
        assert_eq!(names.add("ver.1/README"), "ver.1/README (2)");
        assert_eq!(names.add("a/.hidden"), "a/.hidden");
        assert_eq!(names.add("a/.hidden"), "a/.hidden (2)");
    }

    #[test]
    fn entry_names_cannot_leave_the_extraction_directory() {
        let mut names = Names::default();
        // Legal on Linux, but Windows extractors split on `\` and read a
        // drive prefix.
        assert_eq!(names.add("..\\..\\evil.txt"), ".._.._evil.txt");
        assert_eq!(names.add("C:\\x.txt"), "C__x.txt");
        assert_eq!(names.add("ok/C:"), "ok/C_");
        assert_eq!(names.add("bell\u{7}/a.mp3"), "bell_/a.mp3");
        assert_eq!(names.add("/abs//./../x.mp3"), "abs/_./_../x.mp3");
        // Windows drops trailing dots and spaces, so these become `..`.
        assert_eq!(names.add(".. /... /x.mp3"), "_.. /_... /x.mp3");
    }

    proptest::proptest! {
        /// Whatever the display paths, every entry name is relative, has
        /// only ordinary components, and is unique in the archive.
        #[test]
        fn entry_names_are_safe_and_unique(
            paths in proptest::collection::vec("[a. /\\\\:\u{0}-\u{1f}\u{7f}ä]{0,12}", 1..20)
        ) {
            let mut names = Names::default();
            let mut seen = HashSet::new();
            for path in &paths {
                let name = names.add(path);
                proptest::prop_assert!(!name.is_empty());
                proptest::prop_assert!(
                    name.split('/').all(|c| !c.chars().all(|ch| ch == '.' || ch == ' ')),
                    "{name:?}"
                );
                proptest::prop_assert!(!name.starts_with('/'));
                for c in name.split('/') {
                    proptest::prop_assert!(!c.is_empty() && c != "." && c != "..", "{name:?}");
                    proptest::prop_assert!(
                        !c.chars().any(|ch| ch == '\\' || ch == ':' || ch.is_control()),
                        "{name:?}"
                    );
                }
                proptest::prop_assert!(seen.insert(name.clone()), "{name:?} repeated");
            }
        }
    }

    #[test]
    fn a_file_and_a_directory_never_share_a_name() {
        let mut names = Names::default();
        assert_eq!(names.add("extras"), "extras");
        assert_eq!(names.add("extras/01.mp3"), "extras (2)/01.mp3");
        assert_eq!(
            names.add("extras/02.mp3"),
            "extras (2)/02.mp3",
            "moved consistently"
        );
        let mut names = Names::default();
        assert_eq!(names.add("bonus/01.mp3"), "bonus/01.mp3");
        assert_eq!(names.add("bonus"), "bonus (2)");
    }
}
