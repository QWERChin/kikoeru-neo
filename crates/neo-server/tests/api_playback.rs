//! `prepare`, `playable` and the transcode task, with the real ffmpeg.

mod common;

use std::{process::Command, time::Duration};

use axum::http::StatusCode;
use common::{App, Req, app, app_low_disk, assert_error};
use serde_json::{Value, json};

/// A work in the local root `lib` with a generated two-second wav, a text
/// file, and a track marked missing.
async fn library() -> (App, String) {
    library_in(app().await).await
}

async fn library_in(app: App) -> (App, String) {
    app.user("admin", true).await;
    app.user("alice", false).await;
    let dir = app.library().join("RJ001");
    std::fs::create_dir_all(&dir).unwrap();
    let status = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=duration=2",
            "-y",
        ])
        .arg(dir.join("01.wav"))
        .status()
        .expect("ffmpeg must be on PATH for these tests");
    assert!(status.success());
    std::fs::write(dir.join("readme.txt"), b"hello").unwrap();
    let meta = std::fs::metadata(dir.join("01.wav")).unwrap();
    let size = meta.len();
    let mtime = meta
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    app.exec(&format!(
        "INSERT INTO roots (id, display_name) VALUES ('lib', 'Lib');
         INSERT INTO works (id, code, title, metadata_src, created_at, updated_at)
           VALUES (1, 'RJ000001', 'Work one', 'scan', 0, 0);
         INSERT INTO work_locations (id, work_id, root_id, rel_path, display_path)
           VALUES (1, 1, 'lib', CAST('RJ001' AS BLOB), 'RJ001');
         INSERT INTO tracks (id, work_id, location_id, rel_path, display_path, sort_key, kind, size, mtime, missing_at) VALUES
           (1, 1, 1, CAST('01.wav' AS BLOB), '01.wav', 'a', 'audio', {size}, {mtime}, NULL),
           (2, 1, 1, CAST('readme.txt' AS BLOB), 'readme.txt', 'b', 'text', 5, 1000, NULL),
           (3, 1, 1, CAST('gone.wav' AS BLOB), 'gone.wav', 'c', 'audio', 1, 1000, 1);"
    ))
    .await;
    let token = app.login("alice").await;
    (app, token)
}

fn prepare(token: &str, can_play: &[&str], force: bool) -> Req {
    Req::post("/api/tracks/1/prepare")
        .token(token)
        .json(json!({"can_play": can_play, "force_fallback": force}))
}

