//! Proxying remote (OpenList) files through `stream`, `download` and
//! `subtitle`, with a scripted remote storage.

mod common;

use std::sync::{Arc, LazyLock, Mutex};

use axum::http::{HeaderMap, StatusCode, header};
use common::{App, Req, app_with_cloud_and, assert_error};
use futures_util::{StreamExt, stream};
use neo_server::http::AppState;
use neo_storage::{
    Access, Entry, EntryKind, RawPath, RemoteFile, RemoteOpen, RemoteResponse, Source, Storage,
    StorageError,
};

const CONTENT: &[u8] = b"0123456789";
const LYRICS: &str = "[00:01.00]remote line\n";

/// How the fake upstream answers the next open.
#[derive(Clone, Copy, PartialEq)]
enum Upstream {
    Normal,
    IgnoresRange,
    Forbidden,
    Unavailable,
    Gzipped,
    /// Sends headers, then never a byte.
    Stalls,
    /// Sends bytes for as long as anyone reads them.
    Endless,
}

struct Remote {
    mode: Mutex<Upstream>,
    seen: Mutex<Vec<HeaderMap>>,
    opens: Mutex<usize>,
    /// Stalled and endless bodies not dropped yet: each holds a transfer
    /// slot.
    live: Mutex<usize>,
}

/// Counts an endless body as live until it is dropped.
struct LiveBody(Arc<Remote>);

impl Drop for LiveBody {
    fn drop(&mut self) {
        *self.0.live.lock().unwrap() -= 1;
    }
}

struct Opener {
    remote: Arc<Remote>,
    content: Vec<u8>,
}

#[async_trait::async_trait]
impl RemoteOpen for Opener {
    async fn open(&self, headers: &HeaderMap) -> Result<RemoteResponse, StorageError> {
        *self.remote.opens.lock().unwrap() += 1;
        self.remote.seen.lock().unwrap().push(headers.clone());
        let mode = *self.remote.mode.lock().unwrap();
        let mut out = HeaderMap::new();
        out.insert(
            header::CONTENT_TYPE,
            "application/octet-stream".parse().unwrap(),
        );
        out.insert(header::ETAG, "\"upstream\"".parse().unwrap());
        out.insert(header::SET_COOKIE, "provider=secret".parse().unwrap());
        let len = self.content.len();
        let range = headers
            .get(header::RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|r| r.strip_prefix("bytes="))
            .and_then(|r| r.split_once('-'))
            .and_then(|(start, end)| {
                let start = start.parse::<usize>().ok()?;
                let end = if end.is_empty() {
                    len - 1
                } else {
                    end.parse::<usize>().ok()?.min(len - 1)
                };
                Some((start, end))
            });
        let (status, body) = match (mode, range) {
            (Upstream::Unavailable, _) => {
                return Err(StorageError::Unavailable("provider throttled".into()));
            }
            (Upstream::Forbidden, _) => (StatusCode::FORBIDDEN, Vec::new()),
            (Upstream::Normal, Some((start, end))) => {
                out.insert(
                    header::CONTENT_RANGE,
                    format!("bytes {start}-{end}/{len}").parse().unwrap(),
                );
                (
                    StatusCode::PARTIAL_CONTENT,
                    self.content[start..=end].to_vec(),
                )
            }
            _ => (StatusCode::OK, self.content.clone()),
        };
        if mode == Upstream::Gzipped {
            out.insert(header::CONTENT_ENCODING, "gzip".parse().unwrap());
        }
        out.insert(header::CONTENT_LENGTH, body.len().into());
        Ok(RemoteResponse {
            status,
            headers: out,
            // Two chunks, so a reader that stops at a chunk boundary is
            // caught: stale.wav's listing ends exactly at the split.
            body: if mode == Upstream::Stalls {
                *self.remote.live.lock().unwrap() += 1;
                let live = LiveBody(Arc::clone(&self.remote));
                Box::pin(
                    stream::pending::<Result<bytes::Bytes, std::io::Error>>().map(move |c| {
                        let _ = &live;
                        c
                    }),
                )
            } else if mode == Upstream::Endless {
                *self.remote.live.lock().unwrap() += 1;
                let live = LiveBody(Arc::clone(&self.remote));
                Box::pin(stream::repeat_with(move || {
                    let _ = &live;
                    Ok(bytes::Bytes::from(vec![0u8; 64 * 1024]))
                }))
            } else {
                let mut first = bytes::Bytes::from(body);
                let second = first.split_off(first.len() / 2);
                Box::pin(stream::iter([Ok(first), Ok(second)]))
            },
        })
    }
}

