//! `--smoke <report> [<library>]`: the release workflow's check that a
//! bundle works on a machine without a display. It runs the bundled ffmpeg
//! and ffprobe, starts the embedded backend on them, and checks that the
//! backend serves its health route and the frontend. Given a library
//! folder, it also adds it as a root, scans it and prepares every audio and
//! video track the way a browser that plays only mp3 and mp4 would, so
//! anything else goes through the bundled ffmpeg (book/src/testing.md).
//! The result goes to the report file, because a Windows release build has
//! no console to print to.

use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use neo_server::embed::{self, Options, Server};
use serde_json::{Value, json};

/// The browser the prepare requests pretend to be.
const CAN_PLAY: [&str; 2] = ["audio/mpeg", "video/mp4"];
/// Scans and transcodes of a few seconds of media, on a slow runner.
const TASK_TIMEOUT: Duration = Duration::from_secs(300);
const SMOKE_USER: &str = "smoke";

/// Runs the check and returns the process exit code.
pub fn main(report: &Path, library: Option<&Path>) -> i32 {
    let mut log = String::new();
    let outcome = check(&mut log, library);
    if let Err(e) = &outcome {
        let _ = writeln!(log, "FAILED: {e:#}");
    }
    if let Err(e) = std::fs::write(report, &log) {
        eprintln!("cannot write {}: {e}\n{log}", report.display());
    }
    i32::from(outcome.is_err())
}

fn check(log: &mut String, library: Option<&Path>) -> Result<()> {
    let ffmpeg = crate::tools::bundled("ffmpeg").context("no ffmpeg next to the executable")?;
    let ffprobe = crate::tools::bundled("ffprobe").context("no ffprobe next to the executable")?;
    for tool in [&ffmpeg, &ffprobe] {
        let out = Command::new(tool)
            .arg("-version")
            .output()
            .with_context(|| format!("cannot run {}", tool.display()))?;
        ensure!(out.status.success(), "{} -version failed", tool.display());
        let first = String::from_utf8_lossy(&out.stdout);
        writeln!(log, "{}", first.lines().next().unwrap_or_default())?;
    }
    let data_dir = std::env::temp_dir().join(format!("kikoeru-neo-smoke-{}", std::process::id()));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let served = runtime.block_on(serve(data_dir.clone(), ffmpeg, ffprobe, library, log));
    let _ = std::fs::remove_dir_all(&data_dir);
    served
}

async fn serve(
    data_dir: PathBuf,
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    library: Option<&Path>,
    log: &mut String,
) -> Result<()> {
    // No DLsite requests from CI: works keep their folder names.
    std::fs::create_dir_all(&data_dir)?;
    std::fs::write(
        data_dir.join("config.toml"),
        "[metadata]\nsource = \"none\"\n",
    )?;
    let server = embed::start(Options {
        data_dir,
        ffmpeg: Some(ffmpeg),
        ffprobe: Some(ffprobe),
        web: Some(crate::web::share(&mut crate::web::context())),
    })
    .await?;
    let checked = requests(&server, library, log).await;
    server.stop().await?;
    checked
}

async fn requests(server: &Server, library: Option<&Path>, log: &mut String) -> Result<()> {
    let api = Api {
        client: reqwest::Client::builder().no_proxy().build()?,
        base: format!("http://127.0.0.1:{}", server.port()),
        token: None,
    };
    let health = api.get("/api/health").await?;
    ensure!(health["status"] == "ok", "unexpected health: {health}");
    // A 200 here means the backend serves the frontend Tauri embedded.
    api.client.get(&api.base).send().await?.error_for_status()?;
    writeln!(
        log,
        "backend served /api/health and / on port {}",
        server.port()
    )?;
    let Some(library) = library else {
        return Ok(());
    };
    let mut password = [0u8; 16];
    getrandom::fill(&mut password).map_err(|e| anyhow::anyhow!("no system randomness: {e}"))?;
    let password: String = password.iter().map(|b| format!("{b:02x}")).collect();
    server.ensure_admin(SMOKE_USER, &password).await?;
    let api = Api {
        token: Some(server.session_token(SMOKE_USER).await?),
        ..api
    };
    scan(&api, library, log).await?;
    prepare_all(&api, log).await
}

