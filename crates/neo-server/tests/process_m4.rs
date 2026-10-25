//! The M4 completion conditions (book/src/roadmap.md, M4 【联调】) against
//! the real `serve` binary and a local fake DLsite that serves the fixtures
//! in tests/fixtures/dlsite.

mod process;

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    extract::{Path as UrlPath, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use process::{Server, create_admin};
use serde_json::{Value, json};
use tokio::{io::AsyncBufReadExt, process::Command, time::sleep};

const CODE: &str = "RJ01000001";
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/dlsite");

/// A stand-in for DLsite. `down` answers every request with 500, and
/// `delay_ms` holds every answer back, so a scan sits in its metadata
/// stage for as long as a test needs.
#[derive(Default)]
struct Fake {
    base: std::sync::OnceLock<String>,
    down: AtomicBool,
    delay_ms: AtomicU64,
}

impl Fake {
    async fn start() -> Arc<Self> {
        let fake = Arc::new(Self::default());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address: SocketAddr = listener.local_addr().unwrap();
        fake.base.set(format!("http://{address}/")).unwrap();
        let app = Router::new()
            .route("/maniax/api/=/product.json", get(product))
            .route("/maniax/product/info/ajax", get(info))
            .route("/images/{file}", get(image))
            .with_state(fake.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        fake
    }

    fn base(&self) -> &str {
        self.base.get().unwrap()
    }

    async fn gate(&self) -> Option<Response> {
        let delay = self.delay_ms.load(Ordering::SeqCst);
        if delay > 0 {
            sleep(Duration::from_millis(delay)).await;
        }
        self.down
            .load(Ordering::SeqCst)
            .then(|| StatusCode::INTERNAL_SERVER_ERROR.into_response())
    }
}

fn fixture(name: &str) -> Option<String> {
    std::fs::read_to_string(Path::new(FIXTURES).join(name)).ok()
}

fn json_response(body: String) -> Response {
    ([(header::CONTENT_TYPE, "application/json")], body).into_response()
}

async fn product(
    State(fake): State<Arc<Fake>>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    if let Some(r) = fake.gate().await {
        return r;
    }
    let code = q.get("workno").cloned().unwrap_or_default();
    match fixture(&format!("{code}.product.json")) {
        Some(body) => json_response(body.replace("{base}", fake.base())),
        None => json_response("[]".to_owned()),
    }
}

async fn info(
    State(fake): State<Arc<Fake>>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    if let Some(r) = fake.gate().await {
        return r;
    }
    let code = q.get("product_id").cloned().unwrap_or_default();
    json_response(fixture(&format!("{code}.info.json")).unwrap_or_else(|| "{}".to_owned()))
}

async fn image(State(fake): State<Arc<Fake>>, UrlPath(_file): UrlPath<String>) -> Response {
    if let Some(r) = fake.gate().await {
        return r;
    }
    let bytes = std::fs::read(Path::new(FIXTURES).join("cover.jpg")).unwrap();
    ([(header::CONTENT_TYPE, "image/jpeg")], bytes).into_response()
}

/// One second of 8 kHz mono silence, enough for ffprobe.
fn wav() -> Vec<u8> {
    let data = vec![0u8; 16_000];
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&8_000u32.to_le_bytes());
    out.extend_from_slice(&16_000u32.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    out
}

struct Setup {
    dir: tempfile::TempDir,
    fake: Arc<Fake>,
    server: Server,
    token: String,
}

impl Setup {
    /// A data directory with one local root holding one new work, a server
    /// pointed at the fake, and an admin token.
    async fn new() -> Self {
        Self::start(None).await
    }

    /// As [`Setup::new`], with DLsite at `dlsite_url` instead of the fake.
    async fn start(dlsite_url: Option<&str>) -> Self {
        let fake = Fake::start().await;
        let dir = tempfile::tempdir().unwrap();
        let library = dir.path().join("library");
        let work = library.join(format!("示例目录/{CODE}"));
        std::fs::create_dir_all(&work).unwrap();
        for n in 1..=3 {
            std::fs::write(work.join(format!("0{n}.wav")), wav()).unwrap();
        }
        std::fs::write(
            dir.path().join("config.toml"),
            format!(
                "listen = \"127.0.0.1:0\"\n\n\
                 [[roots]]\ntype = \"local\"\nid = \"lib\"\ndisplay_name = \"Library\"\npath = \"{}\"\n\n\
                 [metadata]\ndlsite_url = \"{}\"\n",
                library.display(),
                dlsite_url.unwrap_or(fake.base())
            ),
        )
        .unwrap();
        create_admin(dir.path(), "admin").await;
        let server = Server::start(dir.path()).await;
        let token = server.login("admin", "initial password").await;
        Self {
            dir,
            fake,
            server,
            token,
        }
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        self.server
            .request("GET", path, Some(&self.token), None)
            .await
    }

    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.server
            .request("POST", path, Some(&self.token), Some(body))
            .await
    }

    async fn scan(&self) -> i64 {
        let (status, body) = self.post("/api/admin/scan", json!({"root": "lib"})).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        body["task_ids"][0].as_i64().unwrap()
    }

    async fn refresh(&self, work: i64, force: bool) -> i64 {
        let (status, body) = self
            .post(
                &format!("/api/works/{work}/refresh?force={force}"),
                json!({}),
            )
            .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        body["task_id"].as_i64().unwrap()
    }

    async fn task(&self, id: i64) -> Value {
        self.get(&format!("/api/tasks/{id}")).await.1
    }

    async fn until_ended(&self, id: i64) -> Value {
        for _ in 0..600 {
            let task = self.task(id).await;
            if !matches!(task["state"].as_str(), Some("queued" | "running")) {
                return task;
            }
            sleep(Duration::from_millis(100)).await;
        }
        panic!("task {id} did not end: {}", self.task(id).await);
    }

    /// Every task, ended or not.
    async fn tasks(&self) -> Vec<Value> {
        let (status, body) = self.get("/api/tasks?page_size=100").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["items"].as_array().unwrap().clone()
    }

    async fn until_all_ended(&self) {
        for _ in 0..600 {
            if self
                .tasks()
                .await
                .iter()
                .all(|t| !matches!(t["state"].as_str(), Some("queued" | "running")))
            {
                return;
            }
            sleep(Duration::from_millis(100)).await;
        }
        panic!("tasks did not end: {:?}", self.tasks().await);
    }

    async fn work_id(&self) -> i64 {
        for _ in 0..200 {
            let (_, body) = self.get("/api/works?page=1").await;
            if let Some(w) = body["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|w| w["code"] == CODE)
            {
                return w["id"].as_i64().unwrap();
            }
            sleep(Duration::from_millis(50)).await;
        }
        panic!("{CODE} was not added");
    }

    async fn detail(&self, work: i64) -> Value {
        let (status, body) = self.get(&format!("/api/works/{work}")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    /// The scan's counts. The API only returns a result for transcodes, so
    /// this reads the stored row.
    async fn result(&self, task: i64) -> Value {
        let result: String = sqlx::query_scalar("SELECT result FROM tasks WHERE id = ?")
            .bind(task)
            .fetch_one(&self.db().await)
            .await
            .unwrap();
        serde_json::from_str(&result).unwrap()
    }

    async fn db(&self) -> sqlx::SqlitePool {
        sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&format!(
                "sqlite://{}",
                self.dir.path().join("neo.sqlite3").display()
            ))
            .await
            .unwrap()
    }

    fn data(&self, path: &str) -> PathBuf {
        self.dir.path().join(path)
    }

    /// ffprobe processes the server started and has not reaped.
    fn ffprobe_children(&self) -> usize {
        let pid = self.server.pid().to_string();
        std::fs::read_dir("/proc")
            .unwrap()
            .filter_map(|e| std::fs::read_to_string(e.ok()?.path().join("stat")).ok())
            .filter(|stat| {
                // "pid (comm) state ppid ...": comm may hold spaces, so split
                // after its closing parenthesis.
                let Some((head, rest)) = stat.rsplit_once(") ") else {
                    return false;
                };
                let ppid = rest.split_whitespace().nth(1);
                head.ends_with("(ffprobe") && ppid == Some(pid.as_str())
            })
            .count()
    }

    fn leftover_temp_files(&self) -> Vec<PathBuf> {
        let mut left = Vec::new();
        for dir in ["cache/tmp", "covers/.swap"] {
            if let Ok(entries) = std::fs::read_dir(self.data(dir)) {
                left.extend(entries.map(|e| e.unwrap().path()));
            }
        }
        left
    }
}

fn names(list: &Value) -> Vec<String> {
    let mut names: Vec<String> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["name"].as_str().unwrap().to_owned())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn a_new_work_gets_metadata_tags_and_a_cover_from_a_scan() {
    let mut s = Setup::new().await;
    let scan = s.scan().await;
    let task = s.until_ended(scan).await;
    assert_eq!(task["state"], "done", "{task}");
    assert_eq!(s.result(scan).await["metadata_fetched"], 1);

    let work = s.work_id().await;
    let detail = s.detail(work).await;
    assert_eq!(detail["title"], "示例作品一");
    assert_eq!(detail["circle"]["name"], "示例社团");
    assert_eq!(detail["circle"]["id"], 10001);
    assert_eq!(detail["release_date"], "2024-05-01");
    assert_eq!(detail["nsfw"], true);
    assert_eq!(names(&detail["vas"]), ["示例声优乙", "示例声优甲"]);
    assert_eq!(names(&detail["tags"]), ["ASMR", "School / Academy"]);
    assert_eq!(detail["has_cover"], true);
    assert_eq!(detail["cover_version"], 1);
    let (status, cover) = s
        .server
        .get_bytes(&format!("/api/works/{work}/cover"), &s.token)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        cover,
        std::fs::read(Path::new(FIXTURES).join("cover.jpg")).unwrap()
    );
    s.server.crash().await;
}

#[tokio::test]
async fn a_proxy_saved_in_the_settings_is_used_after_a_restart() {
    // DLsite's host never resolves, so metadata only arrives through the
    // fake, which also answers as an HTTP proxy.
    let mut s = Setup::start(Some("http://dlsite.invalid/")).await;
    let (status, body) = s
        .server
        .request(
            "PUT",
            "/api/admin/settings",
            Some(&s.token),
            Some(json!({"metadata_proxy": s.fake.base()})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    s.server.crash().await;
    s.server = Server::start(s.dir.path()).await;
    s.token = s.server.login("admin", "initial password").await;

    let scan = s.scan().await;
    let task = s.until_ended(scan).await;

    assert_eq!(task["state"], "done", "{task}");
    assert_eq!(s.result(scan).await["metadata_fetched"], 1);
    let work = s.work_id().await;
    assert_eq!(s.detail(work).await["title"], "示例作品一");
    s.server.crash().await;
}

#[tokio::test]
async fn an_outage_still_adds_the_work_and_a_refresh_fills_it_in_later() {
    let mut s = Setup::new().await;
    s.fake.down.store(true, Ordering::SeqCst);
    let scan = s.scan().await;
    let task = s.until_ended(scan).await;
    assert_eq!(task["state"], "done", "{task}");
    assert_eq!(s.result(scan).await["metadata_unavailable"], 1);

    let work = s.work_id().await;
    let detail = s.detail(work).await;
    assert_eq!(detail["title"], CODE, "the directory name stands in");
    assert_eq!(detail["has_cover"], false);
    let src: String = sqlx::query_scalar("SELECT metadata_src FROM works WHERE id = ?")
        .bind(work)
        .fetch_one(&s.db().await)
        .await
        .unwrap();
    assert_eq!(src, "none");
    let tracks: Vec<i64> = detail["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_i64().unwrap())
        .collect();
    assert_eq!(tracks.len(), 3);
    let (status, audio) = s
        .server
        .get_bytes(&format!("/api/tracks/{}/stream", tracks[0]), &s.token)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(audio, wav(), "playable without metadata");

    s.fake.down.store(false, Ordering::SeqCst);
    let scans_before = s
        .tasks()
        .await
        .iter()
        .filter(|t| t["kind"] == "scan")
        .count();
    let refresh = s.until_ended(s.refresh(work, false).await).await;
    assert_eq!(refresh["state"], "done", "{refresh}");
    assert_eq!(refresh["kind"], "metadata");
    let detail = s.detail(work).await;
    assert_eq!(detail["title"], "示例作品一");
    assert_eq!(detail["has_cover"], true);
    // Only the metadata stage ran: no new scan, and the tracks kept their IDs.
    assert_eq!(
        s.tasks()
            .await
            .iter()
            .filter(|t| t["kind"] == "scan")
            .count(),
        scans_before
    );
    let after: Vec<i64> = detail["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_i64().unwrap())
        .collect();
    assert_eq!(after, tracks);
    s.server.crash().await;
}

#[tokio::test]
async fn manual_metadata_and_imported_covers_survive_a_normal_refresh() {
    let mut s = Setup::new().await;
    s.fake.down.store(true, Ordering::SeqCst);
    s.until_ended(s.scan().await).await;
    s.fake.down.store(false, Ordering::SeqCst);
    let work = s.work_id().await;

    let (status, body) = s
        .server
        .request(
            "PUT",
            &format!("/api/works/{work}/metadata"),
            Some(&s.token),
            Some(json!({
                "title": "手动标题",
                "release_date": null,
                "nsfw": false,
                "circle_name": "手动社团",
                "va_names": ["手动声优"],
                "tag_names": ["手动标签"],
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // An imported cover, as the legacy import leaves it.
    std::fs::write(s.data(&format!("covers/{work}.jpg")), b"imported cover").unwrap();
    sqlx::query("UPDATE works SET cover_src = 'imported' WHERE id = ?")
        .bind(work)
        .execute(&s.db().await)
        .await
        .unwrap();

    let refresh = s.until_ended(s.refresh(work, false).await).await;
    assert_eq!(refresh["state"], "done", "{refresh}");
    let detail = s.detail(work).await;
    assert_eq!(detail["title"], "手动标题");
    assert_eq!(detail["circle"]["name"], "手动社团");
    assert_eq!(names(&detail["vas"]), ["手动声优"]);
    assert_eq!(names(&detail["tags"]), ["手动标签"]);
    let (_, cover) = s
        .server
        .get_bytes(&format!("/api/works/{work}/cover"), &s.token)
        .await;
    assert_eq!(cover, b"imported cover");

    let refresh = s.until_ended(s.refresh(work, true).await).await;
    assert_eq!(refresh["state"], "done", "{refresh}");
    let detail = s.detail(work).await;
    assert_eq!(detail["title"], "示例作品一");
    assert_eq!(names(&detail["tags"]), ["ASMR", "School / Academy"]);
    let (_, cover) = s
        .server
        .get_bytes(&format!("/api/works/{work}/cover"), &s.token)
        .await;
    assert_eq!(
        cover,
        std::fs::read(Path::new(FIXTURES).join("cover.jpg")).unwrap()
    );
    s.server.crash().await;
}

#[tokio::test]
async fn cancelling_a_scan_leaves_no_probes_or_temp_files_and_blocks_cleanup_meanwhile() {
    let mut s = Setup::new().await;
    // The scan reaches its metadata stage and waits there.
    s.fake.delay_ms.store(60_000, Ordering::SeqCst);
    let scan = s.scan().await;
    s.work_id().await;
    sleep(Duration::from_millis(500)).await;
    assert_eq!(s.task(scan).await["state"], "running");

    let (status, body) = s
        .post(
            "/api/admin/missing/cleanup",
            json!({"work_ids": [], "location_ids": []}),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "scan_running");

    let (status, _) = s
        .post(&format!("/api/tasks/{scan}/cancel"), json!({}))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let task = s.until_ended(scan).await;
    assert_eq!(task["state"], "cancelled", "{task}");
    // The probes the scan queued run to their end on their own.
    s.until_all_ended().await;
    let probes: Vec<Value> = s
        .tasks()
        .await
        .into_iter()
        .filter(|t| t["kind"] == "probe")
        .collect();
    assert_eq!(probes.len(), 3, "{probes:?}");
    assert!(probes.iter().all(|t| t["state"] == "done"), "{probes:?}");
    assert_eq!(s.ffprobe_children(), 0);
    assert_eq!(s.leftover_temp_files(), Vec::<PathBuf>::new());
    s.server.crash().await;
}

#[tokio::test]
async fn closing_the_event_stream_leaves_the_scan_running_to_the_end() {
    let mut s = Setup::new().await;
    s.fake.delay_ms.store(2_000, Ordering::SeqCst);
    let scan = s.scan().await;
    let mut events = Command::new("curl")
        .args(["--disable", "--silent", "--no-buffer", "--noproxy", "*"])
        .args(["--header", &format!("Authorization: Bearer {}", s.token)])
        .arg(s.server.url(&format!("/api/tasks/{scan}/events")))
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut lines = tokio::io::BufReader::new(events.stdout.take().unwrap()).lines();
    let first = tokio::time::timeout(Duration::from_secs(20), async {
        while let Some(line) = lines.next_line().await.unwrap() {
            if let Some(data) = line.strip_prefix("data: ") {
                return serde_json::from_str::<Value>(data).unwrap();
            }
        }
        panic!("the stream ended without a snapshot");
    })
    .await
    .unwrap();
    assert_eq!(first["id"], scan);
    events.kill().await.unwrap();

    let task = s.until_ended(scan).await;
    assert_eq!(task["state"], "done", "{task}");
    assert_eq!(s.result(scan).await["metadata_fetched"], 1);
    s.server.crash().await;
}