struct Cloud(Arc<Remote>);

impl Cloud {
    fn file(&self, path: &RawPath) -> Result<RemoteFile, StorageError> {
        let content = match path.to_string_lossy().as_str() {
            "RJ002/01.mp3" | "RJ002/unsized.mp3" => CONTENT.to_vec(),
            "RJ002/01.lrc" => LYRICS.as_bytes().to_vec(),
            "RJ002/short.wav" | "RJ002/stale.wav" | "RJ002/huge.wav" => SHORT_WAV.clone(),
            "RJ002/long.wav" | "RJ002/undated.wav" => LONG_WAV.clone(),
            "RJ002/late.m4a" => LATE_M4A.clone(),
            "RJ002/movie.mkv" => MOVIE_MKV.clone(),
            _ => return Err(StorageError::NotFound),
        };
        // The listing of stale.wav still has the size of an older version.
        let listed = match path.to_string_lossy().as_str() {
            "RJ002/stale.wav" => content.len() as u64 / 2,
            "RJ002/huge.wav" => HUGE,
            _ => content.len() as u64,
        };
        // Some providers list no modification time.
        let mtime_ms = match path.to_string_lossy().as_str() {
            "RJ002/undated.wav" => None,
            _ => Some(1_700_000_000_000),
        };
        // WebDAV may list a file without getcontentlength.
        let size = (path.to_string_lossy() != "RJ002/unsized.mp3").then_some(listed);
        Ok(RemoteFile {
            size,
            mtime_ms,
            opener: Arc::new(Opener {
                remote: Arc::clone(&self.0),
                content,
            }),
        })
    }
}

#[async_trait::async_trait]
impl Storage for Cloud {
    async fn list(&self, _: &RawPath) -> Result<Vec<Entry>, StorageError> {
        Ok(Vec::new())
    }

    async fn stat(&self, path: &RawPath) -> Result<Entry, StorageError> {
        let file = self.file(path)?;
        Ok(Entry {
            name: path.file_name().unwrap_or_default().to_vec(),
            kind: EntryKind::File,
            size: file.size,
            mtime_ms: file.mtime_ms,
            etag: None,
        })
    }

    async fn source(&self, path: &RawPath) -> Result<Source, StorageError> {
        self.file(path).map(Source::Remote)
    }

    async fn access(&self, path: &RawPath) -> Result<Access, StorageError> {
        self.file(path).map(Access::Remote)
    }
}

async fn library() -> (App, Arc<Remote>, String) {
    library_with(|s, _| s).await
}

async fn library_with(
    configure: impl FnOnce(AppState, &std::path::Path) -> AppState,
) -> (App, Arc<Remote>, String) {
    let remote = Arc::new(Remote {
        mode: Mutex::new(Upstream::Normal),
        seen: Mutex::new(Vec::new()),
        opens: Mutex::new(0),
        live: Mutex::new(0),
    });
    let app = app_with_cloud_and(Arc::new(Cloud(Arc::clone(&remote))), configure).await;
    app.user("admin", true).await;
    app.user("alice", false).await;
    app.exec(
        "INSERT INTO roots (id, display_name) VALUES ('cloud', 'Cloud');
         INSERT INTO works (id, code, title, metadata_src, created_at, updated_at)
           VALUES (2, 'RJ000002', 'Remote work', 'scan', 0, 0);
         INSERT INTO work_locations (id, work_id, root_id, rel_path, display_path)
           VALUES (2, 2, 'cloud', CAST('RJ002' AS BLOB), 'RJ002');
         INSERT INTO tracks (id, work_id, location_id, rel_path, display_path, sort_key, kind) VALUES
           (1, 2, 2, CAST('01.mp3' AS BLOB), '01.mp3', 'a', 'audio'),
           (2, 2, 2, CAST('01.lrc' AS BLOB), '01.lrc', 'b', 'subtitle'),
           (3, 2, 2, CAST('gone.mp3' AS BLOB), 'gone.mp3', 'c', 'audio');
         INSERT INTO tracks (id, work_id, location_id, rel_path, display_path, sort_key, kind, mtime) VALUES
           (4, 2, 2, CAST('short.wav' AS BLOB), 'short.wav', 'd', 'audio', 1700000000000),
           (5, 2, 2, CAST('long.wav' AS BLOB), 'long.wav', 'e', 'audio', 1700000000000),
           (6, 2, 2, CAST('late.m4a' AS BLOB), 'late.m4a', 'f', 'audio', 1700000000000),
           (7, 2, 2, CAST('stale.wav' AS BLOB), 'stale.wav', 'g', 'audio', 1700000000000),
           (8, 2, 2, CAST('huge.wav' AS BLOB), 'huge.wav', 'h', 'audio', 1700000000000),
           (9, 2, 2, CAST('movie.mkv' AS BLOB), 'movie.mkv', 'i', 'video', 1700000000000);",
    )
    .await;
    let token = app.login("alice").await;
    (app, remote, token)
}