async fn wait_done(app: &App, token: &str, task: &Value) -> Value {
    let uri = format!("/api/tasks/{task}");
    for _ in 0..300 {
        let (status, body) = app.send(Req::get(&uri).token(token)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        match body["state"].as_str().unwrap() {
            "done" => return body,
            "queued" | "running" => tokio::time::sleep(Duration::from_millis(50)).await,
            other => panic!("task ended {other}: {body}"),
        }
    }
    panic!("task {task} did not finish");
}

#[tokio::test]
async fn a_format_the_browser_plays_is_ready_at_once_and_the_probe_is_stored() {
    let (app, token) = library().await;
    let (status, body) = app.send(prepare(&token, &["audio/wav"], false)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({"state": "ready", "url": "/api/tracks/1/stream", "mime_type": "audio/wav"})
    );
    let (probe, duration): (Option<String>, Option<i64>) =
        sqlx::query_as("SELECT probe, duration_ms FROM tracks WHERE id = 1")
            .fetch_one(&app.db.sqlite().unwrap().read)
            .await
            .unwrap();
    assert!(probe.unwrap().contains("pcm_"));
    assert!((1900..=2100).contains(&duration.unwrap()));
}

#[tokio::test]
async fn an_unplayable_format_is_transcoded_once_and_then_served() {
    let (app, token) = library().await;
    let (status, body) = app
        .send(Req::get("/api/tracks/1/playable").token(&token))
        .await;
    assert_error(status, &body, StatusCode::CONFLICT, "media_not_ready");

    let release = hold_heavy_lane(&app).await;
    let (status, first) = app.send(prepare(&token, &["audio/mpeg"], false)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{first}");
    assert_eq!(first["state"], "processing");
    let (_, again) = app.send(prepare(&token, &["audio/mpeg"], false)).await;
    assert_eq!(again["task_id"], first["task_id"], "one transcode per file");
    release.send(()).unwrap();

    let done = wait_done(&app, &token, &first["task_id"]).await;
    assert_eq!(
        done["result"],
        json!({"url": "/api/tracks/1/playable", "mime_type": "audio/mp4"})
    );
    assert_eq!(done["progress"], 1);

    let (status, body) = app.send(prepare(&token, &["audio/mpeg"], false)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["url"], "/api/tracks/1/playable");

    let (status, headers, bytes) = app
        .send_raw(
            Req::get("/api/tracks/1/playable")
                .token(&token)
                .header("range", "bytes=0-99"),
        )
        .await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(headers["content-type"], "audio/mp4");
    assert_eq!(bytes.len(), 100);
}

#[tokio::test]
async fn force_fallback_transcodes_even_a_playable_format() {
    let (app, token) = library().await;
    let (status, body) = app.send(prepare(&token, &["audio/wav"], true)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    wait_done(&app, &token, &body["task_id"]).await;
}

#[tokio::test]
async fn other_users_cannot_watch_someone_elses_transcode() {
    let (app, token) = library().await;
    app.user("bob", false).await;
    let release = hold_heavy_lane(&app).await;
    let (_, body) = app.send(prepare(&token, &[], false)).await;
    let bob = app.login("bob").await;
    let (status, _) = app
        .send(Req::get(&format!("/api/tasks/{}", body["task_id"])).token(&bob))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Asking for the same track joins the task and makes it visible.
    let (_, joined) = app.send(prepare(&bob, &[], false)).await;
    assert_eq!(joined["task_id"], body["task_id"]);
    let (status, _) = app
        .send(Req::get(&format!("/api/tasks/{}", body["task_id"])).token(&bob))
        .await;
    assert_eq!(status, StatusCode::OK);
    release.send(()).unwrap();
}

#[tokio::test]
async fn prepare_rejects_non_media_missing_and_unknown_tracks() {
    let (app, token) = library().await;
    let send = |id: i64| {
        Req::post(&format!("/api/tracks/{id}/prepare"))
            .token(&token)
            .json(json!({"can_play": []}))
    };
    let (status, body) = app.send(send(2)).await;
    assert_error(status, &body, StatusCode::BAD_REQUEST, "invalid_request");
    let (status, body) = app.send(send(3)).await;
    assert_error(status, &body, StatusCode::GONE, "missing");
    let (status, body) = app.send(send(9)).await;
    assert_error(status, &body, StatusCode::NOT_FOUND, "not_found");
    let (status, _) = app
        .send(Req::post("/api/tracks/1/prepare").json(json!({"can_play": []})))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_queued_transcode_is_rebuilt_after_a_restart() {
    let (app, token) = library().await;
    let key = key_of_track_1(&app).await;
    app.exec(&format!(
        "INSERT INTO tasks (id, kind, dedupe_key, state, payload, created_at, updated_at)
         VALUES (50, 'transcode', 'x', 'queued', '{{\"track_id\": 1, \"key\": \"{key}\"}}', 0, 0)"
    ))
    .await;
    let state = app.state.clone();
    app.state
        .scheduler()
        .recover(|row| neo_server::http::rebuild_task(&state, row))
        .await
        .unwrap();
    let admin = app.login("admin").await;
    let done = wait_done(&app, &admin, &json!(50)).await;
    assert_eq!(done["result"]["url"], "/api/tracks/1/playable");
    let (status, _, _) = app
        .send_raw(Req::get("/api/tracks/1/playable").token(&token))
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_background_probe_stores_the_result() {
    let (app, _token) = library().await;
    let scheduler = app.state.scheduler().clone();
    let task = scheduler
        .submit(
            neo_server::http::probe_spec(1),
            neo_server::http::probe_job(app.state.clone(), 1),
        )
        .await
        .unwrap();
    let admin = app.login("admin").await;
    let done = wait_done(&app, &admin, &json!(task.id)).await;
    assert_eq!(done["kind"], "probe");
    let probe: Option<String> = sqlx::query_scalar("SELECT probe FROM tracks WHERE id = 1")
        .fetch_one(&app.db.sqlite().unwrap().read)
        .await
        .unwrap();
    assert!(probe.unwrap().contains("pcm_"));
    // A missing track ends the task quietly.
    let missing = scheduler
        .submit(
            neo_server::http::probe_spec(3),
            neo_server::http::probe_job(app.state.clone(), 3),
        )
        .await
        .unwrap();
    wait_done(&app, &admin, &json!(missing.id)).await;
}

async fn key_of_track_1(app: &App) -> String {
    let (size, mtime): (i64, i64) = sqlx::query_as("SELECT size, mtime FROM tracks WHERE id = 1")
        .fetch_one(&app.db.sqlite().unwrap().read)
        .await
        .unwrap();
    neo_media::CacheKey {
        root_id: "lib".into(),
        source_version: 0,
        path: b"RJ001/01.wav".to_vec(),
        size: size as u64,
        mtime: Some(mtime),
        params: neo_media::AAC_PARAMS,
    }
    .digest()
}

#[tokio::test]
async fn a_queued_transcode_for_an_older_version_of_the_file_fails() {
    let (app, _token) = library().await;
    app.exec(
        "INSERT INTO tasks (id, kind, dedupe_key, state, payload, created_at, updated_at)
         VALUES (51, 'transcode', 'old', 'queued', '{\"track_id\": 1, \"key\": \"stale\"}', 0, 0)",
    )
    .await;
    let state = app.state.clone();
    app.state
        .scheduler()
        .recover(|row| neo_server::http::rebuild_task(&state, row))
        .await
        .unwrap();
    let admin = app.login("admin").await;
    for _ in 0..100 {
        let (_, body) = app.send(Req::get("/api/tasks/51").token(&admin)).await;
        if body["state"] == "failed" {
            assert_eq!(body["message"], "源文件已经变了，请重新播放");
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the stale transcode did not fail");
}

#[tokio::test]
async fn a_file_replaced_since_the_scan_is_probed_and_transcoded_as_it_is_now() {
    let (app, token) = library().await;
    let (_, first) = app.send(prepare(&token, &[], false)).await;
    wait_done(&app, &token, &first["task_id"]).await;

    // Replace the file without rescanning. The row still has the old size.
    let path = app.library().join("RJ001/01.wav");
    let status = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=duration=3",
            "-y",
        ])
        .arg(&path)
        .status()
        .unwrap();
    assert!(status.success());

    let (status, second) = app.send(prepare(&token, &[], false)).await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "the old transcode is not reused: {second}"
    );
    assert_ne!(second["task_id"], first["task_id"]);
    wait_done(&app, &token, &second["task_id"]).await;
    let (status, _, _) = app
        .send_raw(Req::get("/api/tracks/1/playable").token(&token))
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_running_transcode_can_be_joined_when_the_disk_is_low() {
    let (app, token) = library_in(app_low_disk().await).await;
    let key = key_of_track_1(&app).await;
    app.exec(&format!(
        "INSERT INTO tasks (id, kind, dedupe_key, state, payload, created_at, updated_at)
         VALUES (60, 'transcode', 'transcode:{key}', 'running', '{{}}', 0, 0)"
    ))
    .await;
    let (status, body) = app.send(prepare(&token, &[], false)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["task_id"], 60);

    app.exec("UPDATE tasks SET state = 'failed' WHERE id = 60")
        .await;
    let (status, body) = app.send(prepare(&token, &[], false)).await;
    assert_error(
        status,
        &body,
        StatusCode::INSUFFICIENT_STORAGE,
        "insufficient_storage",
    );
}

/// Occupies the single heavy lane until the returned sender fires, so a
/// transcode submitted meanwhile stays queued and the test cannot race it.
async fn hold_heavy_lane(app: &App) -> tokio::sync::oneshot::Sender<()> {
    use futures_util::FutureExt;
    let (started, started_rx) = tokio::sync::oneshot::channel();
    let (release, release_rx) = tokio::sync::oneshot::channel::<()>();
    let spec = neo_server::tasks::TaskSpec {
        kind: "transcode",
        dedupe_key: None,
        payload: json!({}),
        lane: neo_server::tasks::Lane::Heavy,
        timeout: None,
        queue_limit: None,
    };
    let job: neo_server::tasks::Job = Box::new(move |_| {
        async move {
            let _ = started.send(());
            let _ = release_rx.await;
            Ok(None)
        }
        .boxed()
    });
    app.state.scheduler().submit(spec, job).await.unwrap();
    started_rx.await.unwrap();
    release
}
