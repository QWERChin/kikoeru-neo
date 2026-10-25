//! Changing a stored root must not reuse media from its previous location.

mod common;

use std::{path::Path, process::Command, time::Duration};

use axum::http::StatusCode;
use common::{App, Req};
use serde_json::{Value, json};

fn generate(root: &Path, frequency: u32) {
    let work = root.join("RJ000001");
    std::fs::create_dir_all(&work).unwrap();
    let file = work.join("01.wav");
    let status = Command::new("ffmpeg")
        .args(["-nostdin", "-loglevel", "error", "-f", "lavfi", "-i"])
        .arg(format!(
            "sine=frequency={frequency}:sample_rate=48000:duration=2"
        ))
        .arg(&file)
        .status()
        .unwrap();
    assert!(status.success());
    std::fs::File::options()
        .write(true)
        .open(file)
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1_700_000_000))
        .unwrap();
}

fn settings(root: &Path) -> Value {
    json!({"type": "local", "path": root.display().to_string()})
}

async fn scan(app: &App) {
    let storage = app.state.storage("extra").unwrap();
    neo_scanner::scan_root(
        &app.db,
        &neo_scanner::Root {
            id: "extra",
            storage: &*storage,
            probe_after_scan: true,
            skip_unchanged: false,
        },
        &["extra".to_owned()],
        &neo_scanner::NoObserver,
        neo_db::now_ms(),
    )
    .await
    .unwrap();
}

async fn wait_done(app: &App, token: &str, task: &Value) {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let (status, body) = app
                .send(Req::get(&format!("/api/tasks/{task}")).token(token))
                .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            match body["state"].as_str().unwrap() {
                "done" => break,
                "queued" | "running" => tokio::time::sleep(Duration::from_millis(20)).await,
                _ => panic!("{body}"),
            }
        }
    })
    .await
    .unwrap();
}

fn prepare(token: &str) -> Req {
    Req::post("/api/tracks/1/prepare")
        .token(token)
        .json(json!({"can_play": ["audio/wav"], "force_fallback": true}))
}

async fn setup() -> (App, String, std::path::PathBuf) {
    setup_in(common::app().await).await
}

async fn setup_in(app: App) -> (App, String, std::path::PathBuf) {
    app.user("admin", true).await;
    let token = app.login("admin").await;
    let a = app.data_dir().join("a");
    let b = app.data_dir().join("b");
    generate(&a, 440);
    generate(&b, 880);
    let a_meta = std::fs::metadata(a.join("RJ000001/01.wav")).unwrap();
    let b_meta = std::fs::metadata(b.join("RJ000001/01.wav")).unwrap();
    assert_eq!(a_meta.len(), b_meta.len());
    assert_eq!(a_meta.modified().unwrap(), b_meta.modified().unwrap());
    let (status, body) = app
        .send(Req::post("/api/admin/roots").token(&token).json(json!({
            "id": "extra", "display_name": "Extra", "settings": settings(&a),
        })))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    scan(&app).await;
    (app, token, b)
}