#[tokio::test]
async fn range_requests_are_forwarded_and_answered_by_the_upstream() {
    let (app, remote, token) = library().await;
    let (status, headers, body) = app
        .send_raw(
            Req::get("/api/tracks/1/stream")
                .token(&token)
                .header("range", "bytes=4-")
                .header("cookie", "browser=private"),
        )
        .await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(body, b"456789");
    assert_eq!(headers["content-range"], "bytes 4-9/10");
    assert_eq!(headers["etag"], "\"upstream\"");
    assert_eq!(
        headers["content-type"], "audio/mpeg",
        "a generic type is replaced"
    );
    assert!(
        !headers.contains_key("set-cookie"),
        "provider headers stay upstream"
    );
    let seen = remote.seen.lock().unwrap();
    assert_eq!(seen[0]["range"], "bytes=4-");
    assert!(!seen[0].contains_key("cookie"), "browser headers stay here");
}

#[tokio::test]
async fn an_upstream_that_ignores_range_stays_a_200() {
    let (app, remote, token) = library().await;
    *remote.mode.lock().unwrap() = Upstream::IgnoresRange;
    let (status, _, body) = app
        .send_raw(
            Req::get("/api/tracks/1/stream")
                .token(&token)
                .header("range", "bytes=4-"),
        )
        .await;
    assert_eq!((status, body.as_slice()), (StatusCode::OK, CONTENT));
}

#[tokio::test]
async fn upstream_refusals_become_502_and_outages_503() {
    let (app, remote, token) = library().await;
    *remote.mode.lock().unwrap() = Upstream::Forbidden;
    let (status, body) = app
        .send(Req::get("/api/tracks/1/stream").token(&token))
        .await;
    assert_error(status, &body, StatusCode::BAD_GATEWAY, "upstream_error");
    *remote.mode.lock().unwrap() = Upstream::Unavailable;
    let (status, body) = app
        .send(Req::get("/api/tracks/1/stream").token(&token))
        .await;
    assert_error(
        status,
        &body,
        StatusCode::SERVICE_UNAVAILABLE,
        "storage_unavailable",
    );
    // The message names the library, so the user knows which one is down.
    assert_eq!(body["error"]["message"], "媒体库 Cloud 暂时无法访问");
    set_size(&app, 4).await;
    let (status, body) = app
        .send(
            Req::post("/api/tracks/4/prepare")
                .token(&token)
                .json(serde_json::json!({"can_play": ["audio/wav"]})),
        )
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["error"]["message"], "媒体库 Cloud 暂时无法访问");
    let (status, body) = app
        .send(Req::get("/api/tracks/3/stream").token(&token))
        .await;
    assert_error(status, &body, StatusCode::GONE, "missing");
}

