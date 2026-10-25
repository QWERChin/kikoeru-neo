//! Loudness, waveform and the user settings, with the real ffmpeg.

mod common;

use std::{path::Path, process::Command, time::Duration};

use axum::http::StatusCode;
use common::{App, Req, app, assert_error, assert_same_shape, sample_in};
use serde_json::{Value, json};

/// A 1 kHz sine with a peak of -20 dBFS measures -23 LUFS.
const TONE: &str = "sine=frequency=1000:sample_rate=48000:duration=3,volume=-1.94dB";

fn generate(path: &Path, graph: &str) {
    let status = Command::new("ffmpeg")
        .args(["-nostdin", "-loglevel", "error", "-f", "lavfi", "-i"])
        .arg(graph)
        .args(["-ac", "1", "-y"])
        .arg(path)
        .status()
        .expect("ffmpeg must be on PATH for these tests");
    assert!(status.success());
}

fn size_and_mtime(path: &Path) -> (u64, u128) {
    let meta = std::fs::metadata(path).unwrap();
    let mtime = meta
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    (meta.len(), mtime)
}

/// A work in the local root `lib` with the tone, a text file and a track
/// marked missing. Returns alice's token.
async fn library() -> (App, String) {
    let app = app().await;
    app.user("admin", true).await;
    app.user("alice", false).await;
    let dir = app.library().join("RJ001");
    std::fs::create_dir_all(&dir).unwrap();
    generate(&dir.join("01.wav"), TONE);
    std::fs::write(dir.join("readme.txt"), b"hello").unwrap();
    let (size, mtime) = size_and_mtime(&dir.join("01.wav"));
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

/// Requests the loudness until it is measured, following the 202.
async fn measured(app: &App, token: &str) -> Value {
    let request = || Req::get("/api/tracks/1/loudness").token(token);
    let (status, body) = app.send(request()).await;
    if status == StatusCode::ACCEPTED {
        assert_same_shape(
            &body,
            &sample_in("loudness", "loudness-processing.json"),
            "processing",
        );
        let done = wait_done(app, token, &body["task_id"]).await;
        assert_eq!(done["kind"], "analysis");
        assert_eq!(done["result"], Value::Null);
    } else {
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    let (status, body) = app.send(request()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

#[tokio::test]
async fn head_loudness_does_not_queue_local_analysis() {
    assert_head_does_not_queue_analysis("loudness").await;
}

#[tokio::test]
async fn head_waveform_does_not_queue_local_analysis() {
    assert_head_does_not_queue_analysis("waveform").await;
}

async fn assert_head_does_not_queue_analysis(route: &str) {
    let (app, token) = library().await;
    let admin = app.login("admin").await;
    let (status, _, body) = app
        .send_raw(Req::head(&format!("/api/tracks/1/{route}")).token(&token))
        .await;
    assert!(body.is_empty());
    let (tasks_status, tasks) = app.send(Req::get("/api/tasks").token(&admin)).await;
    assert_eq!(tasks_status, StatusCode::OK, "{tasks}");
    assert_eq!(tasks["total"], 0, "HEAD queued a task: {tasks}");
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn the_first_request_queues_an_analysis_and_then_the_loudness_is_known() {
    let (app, token) = library().await;
    let (status, body) = app
        .send(Req::get("/api/tracks/1/loudness").token(&token))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let loudness = measured(&app, &token).await;
    assert_same_shape(
        &loudness,
        &sample_in("loudness", "loudness.json"),
        "loudness",
    );
    let lufs = loudness["integrated_lufs"].as_f64().unwrap();
    let peak = loudness["true_peak_dbtp"].as_f64().unwrap();
    assert!((lufs + 23.0).abs() <= 0.5, "{lufs} LUFS");
    assert!((peak + 20.0).abs() <= 0.5, "{peak} dBTP");

    // The work detail carries it too.
    let (_, detail) = app.send(Req::get("/api/works/1").token(&token)).await;
    assert_eq!(detail["tracks"][0]["loudness"], loudness);
    assert_eq!(detail["tracks"][1]["loudness"], Value::Null);
}

#[tokio::test]
async fn the_waveform_is_a_thousand_bytes_with_an_etag() {
    let (app, token) = library().await;
    let (status, body) = app
        .send(Req::get("/api/tracks/1/waveform").token(&token))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    wait_done(&app, &token, &body["task_id"]).await;

    let (status, headers, bytes) = app
        .send_raw(Req::get("/api/tracks/1/waveform").token(&token))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "application/octet-stream");
    assert_eq!(headers["cache-control"], "private, max-age=604800");
    assert_eq!(bytes.len(), 1000);
    // -20 dBFS is a tenth of full scale.
    assert!(
        bytes[10..990].iter().all(|b| (24..=28).contains(b)),
        "{bytes:?}"
    );

    let etag = headers["etag"].to_str().unwrap().to_owned();
    let (status, headers, bytes) = app
        .send_raw(
            Req::get("/api/tracks/1/waveform")
                .token(&token)
                .header("if-none-match", &etag),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert_eq!(headers["etag"], etag.as_str());
    assert!(bytes.is_empty());

    // The same task measured the loudness.
    let (status, _) = app
        .send(Req::get("/api/tracks/1/loudness").token(&token))
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn prepare_queues_an_analysis_without_waiting_for_it() {
    let (app, token) = library().await;
    let (status, body) = app
        .send(
            Req::post("/api/tracks/1/prepare")
                .token(&token)
                .json(json!({"can_play": ["audio/wav"], "force_fallback": false})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"], "ready");
    for _ in 0..300 {
        let rows: i64 =
            sqlx::query_scalar("SELECT count(*) FROM track_analysis WHERE track_id = 1")
                .fetch_one(&app.db.sqlite().unwrap().read)
                .await
                .unwrap();
        if rows == 1 {
            let kind: String =
                sqlx::query_scalar("SELECT kind FROM tasks WHERE dedupe_key = 'analysis:1'")
                    .fetch_one(&app.db.sqlite().unwrap().read)
                    .await
                    .unwrap();
            assert_eq!(kind, "analysis");
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("prepare queued no analysis");
}

#[tokio::test]
async fn a_changed_file_is_measured_again() {
    let (app, token) = library().await;
    let first = measured(&app, &token).await;
    let path = app.library().join("RJ001/01.wav");
    // Louder by 10 dB and a different length, so size and mtime both move.
    generate(
        &path,
        "sine=frequency=1000:sample_rate=48000:duration=4,volume=8.06dB",
    );
    let (status, body) = app
        .send(Req::get("/api/tracks/1/loudness").token(&token))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let second = measured(&app, &token).await;
    let delta =
        second["integrated_lufs"].as_f64().unwrap() - first["integrated_lufs"].as_f64().unwrap();
    assert!((delta - 10.0).abs() <= 0.5, "{first} then {second}");
}

#[tokio::test]
async fn analysis_endpoints_answer_only_for_present_audio() {
    let (app, token) = library().await;
    for path in ["loudness", "waveform"] {
        let (status, body) = app
            .send(Req::get(&format!("/api/tracks/2/{path}")).token(&token))
            .await;
        assert_error(status, &body, StatusCode::NOT_FOUND, "not_applicable");
        let (status, body) = app
            .send(Req::get(&format!("/api/tracks/3/{path}")).token(&token))
            .await;
        assert_error(status, &body, StatusCode::GONE, "missing");
        let (status, body) = app
            .send(Req::get(&format!("/api/tracks/99/{path}")).token(&token))
            .await;
        assert_error(status, &body, StatusCode::NOT_FOUND, "not_found");
        let (status, _) = app.send(Req::get(&format!("/api/tracks/1/{path}"))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn other_users_cannot_see_someone_elses_analysis() {
    let (app, token) = library().await;
    app.user("bob", false).await;
    let bob = app.login("bob").await;
    let (status, body) = app
        .send(Req::get("/api/tracks/1/loudness").token(&token))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let uri = format!("/api/tasks/{}", body["task_id"]);
    let (status, _) = app.send(Req::get(&uri).token(&bob)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_queued_analysis_is_rebuilt_after_a_restart() {
    let (app, _token) = library().await;
    app.exec(
        "INSERT INTO tasks (id, kind, dedupe_key, state, payload, created_at, updated_at)
         VALUES (60, 'analysis', 'analysis:1', 'queued', '{\"track_id\": 1}', 0, 0)",
    )
    .await;
    let state = app.state.clone();
    app.state
        .scheduler()
        .recover(|row| neo_server::http::rebuild_task(&state, row))
        .await
        .unwrap();
    let admin = app.login("admin").await;
    wait_done(&app, &admin, &json!(60)).await;
    let lufs: Option<f64> =
        sqlx::query_scalar("SELECT integrated_lufs FROM track_analysis WHERE track_id = 1")
            .fetch_one(&app.db.sqlite().unwrap().read)
            .await
            .unwrap();
    assert!((lufs.unwrap() + 23.0).abs() <= 0.5);
}

fn defaults() -> Value {
    json!({
        "normalize_loudness": false,
        "target_lufs": -16,
        "accent": null,
        "seek_back_seconds": 15,
        "seek_forward_seconds": 30,
    })
}

#[tokio::test]
async fn settings_default_until_saved_and_are_per_user() {
    let (app, token) = library().await;
    let request = |t: &str| Req::get("/api/me/settings").token(t);
    let (status, body) = app.send(request(&token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, defaults());

    let saved = sample_in("loudness", "user-settings.json");
    let (status, body) = app
        .send(
            Req::put("/api/me/settings")
                .token(&token)
                .json(saved.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, saved);
    // A client from before `accent` and the skip seconds sends neither,
    // which means the defaults.
    let custom = json!({"normalize_loudness": true, "target_lufs": -23.5});
    app.send(
        Req::put("/api/me/settings")
            .token(&token)
            .json(custom.clone()),
    )
    .await;
    let mut custom = custom;
    custom["accent"] = Value::Null;
    custom["seek_back_seconds"] = json!(15);
    custom["seek_forward_seconds"] = json!(30);
    assert_eq!(app.send(request(&token)).await.1, custom);

    let admin = app.login("admin").await;
    assert_eq!(app.send(request(&admin)).await.1, defaults());

    for bad in [
        json!({"normalize_loudness": true, "target_lufs": -9}),
        json!({"normalize_loudness": true, "target_lufs": -31}),
        json!({"normalize_loudness": true, "target_lufs": "-16"}),
        json!({"normalize_loudness": true}),
        json!({"normalize_loudness": true, "target_lufs": -16, "accent": "magenta"}),
        json!({"normalize_loudness": true, "target_lufs": -16, "accent": "#ff0000"}),
        json!({"normalize_loudness": true, "target_lufs": -16, "accent": "Blue"}),
        json!({"normalize_loudness": true, "target_lufs": -16, "seek_back_seconds": 0}),
        json!({"normalize_loudness": true, "target_lufs": -16, "seek_back_seconds": 100}),
        json!({"normalize_loudness": true, "target_lufs": -16, "seek_back_seconds": -5}),
        json!({"normalize_loudness": true, "target_lufs": -16, "seek_forward_seconds": 7.5}),
        json!({"normalize_loudness": true, "target_lufs": -16, "seek_forward_seconds": "30"}),
        json!({"normalize_loudness": true, "target_lufs": -16, "seek_forward_seconds": null}),
    ] {
        let (status, body) = app
            .send(Req::put("/api/me/settings").token(&token).json(bad.clone()))
            .await;
        assert_error(status, &body, StatusCode::BAD_REQUEST, "invalid_request");
    }
    assert_eq!(app.send(request(&token)).await.1, custom);
    let (status, _) = app.send(Req::get("/api/me/settings")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn every_accent_colour_is_kept() {
    let (app, token) = library().await;
    for accent in [
        "blue", "purple", "pink", "red", "orange", "green", "teal", "indigo",
    ] {
        let mut settings = defaults();
        settings["accent"] = json!(accent);
        let (status, body) = app
            .send(
                Req::put("/api/me/settings")
                    .token(&token)
                    .json(settings.clone()),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (_, stored) = app.send(Req::get("/api/me/settings").token(&token)).await;
        assert_eq!(stored, settings);
    }
}

#[tokio::test]
async fn skip_seconds_keep_both_ends_of_the_range() {
    let (app, token) = library().await;
    for (back, forward) in [(1, 99), (99, 1), (15, 30)] {
        let mut settings = defaults();
        settings["seek_back_seconds"] = json!(back);
        settings["seek_forward_seconds"] = json!(forward);
        let (status, body) = app
            .send(
                Req::put("/api/me/settings")
                    .token(&token)
                    .json(settings.clone()),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body, settings);
        let (_, stored) = app.send(Req::get("/api/me/settings").token(&token)).await;
        assert_eq!(stored, settings);
    }
}
