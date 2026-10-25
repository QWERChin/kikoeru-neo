//! The M6 completion conditions (book/src/roadmap.md, M6 【联调】) against
//! the real `serve` binary: every video kind on the support list of
//! book/src/media.md, from a local root and from a fake OpenList, through
//! prepare, the playlist and Range requests on segments, plus token expiry,
//! cancel, restart and the cache budget. The samples are made with ffmpeg
//! when the test runs; nothing binary is committed.

mod process;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use axum::{
    Json, Router,
    extract::{Path as UrlPath, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use process::{Server, create_admin};
use serde_json::{Value, json};
use tokio::{process::Command, time::sleep};

/// The browser the tests pretend to be: it plays mp4, not matroska or avi.
fn browser() -> Value {
    json!({"can_play": ["video/mp4", "audio/mp4", "audio/mpeg"]})
}

/// A sample on the support list, and what the backend should do with it.
struct Sample {
    name: &'static str,
    /// ffmpeg output options after the test pattern and tone inputs.
    encode: &'static [&'static str],
    expect: Expect,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Expect {
    /// Served as it is through `stream`.
    Direct,
    /// Published as HLS, by remuxing or transcoding.
    Hls,
}

const SAMPLES: &[Sample] = &[
    // The main kind in the library: H.264 + AAC in mp4 plays directly.
    Sample {
        name: "direct.mp4",
        encode: &["-c:v", "libx264", "-g", "25", "-c:a", "aac"],
        expect: Expect::Direct,
    },
    // H.264 + AAC in another container is remuxed into HLS.
    Sample {
        name: "remux.mkv",
        encode: &["-c:v", "libx264", "-g", "25", "-c:a", "aac"],
        expect: Expect::Hls,
    },
    // Other codecs are transcoded.
    Sample {
        name: "mpeg4.mkv",
        encode: &["-c:v", "mpeg4", "-c:a", "libmp3lame"],
        expect: Expect::Hls,
    },
    // AVI stores no presentation timestamps, and x264's B-frames need
    // them: remuxing relies on regenerated timestamps.
    Sample {
        name: "remux.avi",
        encode: &["-c:v", "libx264", "-g", "25", "-bf", "2", "-c:a", "aac"],
        expect: Expect::Hls,
    },
    Sample {
        name: "xvid.avi",
        encode: &["-c:v", "mpeg4", "-bf", "2", "-c:a", "libmp3lame"],
        expect: Expect::Hls,
    },
];

/// A test picture with a tone.
async fn make_movie(path: &Path, seconds: u32, size: &str, encode: &[&str]) {
    let status = Command::new("ffmpeg")
        .args(["-nostdin", "-hide_banner", "-loglevel", "error"])
        .args(["-f", "lavfi", "-i"])
        .arg(format!("testsrc=size={size}:rate=25:duration={seconds}"))
        .args(["-f", "lavfi", "-i"])
        .arg(format!("sine=duration={seconds}"))
        .args(encode)
        .arg("-y")
        .arg(path)
        .status()
        .await
        .expect("ffmpeg must be on PATH for these tests");
    assert!(status.success(), "ffmpeg could not make {}", path.display());
}

/// A stand-in for OpenList that serves a local directory: login, listing,
/// file info and links, and the files themselves with Range.
struct FakeOpenList {
    dir: PathBuf,
    base: std::sync::OnceLock<String>,
}

impl FakeOpenList {
    async fn start(dir: PathBuf) -> Arc<Self> {
        let fake = Arc::new(Self {
            dir,
            base: std::sync::OnceLock::new(),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        fake.base
            .set(format!("http://{}", listener.local_addr().unwrap()))
            .unwrap();
        let app = Router::new()
            .route("/api/auth/login", post(ol_login))
            .route("/api/fs/list", post(ol_list))
            .route("/api/fs/get", post(ol_get))
            .route("/api/fs/link", post(ol_link))
            .route("/files/{*path}", get(ol_file))
            .with_state(fake.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        fake
    }

    fn local(&self, path: &str) -> PathBuf {
        self.dir.join(path.trim_start_matches('/'))
    }

    fn entry(&self, path: &Path) -> Option<Value> {
        let meta = std::fs::metadata(path).ok()?;
        Some(json!({
            "name": path.file_name()?.to_str()?, "size": meta.len(), "is_dir": meta.is_dir(),
            "modified": "2024-05-01T20:00:00+08:00", "created": "0001-01-01T00:00:00Z",
            "sign": "", "thumb": "", "type": 0, "hashinfo": "null", "hash_info": null,
        }))
    }
}

const OL_USER: &str = "reader";
const OL_PASSWORD: &str = "synthetic-password";

fn ol_ok(data: Value) -> Response {
    Json(json!({"code": 200, "message": "success", "data": data})).into_response()
}

fn ol_code(code: i64, message: &str) -> Response {
    Json(json!({"code": code, "message": message, "data": null})).into_response()
}

async fn ol_login(Json(body): Json<Value>) -> Response {
    if body["username"] != OL_USER || body["password"] != OL_PASSWORD {
        return ol_code(400, "password is incorrect");
    }
    ol_ok(json!({"token": "token-1"}))
}

async fn ol_list(State(fake): State<Arc<FakeOpenList>>, Json(body): Json<Value>) -> Response {
    let path = body["path"].as_str().unwrap_or_default();
    let Ok(read) = std::fs::read_dir(fake.local(path)) else {
        return ol_code(500, "failed get objs: object not found");
    };
    let mut content: Vec<Value> = read.filter_map(|e| fake.entry(&e.ok()?.path())).collect();
    content.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    let total = content.len();
    ol_ok(
        json!({"content": content, "total": total, "readme": "", "header": "",
                 "write": false, "provider": "Fake"}),
    )
}

async fn ol_get(State(fake): State<Arc<FakeOpenList>>, Json(body): Json<Value>) -> Response {
    let path = body["path"].as_str().unwrap_or_default();
    let Some(mut data) = fake.entry(&fake.local(path)) else {
        return ol_code(500, "failed get obj: object not found");
    };
    data["raw_url"] = json!(format!("{}/files{path}", fake.base.get().unwrap()));
    ol_ok(data)
}

async fn ol_link(State(fake): State<Arc<FakeOpenList>>, Json(body): Json<Value>) -> Response {
    let path = body["path"].as_str().unwrap_or_default();
    if !fake.local(path).is_file() {
        return ol_code(500, "failed link: object not found");
    }
    ol_ok(json!({
        "url": format!("{}/files{path}", fake.base.get().unwrap()),
        "header": {}, "Expiration": null,
        "concurrency": 0, "part_size": 0, "content_length": 0,
    }))
}

async fn ol_file(
    State(fake): State<Arc<FakeOpenList>>,
    UrlPath(path): UrlPath<String>,
    headers: HeaderMap,
) -> Response {
    let Ok(body) = std::fs::read(fake.local(&path)) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let len = body.len();
    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|r| r.strip_prefix("bytes="))
        .and_then(|r| r.split_once('-'))
        .and_then(|(start, end)| {
            let start: usize = start.parse().ok()?;
            let end = if end.is_empty() {
                len - 1
            } else {
                end.parse::<usize>().ok()?.min(len - 1)
            };
            (start <= end).then_some((start, end))
        });
    match range {
        Some((start, end)) => (
            StatusCode::PARTIAL_CONTENT,
            [(header::CONTENT_RANGE, format!("bytes {start}-{end}/{len}"))],
            body[start..=end].to_vec(),
        )
            .into_response(),
        None => body.into_response(),
    }
}

struct Setup {
    dir: tempfile::TempDir,
    server: Server,
    admin: String,
    _openlist: Option<Arc<FakeOpenList>>,
}

/// Which root the samples live in.
#[derive(Clone, Copy, PartialEq)]
enum RootKind {
    Local,
    OpenList,
}

impl Setup {
    /// Makes the `movies` in a work directory, serves them from the chosen
    /// kind of root, and scans it. `cache` adds a `[cache]` section.
    async fn new(kind: RootKind, movies: &[(&str, u32, &str, &[&str])], cache: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let library = dir.path().join("library");
        let work = library.join("RJ01000001");
        std::fs::create_dir_all(&work).unwrap();
        for (name, seconds, size, encode) in movies {
            make_movie(&work.join(name), *seconds, size, encode).await;
        }
        let (root, openlist) = match kind {
            RootKind::Local => (
                format!(
                    "[[roots]]\ntype = \"local\"\nid = \"lib\"\ndisplay_name = \"Library\"\npath = \"{}\"\n",
                    library.display()
                ),
                None,
            ),
            RootKind::OpenList => {
                let fake = FakeOpenList::start(library.clone()).await;
                let password = dir.path().join("openlist.password");
                std::fs::write(&password, OL_PASSWORD).unwrap();
                (
                    format!(
                        "[[roots]]\ntype = \"openlist\"\nid = \"lib\"\ndisplay_name = \"Cloud\"\n\
                         url = \"{}\"\nbase_path = \"/\"\nusername = \"{OL_USER}\"\n\
                         password_file = \"{}\"\nlink_rate = 100.0\n",
                        fake.base.get().unwrap(),
                        password.display()
                    ),
                    Some(fake),
                )
            }
        };
        let data = dir.path().join("data");
        std::fs::create_dir(&data).unwrap();
        std::fs::write(
            data.join("config.toml"),
            format!(
                "listen = \"127.0.0.1:0\"\n{root}\n[metadata]\nsource = \"none\"\n\n\
                 [cache]\nmin_free_bytes = 0\n{cache}\n"
            ),
        )
        .unwrap();
        create_admin(&data, "admin").await;
        let server = Server::start(&data).await;
        let admin = server.login("admin", "initial password").await;
        let setup = Self {
            dir,
            server,
            admin,
            _openlist: openlist,
        };
        let (status, body) = setup.post("/api/admin/scan", json!({"root": "lib"})).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        let scan = setup
            .until_ended(body["task_ids"][0].as_i64().unwrap())
            .await;
        assert_eq!(scan["state"], "done", "{scan}");
        setup
    }

    fn data(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    async fn restart(&mut self) {
        self.server.crash().await;
        self.server = Server::start(&self.data()).await;
        self.admin = self.server.login("admin", "initial password").await;
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        self.server
            .request("GET", path, Some(&self.admin), None)
            .await
    }

    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.server
            .request("POST", path, Some(&self.admin), Some(body))
            .await
    }

    async fn track(&self, name: &str) -> i64 {
        let (_, works) = self.get("/api/works?page=1").await;
        let work = works["items"][0]["id"].as_i64().unwrap();
        let (_, detail) = self.get(&format!("/api/works/{work}")).await;
        detail["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["path"].as_str().is_some_and(|p| p.ends_with(name)))
            .unwrap_or_else(|| panic!("no track {name}: {detail}"))["id"]
            .as_i64()
            .unwrap()
    }

    async fn until_ended(&self, id: i64) -> Value {
        for _ in 0..1200 {
            let (_, task) = self.get(&format!("/api/tasks/{id}")).await;
            if !matches!(task["state"].as_str(), Some("queued" | "running")) {
                return task;
            }
            sleep(Duration::from_millis(100)).await;
        }
        panic!("task {id} did not end");
    }

    async fn media_token(&self) -> String {
        let (status, body) = self.post("/api/auth/media-token", json!({})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["token"].as_str().unwrap().to_owned()
    }

    /// Prepares a track and waits for any task. Returns the final `url`.
    async fn prepare(&self, track: i64, body: Value) -> Result<String, Value> {
        let (status, answer) = self
            .post(&format!("/api/tracks/{track}/prepare"), body)
            .await;
        match status {
            StatusCode::OK => Ok(answer["url"].as_str().unwrap().to_owned()),
            StatusCode::ACCEPTED => {
                let task = self.until_ended(answer["task_id"].as_i64().unwrap()).await;
                if task["state"] == "done" {
                    Ok(task["result"]["url"].as_str().unwrap().to_owned())
                } else {
                    Err(task)
                }
            }
            _ => Err(answer),
        }
    }

    /// GET with an optional Range, no auth header: the URL carries the
    /// media token, as the player's requests do.
    async fn fetch(
        &self,
        path: &str,
        range: Option<&str>,
    ) -> (u16, HashMap<String, String>, Vec<u8>) {
        let body = tempfile::NamedTempFile::new().unwrap();
        let heads = tempfile::NamedTempFile::new().unwrap();
        let mut curl = Command::new("curl");
        curl.args([
            "--disable",
            "--silent",
            "--noproxy",
            "*",
            "--max-time",
            "20",
        ])
        .args(["--write-out", "%{http_code}", "--output"])
        .arg(body.path())
        .arg("--dump-header")
        .arg(heads.path());
        if let Some(range) = range {
            curl.args(["--header", &format!("Range: {range}")]);
        }
        let out = curl
            .arg(self.server.url(path))
            .stdout(Stdio::piped())
            .output()
            .await
            .unwrap();
        let status: u16 = String::from_utf8(out.stdout)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let headers = std::fs::read_to_string(heads.path())
            .unwrap()
            .lines()
            .filter_map(|l| l.split_once(": "))
            .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_owned()))
            .collect();
        (status, headers, std::fs::read(body.path()).unwrap())
    }

    /// The segment URIs of a playlist, resolved against its directory.
    async fn segments(&self, track: i64, token: &str) -> Vec<String> {
        let (status, headers, playlist) = self
            .fetch(
                &format!("/api/tracks/{track}/hls/index.m3u8?token={token}"),
                None,
            )
            .await;
        assert_eq!(status, 200);
        assert_eq!(headers["content-type"], "application/vnd.apple.mpegurl");
        let playlist = String::from_utf8(playlist).unwrap();
        assert!(playlist.contains("#EXT-X-ENDLIST"), "{playlist}");
        // Broken timestamps show up as segment durations that do not add
        // up to the 8 s every sample lasts.
        let total: f64 = playlist
            .lines()
            .filter_map(|l| l.strip_prefix("#EXTINF:"))
            .map(|d| d.trim_end_matches(',').parse::<f64>().unwrap())
            .sum();
        assert!((7.5..8.6).contains(&total), "{total} s: {playlist}");
        playlist
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|uri| format!("/api/tracks/{track}/hls/{uri}"))
            .collect()
    }

    fn leftovers(&self) -> Vec<PathBuf> {
        std::fs::read_dir(self.data().join("cache/tmp"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect()
    }

    /// ffmpeg or ffprobe processes the server started and has not reaped.
    fn children(&self) -> usize {
        let pid = self.server.pid().to_string();
        std::fs::read_dir("/proc")
            .unwrap()
            .filter_map(|e| std::fs::read_to_string(e.ok()?.path().join("stat")).ok())
            .filter(|stat| {
                let Some((head, rest)) = stat.rsplit_once(") ") else {
                    return false;
                };
                (head.ends_with("(ffmpeg") || head.ends_with("(ffprobe"))
                    && rest.split_whitespace().nth(1) == Some(pid.as_str())
            })
            .count()
    }
}

/// Plays every sample the way the player would and checks each answer.
async fn play_the_matrix(kind: RootKind) {
    let movies: Vec<(&str, u32, &str, &[&str])> = SAMPLES
        .iter()
        .map(|s| (s.name, 8, "320x240", s.encode))
        .collect();
    let mut setup = Setup::new(kind, &movies, "").await;
    let token = setup.media_token().await;
    for sample in SAMPLES {
        let track = setup.track(sample.name).await;
        let url = setup
            .prepare(track, browser())
            .await
            .unwrap_or_else(|e| panic!("{}: {e}", sample.name));
        match sample.expect {
            Expect::Direct => {
                assert_eq!(
                    url,
                    format!("/api/tracks/{track}/stream"),
                    "{}",
                    sample.name
                );
                let (status, headers, part) = setup
                    .fetch(&format!("{url}?token={token}"), Some("bytes=0-99"))
                    .await;
                assert_eq!(status, 206, "{}", sample.name);
                assert_eq!(part.len(), 100);
                assert!(headers["content-range"].starts_with("bytes 0-99/"));
            }
            Expect::Hls => {
                assert_eq!(
                    url,
                    format!("/api/tracks/{track}/hls/index.m3u8"),
                    "{}",
                    sample.name
                );
                let segments = setup.segments(track, &token).await;
                assert_eq!(segments.len(), 2, "{}: 8 s in 6 s segments", sample.name);
                for segment in &segments {
                    let (status, _, whole) = setup.fetch(segment, None).await;
                    assert_eq!(status, 200, "{}: {segment}", sample.name);
                    assert!(
                        whole.len() > 188 && whole[0] == 0x47,
                        "{}: MPEG-TS",
                        sample.name
                    );
                    // A seek inside a segment.
                    let (status, headers, part) = setup.fetch(segment, Some("bytes=188-375")).await;
                    assert_eq!(status, 206, "{}: {segment}", sample.name);
                    assert_eq!(part, whole[188..376], "{}", sample.name);
                    assert_eq!(
                        headers["content-range"],
                        format!("bytes 188-375/{}", whole.len())
                    );
                }
            }
        }
    }
    assert_eq!(setup.leftovers(), Vec::<PathBuf>::new());
    setup.server.crash().await;
}

#[tokio::test]
async fn every_sample_plays_from_a_local_root() {
    play_the_matrix(RootKind::Local).await;
}

#[tokio::test]
async fn every_sample_plays_from_an_openlist_root() {
    play_the_matrix(RootKind::OpenList).await;
}

#[tokio::test]
async fn an_expired_media_token_gets_401_on_a_segment_until_refreshed() {
    let h264: &[&str] = &["-c:v", "libx264", "-g", "25", "-c:a", "aac"];
    let mut setup = Setup::new(RootKind::Local, &[("remux.mkv", 8, "320x240", h264)], "").await;
    let track = setup.track("remux.mkv").await;
    setup.prepare(track, browser()).await.unwrap();

    // A media token issued seven hours ago, past its six-hour life, signed
    // with the server's own key.
    let key = std::fs::read(setup.data().join("secrets/jwt.key")).unwrap();
    let issued = neo_server::token::now_secs() - 7 * 3600;
    let (_, me) = setup.get("/api/auth/me").await;
    let db = sqlx::sqlite::SqlitePoolOptions::new()
        .connect(&format!(
            "sqlite://{}",
            setup.data().join("neo.sqlite3").display()
        ))
        .await
        .unwrap();
    let version: i64 = sqlx::query_scalar("SELECT token_version FROM users WHERE id = ?")
        .bind(me["id"].as_i64().unwrap())
        .fetch_one(&db)
        .await
        .unwrap();
    let expired = neo_server::token::Tokens::new(&key)
        .issue(
            me["id"].as_i64().unwrap(),
            version,
            neo_server::token::Audience::Media,
            issued,
        )
        .unwrap()
        .token;

    let fresh = setup.media_token().await;
    let segment = setup.segments(track, &fresh).await.remove(0);
    let stale = segment.replace(&fresh, &expired);
    assert_eq!(setup.fetch(&stale, None).await.0, 401);
    // What the player does next: a new media token and the playlist again.
    let renewed = setup.media_token().await;
    let segment = setup.segments(track, &renewed).await.remove(0);
    assert_eq!(setup.fetch(&segment, Some("bytes=0-187")).await.0, 206);
    setup.server.crash().await;
}

/// A transcode long enough to still be running when the test acts on it.
const LONG: &[&str] = &["-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac"];

async fn start_long_transcode(setup: &Setup) -> (i64, i64) {
    let track = setup.track("long.mkv").await;
    let (status, body) = setup
        .post(
            &format!("/api/tracks/{track}/prepare"),
            json!({"can_play": [], "force_fallback": true}),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let task = body["task_id"].as_i64().unwrap();
    // Running, with ffmpeg already writing segments: the sample is two
    // minutes of 720p, far more than the server can encode before the test
    // acts on the very next request.
    for _ in 0..400 {
        let (_, state) = setup.get(&format!("/api/tasks/{task}")).await;
        assert_ne!(state["state"], "done", "the sample was too short");
        let writing = std::fs::read_dir(setup.data().join("cache/tmp"))
            .unwrap()
            .filter_map(Result::ok)
            .any(|e| e.path().is_dir() && dir_size(&e.path()) > 0);
        if state["state"] == "running" && writing {
            return (track, task);
        }
        sleep(Duration::from_millis(50)).await;
    }
    panic!("the transcode never started writing");
}

#[tokio::test]
async fn cancelling_a_transcode_leaves_no_files_or_processes() {
    let mut setup = Setup::new(RootKind::Local, &[("long.mkv", 120, "1280x720", LONG)], "").await;
    let (_, task) = start_long_transcode(&setup).await;
    assert_eq!(
        setup.get(&format!("/api/tasks/{task}")).await.1["state"],
        "running"
    );
    let (status, _) = setup
        .post(&format!("/api/tasks/{task}/cancel"), json!({}))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(setup.until_ended(task).await["state"], "cancelled");
    assert_eq!(setup.leftovers(), Vec::<PathBuf>::new());
    assert_eq!(setup.children(), 0);
    setup.server.crash().await;
}

#[tokio::test]
async fn a_restart_interrupts_a_transcode_and_prepare_starts_it_again() {
    let mut setup = Setup::new(RootKind::Local, &[("long.mkv", 30, "640x360", LONG)], "").await;
    let (track, task) = start_long_transcode(&setup).await;
    setup.restart().await;
    let (_, old) = setup.get(&format!("/api/tasks/{task}")).await;
    assert_eq!(old["state"], "interrupted", "{old}");
    assert_eq!(
        setup.leftovers(),
        Vec::<PathBuf>::new(),
        "startup clears cache/tmp"
    );
    let url = setup
        .prepare(track, json!({"can_play": [], "force_fallback": true}))
        .await
        .unwrap();
    assert_eq!(url, format!("/api/tracks/{track}/hls/index.m3u8"));
    setup.server.crash().await;
}

fn dir_size(path: &Path) -> u64 {
    std::fs::read_dir(path)
        .map(|entries| {
            entries
                .map(|e| e.unwrap())
                .map(|e| {
                    let meta = e.metadata().unwrap();
                    if meta.is_dir() {
                        dir_size(&e.path())
                    } else {
                        meta.len()
                    }
                })
                .sum()
        })
        .unwrap_or(0)
}

fn hls_sets(data: &Path) -> Vec<PathBuf> {
    let mut sets: Vec<PathBuf> = std::fs::read_dir(data.join("cache/transcode"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "hls"))
        .collect();
    sets.sort();
    sets
}

#[tokio::test]
async fn the_cache_budget_evicts_the_least_recently_used_hls_set() {
    let h264: &[&str] = &["-c:v", "libx264", "-g", "25", "-c:a", "aac"];
    let mut setup = Setup::new(
        RootKind::Local,
        &[
            ("first.mkv", 8, "320x240", h264),
            ("second.mkv", 8, "320x240", h264),
        ],
        "",
    )
    .await;
    let first = setup.track("first.mkv").await;
    let second = setup.track("second.mkv").await;
    setup.prepare(first, browser()).await.unwrap();
    let sets = hls_sets(&setup.data());
    assert_eq!(sets.len(), 1);
    let one = dir_size(&sets[0]);

    // Room for one set and a half: publishing the second must evict the
    // first, which nobody is reading.
    std::fs::write(
        setup.data().join("config.toml"),
        std::fs::read_to_string(setup.data().join("config.toml"))
            .unwrap()
            .replace(
                "min_free_bytes = 0\n",
                &format!("min_free_bytes = 0\nmax_bytes = {}\n", one * 3 / 2),
            ),
    )
    .unwrap();
    setup.restart().await;
    setup.prepare(second, browser()).await.unwrap();
    for _ in 0..100 {
        if hls_sets(&setup.data()).len() == 1 {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(hls_sets(&setup.data()).len(), 1, "one set fits the budget");
    let token = setup.media_token().await;
    let (status, _, _) = setup
        .fetch(
            &format!("/api/tracks/{first}/hls/index.m3u8?token={token}"),
            None,
        )
        .await;
    assert_eq!(status, 409, "the evicted set is gone");
    assert_eq!(setup.segments(second, &token).await.len(), 2);
    // Preparing the first again generates it anew.
    setup.prepare(first, browser()).await.unwrap();
    assert_eq!(setup.segments(first, &token).await.len(), 2);
    setup.server.crash().await;
}