#[tokio::test]
async fn head_is_answered_without_opening_the_file() {
    let (app, remote, token) = library().await;
    let (status, headers, body) = app
        .send_raw(
            Req::head("/api/tracks/1/stream")
                .token(&token)
                .header("range", "bytes=0-0"),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-length"], "10");
    assert!(body.is_empty());
    assert_eq!(*remote.opens.lock().unwrap(), 0);
}

#[tokio::test]
async fn head_loudness_does_not_queue_remote_analysis() {
    assert_head_does_not_queue_analysis("loudness").await;
}

#[tokio::test]
async fn head_waveform_does_not_queue_remote_analysis() {
    assert_head_does_not_queue_analysis("waveform").await;
}

async fn assert_head_does_not_queue_analysis(route: &str) {
    let (app, remote, token) = library().await;
    let admin = app.login("admin").await;
    let (status, _, body) = app
        .send_raw(Req::head(&format!("/api/tracks/4/{route}")).token(&token))
        .await;
    assert!(body.is_empty());
    let (tasks_status, tasks) = app.send(Req::get("/api/tasks").token(&admin)).await;
    assert_eq!(tasks_status, StatusCode::OK, "{tasks}");
    assert_eq!(tasks["total"], 0, "HEAD queued a task: {tasks}");
    assert_eq!(*remote.opens.lock().unwrap(), 0, "nothing was fetched");
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn head_of_a_file_of_unknown_size_does_not_open_it() {
    let (app, remote, token) = library().await;
    app.exec(
        "INSERT INTO tracks (id, work_id, location_id, rel_path, display_path, sort_key, kind)
         VALUES (10, 2, 2, CAST('unsized.mp3' AS BLOB), 'unsized.mp3', 'j', 'audio')",
    )
    .await;
    for route in ["stream", "download"] {
        let (status, headers, body) = app
            .send_raw(Req::head(&format!("/api/tracks/10/{route}")).token(&token))
            .await;
        assert_eq!(status, StatusCode::OK, "{route}");
        assert!(
            !headers.contains_key("content-length"),
            "{route}: no made-up length"
        );
        assert!(body.is_empty());
    }
    assert_eq!(*remote.opens.lock().unwrap(), 0);
}

#[tokio::test]
async fn download_and_subtitle_work_for_remote_files() {
    let (app, _remote, token) = library().await;
    let (status, headers, body) = app
        .send_raw(Req::get("/api/tracks/1/download").token(&token))
        .await;
    assert_eq!((status, body.as_slice()), (StatusCode::OK, CONTENT));
    assert!(
        headers["content-disposition"]
            .to_str()
            .unwrap()
            .starts_with("attachment")
    );

    let (status, _, body) = app
        .send_raw(Req::get("/api/tracks/1/subtitle").token(&token))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8(body).unwrap().contains("remote line"));
}

#[tokio::test]
async fn multi_range_requests_ask_upstream_for_the_whole_file() {
    let (app, remote, token) = library().await;
    let (status, _, body) = app
        .send_raw(
            Req::get("/api/tracks/1/stream")
                .token(&token)
                .header("range", "bytes=0-1,5-6")
                .header("if-range", "\"upstream\""),
        )
        .await;
    assert_eq!((status, body.as_slice()), (StatusCode::OK, CONTENT));
    let seen = remote.seen.lock().unwrap();
    assert!(!seen[0].contains_key("range") && !seen[0].contains_key("if-range"));
}

#[tokio::test]
async fn an_encoded_subtitle_is_refused_instead_of_parsed_as_empty() {
    let (app, remote, token) = library().await;
    *remote.mode.lock().unwrap() = Upstream::Gzipped;
    let (status, body) = app
        .send(Req::get("/api/tracks/1/subtitle").token(&token))
        .await;
    assert_error(status, &body, StatusCode::BAD_GATEWAY, "upstream_error");
}

/// The listed size of huge.wav, more than any test machine has free.
const HUGE: u64 = 1 << 42;

/// H.264 + AAC in matroska, which HLS takes without re-encoding.
static MOVIE_MKV: LazyLock<Vec<u8>> = LazyLock::new(|| {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("movie.mkv");
    let status = std::process::Command::new("ffmpeg")
        .args(["-nostdin", "-loglevel", "error", "-f", "lavfi", "-i"])
        .arg("testsrc=size=320x240:rate=25:duration=8")
        .args(["-f", "lavfi", "-i", "sine=duration=8"])
        .args(["-c:v", "libx264", "-g", "25", "-c:a", "aac", "-y"])
        .arg(&out)
        .status()
        .unwrap();
    assert!(status.success());
    std::fs::read(out).unwrap()
});

static SHORT_WAV: LazyLock<Vec<u8>> = LazyLock::new(|| generated(2, &["-f", "wav"]));
static LONG_WAV: LazyLock<Vec<u8>> = LazyLock::new(|| generated(30, &["-f", "wav"]));
/// AAC in MP4 with its index after the audio, as muxers write it without
/// `+faststart`, and longer than the probed prefix.
static LATE_M4A: LazyLock<Vec<u8>> =
    LazyLock::new(|| generated(200, &["-c:a", "aac", "-b:a", "128k", "-f", "ipod"]));

/// A generated sine wave, so ffprobe and ffmpeg have real audio to read.
fn generated(seconds: u32, format: &[&str]) -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out");
    let status = std::process::Command::new("ffmpeg")
        .args(["-nostdin", "-loglevel", "error", "-f", "lavfi", "-i"])
        .arg(format!("sine=duration={seconds}"))
        .args(format)
        .arg(&out)
        .status()
        .expect("ffmpeg must be on PATH for these tests");
    assert!(status.success());
    std::fs::read(out).unwrap()
}