async fn relocate(app: &App, token: &str, root: &Path) {
    let (status, body) = app
        .send(Req::put("/api/admin/roots/extra").token(token).json(json!({
            "display_name": "Extra", "settings": settings(root),
        })))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

async fn scan_with_background_probes(app: &App, token: &str) -> Vec<i64> {
    let (status, body) = app
        .send(
            Req::post("/api/admin/scan")
                .token(token)
                .json(json!({"root": "extra", "full": true})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    wait_done(app, token, &body["task_ids"][0]).await;
    // A completed scan has submitted all probes, but they may still be running.
    let (status, body) = app.send(Req::get("/api/tasks").token(token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut probes = Vec::new();
    for task in body["items"].as_array().unwrap() {
        if task["kind"] == "probe" {
            wait_done(app, token, &task["id"]).await;
            probes.push(task["id"].as_i64().unwrap());
        }
    }
    probes
}

#[tokio::test]
async fn an_unreadable_unchanged_local_file_is_not_probed_on_every_scan() {
    let app = common::app().await;
    app.user("admin", true).await;
    let token = app.login("admin").await;
    let a = app.data_dir().join("a");
    let b = app.data_dir().join("b");
    std::fs::create_dir_all(a.join("RJ000001")).unwrap();
    std::fs::write(a.join("RJ000001/01.wav"), b"not a WAV file").unwrap();
    let (status, body) = app
        .send(Req::post("/api/admin/roots").token(&token).json(json!({
            "id": "extra", "display_name": "Extra", "settings": settings(&a),
        })))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let probes = scan_with_background_probes(&app, &token).await;
    assert_eq!(probes.len(), 1);
    let track = neo_db::media::track_file(&app.db, 1)
        .await
        .unwrap()
        .unwrap();
    assert!(
        track.probe.is_none(),
        "ffprobe cannot read the synthetic file"
    );
    assert_eq!(
        scan_with_background_probes(&app, &token).await,
        probes,
        "an unchanged unreadable file must not enqueue another probe"
    );

    std::fs::rename(&a, &b).unwrap();
    relocate(&app, &token, &b).await;
    let relocated_probes = scan_with_background_probes(&app, &token).await;
    assert_eq!(relocated_probes.len(), 2, "relocation permits a new probe");
    assert_eq!(
        scan_with_background_probes(&app, &token).await,
        relocated_probes,
        "a failed probe after relocation must not be retried on every scan"
    );
    app.state.scheduler().shutdown(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn scanning_a_relocated_root_restores_durations_without_prepare() {
    let app = common::app().await;
    app.user("admin", true).await;
    let token = app.login("admin").await;
    let a = app.data_dir().join("a");
    let b = app.data_dir().join("b");
    generate(&a, 440);
    let (status, body) = app
        .send(Req::post("/api/admin/roots").token(&token).json(json!({
            "id": "extra", "display_name": "Extra", "settings": settings(&a),
        })))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    scan_with_background_probes(&app, &token).await;
    let (status, before) = app.send(Req::get("/api/works/1").token(&token)).await;
    assert_eq!(status, StatusCode::OK, "{before}");
    let id = before["tracks"][0]["id"].as_i64().unwrap();
    assert_eq!(before["tracks"][0]["duration_ms"], 2000);
    assert_eq!(before["total_duration_ms"], 2000);
    let old = neo_db::media::track_file(&app.db, id)
        .await
        .unwrap()
        .unwrap();
    assert!(old.probe.is_some());

    // Moving the directory preserves each file's path, size and mtime.
    std::fs::rename(&a, &b).unwrap();
    relocate(&app, &token, &b).await;
    let invalidated = neo_db::media::track_file(&app.db, id)
        .await
        .unwrap()
        .unwrap();
    assert!(invalidated.probe.is_none());
    assert!(invalidated.duration_ms.is_none());
    assert_eq!(invalidated.source_version, old.source_version + 1);

    scan_with_background_probes(&app, &token).await;
    let (status, after) = app.send(Req::get("/api/works/1").token(&token)).await;
    assert_eq!(status, StatusCode::OK, "{after}");
    assert_eq!(after["tracks"].as_array().unwrap().len(), 1);
    assert_eq!(after["tracks"][0]["id"], id);
    assert_eq!(after["tracks"][0]["duration_ms"], 2000);
    assert_eq!(after["total_duration_ms"], 2000);
    let restored = neo_db::media::track_file(&app.db, id)
        .await
        .unwrap()
        .unwrap();
    assert!(restored.probe.is_some());
    assert_eq!((restored.size, restored.mtime), (old.size, old.mtime));
    assert_eq!(restored.source_version, invalidated.source_version);
    app.state.scheduler().shutdown(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn relocating_a_root_reprocesses_identical_file_metadata() {
    let (app, token, b) = setup().await;
    let (status, body) = app.send(prepare(&token)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    wait_done(&app, &token, &body["task_id"]).await;
    let (_, _, old_audio) = app
        .send_raw(Req::get("/api/tracks/1/playable").token(&token))
        .await;
    let (status, body) = app
        .send(Req::get("/api/tracks/1/loudness").token(&token))
        .await;
    if status == StatusCode::ACCEPTED {
        wait_done(&app, &token, &body["task_id"]).await;
    } else {
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    relocate(&app, &token, &b).await;
    scan(&app).await;
    let (status, body) = app.send(prepare(&token)).await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "the old source must miss: {body}"
    );
    wait_done(&app, &token, &body["task_id"]).await;
    let (status, _, audio) = app
        .send_raw(Req::get("/api/tracks/1/playable").token(&token))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(audio != old_audio);

    // Decode the returned AAC and count rising zero crossings. The new
    // source is 880 Hz, while the old source is 440 Hz.
    let encoded = app.data_dir().join("returned.m4a");
    std::fs::write(&encoded, audio).unwrap();
    let decoded = Command::new("ffmpeg")
        .args(["-nostdin", "-loglevel", "error", "-i"])
        .arg(encoded)
        .args(["-f", "s16le", "-ac", "1", "-ar", "48000", "pipe:1"])
        .output()
        .unwrap();
    assert!(decoded.status.success());
    let samples: Vec<_> = decoded
        .stdout
        .as_chunks::<2>()
        .0
        .iter()
        .copied()
        .map(i16::from_le_bytes)
        .collect();
    let crossings = samples[4800..52800]
        .windows(2)
        .filter(|pair| pair[0] <= 0 && pair[1] > 0)
        .count();
    assert!((870..890).contains(&crossings), "{crossings} Hz");
    app.state.scheduler().shutdown(Duration::from_secs(5)).await;
}

#[cfg(unix)]
#[tokio::test]
async fn an_in_flight_transcode_cannot_publish_for_the_new_location() {
    use std::{os::unix::fs::PermissionsExt, sync::Arc};

    let gate = tempfile::tempdir().unwrap();
    let wrapper = gate.path().join("ffmpeg-gated");
    std::fs::write(
        &wrapper,
        // /bin/sh, because the Nix build sandbox has no /usr/bin/env.
        r#"#!/bin/sh
case " $* " in
*" -c:a aac "*)
    gate=$(dirname "$0")
    touch "$gate/started"
    while [ ! -f "$gate/release" ]; do sleep 0.01; done
    ;;
esac
exec ffmpeg "$@"
"#,
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let app = common::app_with(|state, dir| {
        state.with_media(neo_server::http::Media {
            cache: neo_media::TranscodeCache::open(&dir.join("cache"), u64::MAX, 0)
                .unwrap()
                .with_tools(neo_media::Tools {
                    ffmpeg: wrapper,
                    ..Default::default()
                }),
            probe_slots: Arc::new(tokio::sync::Semaphore::new(4)),
            queue_limit: 32,
            job_timeout: Duration::from_secs(20),
            remote_analysis_delay: Duration::ZERO,
            video_transcode: true,
        })
    })
    .await;
    let (app, token, b) = setup_in(app).await;
    let (status, task) = app.send(prepare(&token)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{task}");
    tokio::time::timeout(Duration::from_secs(10), async {
        while !gate.path().join("started").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    relocate(&app, &token, &b).await;
    std::fs::write(gate.path().join("release"), b"").unwrap();
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let (_, body) = app
                .send(Req::get(&format!("/api/tasks/{}", task["task_id"])).token(&token))
                .await;
            match body["state"].as_str().unwrap() {
                "failed" => break,
                "queued" | "running" => tokio::time::sleep(Duration::from_millis(20)).await,
                _ => panic!("old-source task must fail: {body}"),
            }
        }
    })
    .await
    .unwrap();
    let (status, body) = app
        .send(Req::get("/api/tracks/1/playable").token(&token))
        .await;
    common::assert_error(status, &body, StatusCode::CONFLICT, "media_not_ready");
    let (status, task) = app.send(prepare(&token)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{task}");
    wait_done(&app, &token, &task["task_id"]).await;
    app.state.scheduler().shutdown(Duration::from_secs(5)).await;
}