async fn scan(api: &Api, library: &Path, log: &mut String) -> Result<()> {
    let path = library
        .to_str()
        .context("the library path is not valid text")?;
    api.post(
        "/api/admin/roots",
        &json!({
            "id": "smoke",
            "display_name": "smoke",
            "settings": {"type": "local", "path": path, "watch": false},
        }),
    )
    .await?;
    let started = api
        .post("/api/admin/scan", &json!({"root": "smoke"}))
        .await?;
    let task = started["task_ids"][0].as_i64().context("no scan task")?;
    api.wait(task).await?;
    let works = api.get("/api/works?page=1").await?;
    let count = works["total"].as_i64().unwrap_or(0);
    ensure!(
        count > 0,
        "the scan found no works in {}",
        library.display()
    );
    writeln!(log, "scanned {count} works from {}", library.display())?;
    Ok(())
}

/// Every work's ID, page by page: a library larger than one page must not
/// pass the check on its first page alone.
async fn all_work_ids(api: &Api) -> Result<Vec<i64>> {
    let mut ids = Vec::new();
    let mut page = 1;
    loop {
        let works = api.get(&format!("/api/works?page={page}")).await?;
        let items = works["items"].as_array().context("no works")?;
        for work in items {
            ids.push(work["id"].as_i64().context("a work without an ID")?);
        }
        let total = works["total"].as_u64().context("no total")?;
        if !more_pages(ids.len(), items.len(), total) {
            return Ok(ids);
        }
        page += 1;
    }
}

/// An empty page also ends the walk, so a total that shrank during the
/// walk cannot loop forever.
fn more_pages(seen: usize, on_this_page: usize, total: u64) -> bool {
    on_this_page > 0 && (seen as u64) < total
}

async fn prepare_all(api: &Api, log: &mut String) -> Result<()> {
    for id in all_work_ids(api).await? {
        let detail = api.get(&format!("/api/works/{id}")).await?;
        let tracks = detail["tracks"].as_array().context("no tracks")?;
        for track in tracks {
            if !matches!(track["kind"].as_str(), Some("audio" | "video")) {
                continue;
            }
            let path = track["path"].as_str().unwrap_or_default();
            let track_id = track["id"].as_i64().context("a track without an ID")?;
            let (url, mime) = prepare(api, track_id).await?;
            let start = url::Url::parse(&api.base)?.join(&url)?;
            let read = read_media(api, start)
                .await
                .with_context(|| format!("{path}: {url}"))?;
            writeln!(log, "{path}: {mime} at {url}, {read} bytes read")?;
        }
    }
    Ok(())
}

/// Reads the first bytes of a playable URL the way a media element asks
/// for them. A playlist is followed to its first segment, so HLS counts
/// only once real video bytes arrive.
async fn read_media(api: &Api, mut url: url::Url) -> Result<usize> {
    for _ in 0..3 {
        let playlist = url.path().ends_with(".m3u8");
        let mut request = api
            .client
            .get(url.clone())
            .bearer_auth(api.token.as_deref().unwrap_or_default());
        if !playlist {
            request = request.header("Range", "bytes=0-1023");
        }
        let response = request.send().await?;
        let status = response.status();
        ensure!(status.is_success(), "{url} answered {status}");
        let body = response.bytes().await?;
        ensure!(!body.is_empty(), "{url} sent no bytes");
        if !playlist {
            return Ok(body.len());
        }
        let text = String::from_utf8_lossy(&body);
        let first = text
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty() && !line.starts_with('#'))
            .with_context(|| format!("{url} lists no media"))?;
        // Segment URIs carry their own media token, so they are resolved
        // as they are written.
        url = url.join(first)?;
    }
    bail!("{url}: playlists nested too deeply")
}