async fn set_size(app: &App, id: i64) {
    let size = match id {
        4 | 7 => SHORT_WAV.len(),
        5 | 10 => LONG_WAV.len(),
        _ => LATE_M4A.len(),
    };
    let size = if id == 7 { size / 2 } else { size };
    app.exec(&format!("UPDATE tracks SET size = {size} WHERE id = {id}"))
        .await;
}

#[tokio::test]
async fn a_small_remote_file_is_probed_whole_and_plays_directly() {
    let (app, _remote, token) = library().await;
    set_size(&app, 4).await;
    let (status, body) = app
        .send(
            Req::post("/api/tracks/4/prepare")
                .token(&token)
                .json(serde_json::json!({"can_play": ["audio/wav"]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["url"], "/api/tracks/4/stream");
    let duration: Option<i64> = sqlx::query_scalar("SELECT duration_ms FROM tracks WHERE id = 4")
        .fetch_one(&app.db.sqlite().unwrap().read)
        .await
        .unwrap();
    assert!((1900..=2100).contains(&duration.unwrap()));
}

#[tokio::test]
async fn a_large_remote_file_is_probed_from_its_start_without_storing_a_duration() {
    let (app, remote, token) = library().await;
    set_size(&app, 5).await;
    let (status, body) = app
        .send(
            Req::post("/api/tracks/5/prepare")
                .token(&token)
                .json(serde_json::json!({"can_play": ["audio/wav"]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(remote.seen.lock().unwrap()[0]["range"], "bytes=0-2097151");
    let (probe, duration): (Option<String>, Option<i64>) =
        sqlx::query_as("SELECT probe, duration_ms FROM tracks WHERE id = 5")
            .fetch_one(&app.db.sqlite().unwrap().read)
            .await
            .unwrap();
    assert!(probe.unwrap().contains("pcm_"));
    assert_eq!(
        duration, None,
        "a partial probe only estimates the duration"
    );
}

#[tokio::test]
async fn a_remote_file_is_transcoded_from_a_full_copy() {
    let (app, _remote, token) = library().await;
    set_size(&app, 4).await;
    let (status, body) = app
        .send(
            Req::post("/api/tracks/4/prepare")
                .token(&token)
                .json(serde_json::json!({"can_play": []})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let task = wait_for_task(&app, &token, &body["task_id"]).await;
    assert_eq!(task["state"], "done", "{task}");
    let (status, headers, _) = app
        .send_raw(Req::get("/api/tracks/4/playable").token(&token))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "audio/mp4");
    let leftovers = std::fs::read_dir(app.data_dir().join("cache/tmp"))
        .unwrap()
        .count();
    assert_eq!(leftovers, 0, "the downloaded copy is removed");
}

#[tokio::test]
async fn a_remote_work_downloads_as_a_zip() {
    use std::io::Read;
    let (app, _remote, token) = library().await;
    app.exec("UPDATE tracks SET missing_at = 1 WHERE id IN (3, 4, 5, 6, 7, 8, 9)")
        .await;
    let (status, _, bytes) = app
        .send_raw(Req::get("/api/works/2/download").token(&token))
        .await;
    assert_eq!(status, StatusCode::OK);
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut mp3 = Vec::new();
    archive
        .by_name("01.mp3")
        .unwrap()
        .read_to_end(&mut mp3)
        .unwrap();
    assert_eq!(mp3, CONTENT);
    assert!(archive.by_name("01.lrc").is_ok());
}

#[tokio::test]
async fn an_mp4_with_its_index_at_the_end_is_probed_from_both_ends() {
    let (app, remote, token) = library().await;
    set_size(&app, 6).await;
    let (status, body) = app
        .send(
            Req::post("/api/tracks/6/prepare")
                .token(&token)
                .json(serde_json::json!({"can_play": ["audio/mp4"]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["url"], "/api/tracks/6/stream");
    // The analysis that prepare queues copies the whole file without a
    // range, possibly at the same time, so only the probe's reads count.
    let ranges: Vec<String> = remote
        .seen
        .lock()
        .unwrap()
        .iter()
        .filter_map(|h| h.get("range"))
        .map(|range| range.to_str().unwrap().to_owned())
        .collect();
    let size = LATE_M4A.len();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0], "bytes=0-2097151");
    assert!(ranges[1].ends_with(&format!("-{}", size - 1)), "{ranges:?}");
}

/// A file longer than its listing has changed. Copying only the listed
/// length would hand ffmpeg a truncated file and publish it as complete.
#[tokio::test]
async fn a_remote_file_longer_than_its_listing_is_refused() {
    let (app, _remote, token) = library().await;
    set_size(&app, 7).await;
    let (status, body) = app
        .send(
            Req::post("/api/tracks/7/prepare")
                .token(&token)
                .json(serde_json::json!({"can_play": []})),
        )
        .await;
    assert_error(
        status,
        &body,
        StatusCode::SERVICE_UNAVAILABLE,
        "storage_unavailable",
    );
    let probe: Option<String> = sqlx::query_scalar("SELECT probe FROM tracks WHERE id = 7")
        .fetch_one(&app.db.sqlite().unwrap().read)
        .await
        .unwrap();
    assert_eq!(probe, None);
}

async fn wait_for_task(app: &App, token: &str, id: &serde_json::Value) -> serde_json::Value {
    let uri = format!("/api/tasks/{id}");
    for _ in 0..200 {
        let (_, task) = app.send(Req::get(&uri).token(token)).await;
        if !matches!(task["state"].as_str(), Some("queued" | "running")) {
            return task;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("task {id} did not finish");
}

#[tokio::test]
async fn a_disconnect_frees_an_archive_slot_while_upstream_stalls() {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let (app, remote, token) = library().await;
    app.exec("UPDATE tracks SET missing_at = 1 WHERE id <> 1")
        .await;
    *remote.mode.lock().unwrap() = Upstream::Stalls;
    let request = || {
        Request::get("/api/works/2/download")
            .header("authorization", format!("Bearer {token}"))
            .extension(axum::extract::ConnectInfo(
                "192.0.2.10:40000".parse::<std::net::SocketAddr>().unwrap(),
            ))
            .body(Body::empty())
            .unwrap()
    };
    let mut held = Vec::new();
    for _ in 0..2 {
        held.push(app.router.clone().oneshot(request()).await.unwrap());
    }
    let full = app.router.clone().oneshot(request()).await.unwrap();
    assert_eq!(full.status(), StatusCode::TOO_MANY_REQUESTS);
    drop(held);
    // The tasks end on their next poll; wait for that rather than a fixed
    // time, and fail only if the slots never come back.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let next = app.router.clone().oneshot(request()).await.unwrap();
        if next.status() == StatusCode::OK {
            break;
        }
        assert_eq!(next.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(
            std::time::Instant::now() < deadline,
            "the slots were never released"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// A remote source is copied whole before it is transcoded, so a disk that
/// cannot hold the copy refuses the transcode up front.
#[tokio::test]
async fn a_remote_file_too_big_for_the_disk_is_refused_before_queueing() {
    let (app, _remote, token) = library().await;
    set_size(&app, 4).await;
    let (status, _) = app
        .send(
            Req::post("/api/tracks/4/prepare")
                .token(&token)
                .json(serde_json::json!({"can_play": ["audio/wav"]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    // Probed already, so prepare goes straight to the transcode decision.
    app.exec(&format!(
        "UPDATE tracks SET size = {HUGE}, probe = (SELECT probe FROM tracks WHERE id = 4)
         WHERE id = 8"
    ))
    .await;
    let (status, body) = app
        .send(
            Req::post("/api/tracks/8/prepare")
                .token(&token)
                .json(serde_json::json!({"can_play": []})),
        )
        .await;
    assert_error(
        status,
        &body,
        StatusCode::INSUFFICIENT_STORAGE,
        "insufficient_storage",
    );
}

/// A remote video is copied whole into cache/tmp, remuxed, and served from
/// the cache like a local one.
#[tokio::test]
async fn a_remote_video_is_remuxed_to_hls() {
    let (app, _remote, token) = library().await;
    app.exec(&format!(
        "UPDATE tracks SET size = {} WHERE id = 9",
        MOVIE_MKV.len()
    ))
    .await;
    let (status, body) = app
        .send(
            Req::post("/api/tracks/9/prepare")
                .token(&token)
                .json(serde_json::json!({"can_play": ["video/mp4"]})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let task = wait_for_task(&app, &token, &body["task_id"]).await;
    assert_eq!(task["state"], "done", "{task}");
    let (status, _, playlist) = app
        .send_raw(Req::get("/api/tracks/9/hls/index.m3u8").token(&token))
        .await;
    assert_eq!(status, StatusCode::OK);
    let playlist = String::from_utf8(playlist).unwrap();
    let first = playlist.lines().find(|l| l.contains("/seg-")).unwrap();
    let (status, headers, _) = app
        .send_raw(Req::get(&format!("/api/tracks/9/hls/{first}")))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "video/mp2t");
    let leftovers = std::fs::read_dir(app.data_dir().join("cache/tmp"))
        .unwrap()
        .count();
    assert_eq!(leftovers, 0, "the downloaded copy is removed");
}

/// Requests that fetched the whole remote file. The probe of `long.wav`
/// reads only its start, and the player's reads carry a range.
fn whole_fetches(remote: &Remote) -> usize {
    remote
        .seen
        .lock()
        .unwrap()
        .iter()
        .filter(|h| !h.contains_key("range"))
        .count()
}

/// Asks for the loudness until it is measured, following each 202.
async fn loudness_of(app: &App, token: &str, id: i64) -> f64 {
    let uri = format!("/api/tracks/{id}/loudness");
    for _ in 0..5 {
        let (status, body) = app.send(Req::get(&uri).token(token)).await;
        match status {
            StatusCode::OK => return body["integrated_lufs"].as_f64().unwrap(),
            StatusCode::ACCEPTED => {
                let task = wait_for_task(app, token, &body["task_id"]).await;
                assert_eq!(task["state"], "done", "{task}");
            }
            other => panic!("{uri} answered {other}: {body}"),
        }
    }
    panic!("{uri} was never measured");
}

#[tokio::test]
async fn a_first_play_downloads_a_remote_file_once_and_measures_its_loudness() {
    // Transcoded: the analysis reads the transcode's output.
    let (app, remote, token) = library().await;
    set_size(&app, 5).await;
    let (status, body) = app
        .send(
            Req::post("/api/tracks/5/prepare")
                .token(&token)
                .json(serde_json::json!({"can_play": []})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    // While the transcode runs, the loudness waits for it.
    let (status, waiting) = app
        .send(Req::get("/api/tracks/5/loudness").token(&token))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{waiting}");
    let task = wait_for_task(&app, &token, &body["task_id"]).await;
    assert_eq!(task["state"], "done", "{task}");
    let transcoded = loudness_of(&app, &token, 5).await;
    assert_eq!(
        whole_fetches(&remote),
        1,
        "one download for transcode and analysis"
    );

    // Direct: the analysis copies the file itself, after the player started.
    let (app, remote, token) = library().await;
    set_size(&app, 5).await;
    let (status, body) = app
        .send(
            Req::post("/api/tracks/5/prepare")
                .token(&token)
                .json(serde_json::json!({"can_play": ["audio/wav"]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _, _) = app
        .send_raw(
            Req::get("/api/tracks/5/stream")
                .token(&token)
                .header("range", "bytes=0-"),
        )
        .await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        whole_fetches(&remote),
        0,
        "the analysis waits for the player"
    );
    let direct = loudness_of(&app, &token, 5).await;
    assert_eq!(whole_fetches(&remote), 1, "one download for the analysis");

    // AAC keeps the loudness of the original.
    assert!(
        (transcoded - direct).abs() <= 0.5,
        "{transcoded} vs {direct}"
    );
}

#[tokio::test]
async fn remote_video_is_not_analysed() {
    let (app, remote, token) = library().await;
    let (status, body) = app
        .send(Req::get("/api/tracks/9/loudness").token(&token))
        .await;
    assert_error(status, &body, StatusCode::NOT_FOUND, "not_applicable");
    let (status, body) = app
        .send(Req::get("/api/tracks/9/waveform").token(&token))
        .await;
    assert_error(status, &body, StatusCode::NOT_FOUND, "not_applicable");
    assert_eq!(*remote.opens.lock().unwrap(), 0, "nothing was fetched");
    let queued: i64 = sqlx::query_scalar("SELECT count(*) FROM tasks WHERE kind = 'analysis'")
        .fetch_one(&app.db.sqlite().unwrap().read)
        .await
        .unwrap();
    assert_eq!(queued, 0);
}

#[tokio::test]
async fn a_remote_file_without_mtime_is_analysed_from_its_own_copy() {
    // Its transcode may be days old, while results derived from it would
    // be trusted for 7 days from now.
    let (app, remote, token) = library().await;
    app.exec(
        "INSERT INTO tracks (id, work_id, location_id, rel_path, display_path, sort_key, kind)
           VALUES (10, 2, 2, CAST('undated.wav' AS BLOB), 'undated.wav', 'j', 'audio')",
    )
    .await;
    set_size(&app, 10).await;
    let (status, body) = app
        .send(
            Req::post("/api/tracks/10/prepare")
                .token(&token)
                .json(serde_json::json!({"can_play": []})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let task = wait_for_task(&app, &token, &body["task_id"]).await;
    assert_eq!(task["state"], "done", "{task}");
    loudness_of(&app, &token, 10).await;
    assert_eq!(
        whole_fetches(&remote),
        2,
        "the analysis downloads the file itself"
    );
}

#[tokio::test]
async fn the_wait_before_a_remote_copy_is_not_part_of_the_job_timeout() {
    let (app, _remote, token) = library_with(|state, dir| {
        state.with_media(neo_server::http::Media {
            cache: neo_media::TranscodeCache::open(&dir.join("cache"), u64::MAX, 0).unwrap(),
            probe_slots: Arc::new(tokio::sync::Semaphore::new(4)),
            queue_limit: 32,
            job_timeout: std::time::Duration::from_secs(2),
            remote_analysis_delay: std::time::Duration::from_secs(3),
            video_transcode: true,
        })
    })
    .await;
    set_size(&app, 5).await;
    let (status, body) = app
        .send(
            Req::post("/api/tracks/5/prepare")
                .token(&token)
                .json(serde_json::json!({"can_play": ["audio/wav"]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    loudness_of(&app, &token, 5).await;
}

#[tokio::test]
async fn a_client_that_stops_reading_a_remote_file_frees_its_upstream() {
    // A paused player keeps its response open and reads nothing. Holding
    // the upstream for it would keep one of the root's transfer slots.
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let (app, remote, token) = library().await;
    *remote.mode.lock().unwrap() = Upstream::Endless;
    let held = app
        .router
        .clone()
        .oneshot(
            Request::get("/api/tracks/1/stream")
                .header("authorization", format!("Bearer {token}"))
                .extension(axum::extract::ConnectInfo(
                    "192.0.2.10:40000".parse::<std::net::SocketAddr>().unwrap(),
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(held.status(), StatusCode::OK);
    assert_eq!(*remote.live.lock().unwrap(), 1);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while *remote.live.lock().unwrap() > 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "the upstream was never dropped"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    // A client that reads again gets the buffered bytes and then an error,
    // not what looks like the whole file.
    use http_body_util::BodyExt;
    assert!(held.into_body().collect().await.is_err());
}

#[tokio::test]
async fn a_client_that_leaves_while_the_upstream_is_quiet_frees_it() {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let (app, remote, token) = library().await;
    *remote.mode.lock().unwrap() = Upstream::Stalls;
    let held = app
        .router
        .clone()
        .oneshot(
            Request::get("/api/tracks/1/stream")
                .header("authorization", format!("Bearer {token}"))
                .extension(axum::extract::ConnectInfo(
                    "192.0.2.10:40000".parse::<std::net::SocketAddr>().unwrap(),
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(*remote.live.lock().unwrap(), 1);
    drop(held);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while *remote.live.lock().unwrap() > 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "the upstream outlived the client"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}