/// Prepares a track and waits for a transcode when one is needed.
async fn prepare(api: &Api, track: i64) -> Result<(String, String)> {
    let answer = api
        .post(
            &format!("/api/tracks/{track}/prepare"),
            &json!({"can_play": CAN_PLAY, "force_fallback": false}),
        )
        .await?;
    let media = match answer["state"].as_str() {
        Some("ready") => answer,
        Some("processing") => {
            let task = answer["task_id"].as_i64().context("no transcode task")?;
            api.wait(task).await?["result"].clone()
        }
        _ => bail!("unexpected prepare answer: {answer}"),
    };
    let url = media["url"].as_str().context("no media URL")?;
    let mime = media["mime_type"].as_str().unwrap_or_default();
    Ok((url.to_owned(), mime.to_owned()))
}

/// `--e2e-server <library> <port file>`, in test builds only: a standalone
/// server for the WebDriver test's remote mode. It serves the scanned
/// library to the user `remote`, whose password comes from
/// `KIKOERU_NEO_E2E_REMOTE_PASSWORD`, writes its port to the file, and runs
/// until it is killed.
#[cfg(feature = "e2e")]
pub fn e2e_server(library: &Path, port_file: &Path) -> i32 {
    let run = || -> Result<()> {
        let password = std::env::var("KIKOERU_NEO_E2E_REMOTE_PASSWORD")
            .context("KIKOERU_NEO_E2E_REMOTE_PASSWORD is not set")?;
        let data_dir =
            std::env::temp_dir().join(format!("kikoeru-neo-e2e-remote-{}", std::process::id()));
        std::fs::create_dir_all(&data_dir)?;
        std::fs::write(
            data_dir.join("config.toml"),
            "[metadata]\nsource = \"none\"\n",
        )?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            // As in local mode: the bundled tools, else the ones on PATH.
            let server = embed::start(Options {
                data_dir,
                ffmpeg: crate::tools::bundled("ffmpeg"),
                ffprobe: crate::tools::bundled("ffprobe"),
                web: Some(crate::web::share(&mut crate::web::context())),
            })
            .await?;
            server.ensure_admin("remote", &password).await?;
            let api = Api {
                client: reqwest::Client::builder().no_proxy().build()?,
                base: format!("http://127.0.0.1:{}", server.port()),
                token: Some(server.session_token("remote").await?),
            };
            let mut log = String::new();
            scan(&api, library, &mut log).await?;
            std::fs::write(port_file, server.port().to_string())?;
            println!("{log}e2e server on port {}", server.port());
            std::future::pending::<()>().await;
            Ok(())
        })
    };
    match run() {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("e2e server failed: {e:#}");
            1
        }
    }
}

struct Api {
    client: reqwest::Client,
    base: String,
    token: Option<String>,
}

impl Api {
    async fn get(&self, path: &str) -> Result<Value> {
        let mut request = self.client.get(format!("{}{path}", self.base));
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        answer(path, request.send().await?).await
    }

    async fn post(&self, path: &str, body: &Value) -> Result<Value> {
        let request = self
            .client
            .post(format!("{}{path}", self.base))
            .bearer_auth(self.token.as_deref().unwrap_or_default())
            .json(body);
        answer(path, request.send().await?).await
    }

    /// Polls a task until it is done, and fails on any other end state.
    async fn wait(&self, task: i64) -> Result<Value> {
        let deadline = Instant::now() + TASK_TIMEOUT;
        loop {
            let snapshot = self.get(&format!("/api/tasks/{task}")).await?;
            match snapshot["state"].as_str() {
                Some("done") => return Ok(snapshot),
                Some("queued" | "running") if Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
                _ => bail!("task {task} did not finish: {snapshot}"),
            }
        }
    }
}

async fn answer(path: &str, response: reqwest::Response) -> Result<Value> {
    let status = response.status();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    ensure!(status.is_success(), "{path} answered {status}: {body}");
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::more_pages;

    #[test]
    fn the_walk_goes_past_the_first_page_and_stops_at_the_total() {
        // 41 works at 40 a page: the first page leaves one for the second.
        assert!(more_pages(40, 40, 41));
        assert!(!more_pages(41, 1, 41));
        assert!(!more_pages(3, 3, 3));
        assert!(!more_pages(40, 0, 41));
    }
}
