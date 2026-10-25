//! `POST /api/admin/scan` through the scheduler, against the test app's
//! local root `lib` and its OpenList root `cloud`, which has no storage, as
//! when its backend fails to load.

mod common;

use std::time::Duration;

use axum::http::StatusCode;
use common::*;
use serde_json::{Value, json};

async fn setup() -> (App, String) {
    let app = app().await;
    app.user("admin", true).await;
    app.exec("INSERT INTO roots (id, display_name) VALUES ('lib', 'Library'), ('cloud', 'Cloud')")
        .await;
    let token = app.login("admin").await;
    (app, token)
}

/// Polls the task until it leaves `queued` and `running`.
async fn finished(app: &App, token: &str, id: i64) -> Value {
    for _ in 0..200 {
        let (status, body) = app
            .send(Req::get(&format!("/api/tasks/{id}")).token(token))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        if !matches!(body["state"].as_str(), Some("queued" | "running")) {
            return body;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("task {id} did not finish");
}

#[tokio::test]
async fn scans_a_local_root_as_a_task() {
    let (app, token) = setup().await;
    let lib = app.library();
    std::fs::create_dir_all(lib.join("RJ123456/mp3")).unwrap();
    std::fs::write(lib.join("RJ123456/mp3/01.mp3"), b"audio").unwrap();

    let (status, body) = app
        .send(
            Req::post("/api/admin/scan")
                .token(&token)
                .json(json!({"root": "lib"})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_same_shape(&body, &sample_in("m2", "scan-response.json"), "scan");
    let id = body["task_ids"][0].as_i64().unwrap();
    let task = finished(&app, &token, id).await;
    assert_eq!(task["state"], "done", "{task}");
    assert_eq!(task["kind"], "scan");
    let works: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM works")
        .fetch_one(&app.db.sqlite().unwrap().read)
        .await
        .unwrap();
    assert_eq!(works, 1);
    let tracks: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tracks")
        .fetch_one(&app.db.sqlite().unwrap().read)
        .await
        .unwrap();
    assert_eq!(tracks, 1);

    // The new local audio track gets a background probe. Submitting runs on
    // its own task, so it may land just after the scan finishes.
    let mut probes = 0;
    for _ in 0..200 {
        probes = sqlx::query_scalar(
            "SELECT COUNT(*) FROM tasks WHERE kind = 'probe'
             AND dedupe_key = 'probe:' || (SELECT t.id || ':' || r.source_version || ':' || t.size || ':' || t.mtime
                 FROM tracks t JOIN work_locations l ON l.id = t.location_id
                 JOIN roots r ON r.id = l.root_id)",
        )
        .fetch_one(&app.db.sqlite().unwrap().read)
        .await
        .unwrap();
        if probes > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(probes, 1);

    // Null scans every root that has a backend, which here is only `lib`.
    let (status, body) = app
        .send(
            Req::post("/api/admin/scan")
                .token(&token)
                .json(json!({"root": null})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["task_ids"].as_array().unwrap().len(), 1);
    let task = finished(&app, &token, body["task_ids"][0].as_i64().unwrap()).await;
    assert_eq!(task["state"], "done", "{task}");
}

#[tokio::test]
async fn a_scan_without_a_metadata_source_takes_the_cover_from_the_folder() {
    let app = app_with(|state, dir| {
        state.with_covers(neo_scanner::metadata::CoverDirs::new(
            dir.join("covers"),
            dir.join("cache/tmp"),
        ))
    })
    .await;
    app.user("admin", true).await;
    app.exec("INSERT INTO roots (id, display_name) VALUES ('lib', 'Library')")
        .await;
    let token = app.login("admin").await;
    let work = app.library().join("RJ123456");
    std::fs::create_dir_all(work.join("插图")).unwrap();
    std::fs::write(work.join("01.mp3"), b"audio").unwrap();
    let mut png = Vec::new();
    image::DynamicImage::ImageRgb8(image::RgbImage::new(64, 48))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    std::fs::write(work.join("插图/封面.png"), png).unwrap();

    let (_, body) = app
        .send(
            Req::post("/api/admin/scan")
                .token(&token)
                .json(json!({"root": "lib"})),
        )
        .await;
    let task = finished(&app, &token, body["task_ids"][0].as_i64().unwrap()).await;
    assert_eq!(task["state"], "done", "{task}");

    let (_, works) = app.send(Req::get("/api/works").token(&token)).await;
    let id = works["items"][0]["id"].as_i64().unwrap();
    assert_eq!(works["items"][0]["has_cover"], true, "{works}");
    assert_eq!(works["items"][0]["cover_version"], 1);
    let (status, headers, bytes) = app
        .send_raw(Req::get(&format!("/api/works/{id}/cover")).token(&token))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "image/jpeg");
    assert_eq!(
        image::guess_format(&bytes).unwrap(),
        image::ImageFormat::Jpeg
    );
}

#[tokio::test]
async fn an_unmounted_root_fails_the_task_and_changes_nothing() {
    let (app, token) = setup().await;
    // The library directory exists but is empty, as an unmounted disk
    // leaves it.
    let (_, body) = app
        .send(
            Req::post("/api/admin/scan")
                .token(&token)
                .json(json!({"root": "lib"})),
        )
        .await;
    let task = finished(&app, &token, body["task_ids"][0].as_i64().unwrap()).await;
    assert_eq!(task["state"], "failed", "{task}");
    assert!(task["message"].as_str().unwrap().contains("lib"), "{task}");
    let status: String = sqlx::query_scalar("SELECT status FROM roots WHERE id = 'lib'")
        .fetch_one(&app.db.sqlite().unwrap().read)
        .await
        .unwrap();
    assert_eq!(status, "unavailable");
}

#[tokio::test]
async fn scan_errors() {
    let (app, token) = setup().await;
    app.user("bob", false).await;
    let bob = app.login("bob").await;

    let (status, body) = app
        .send(
            Req::post("/api/admin/scan")
                .token(&bob)
                .json(json!({"root": "lib"})),
        )
        .await;
    assert_error(status, &body, StatusCode::FORBIDDEN, "forbidden");
    let (status, body) = app
        .send(
            Req::post("/api/admin/scan")
                .token(&token)
                .json(json!({"root": "nope"})),
        )
        .await;
    assert_error(status, &body, StatusCode::NOT_FOUND, "not_found");
    let (status, body) = app
        .send(
            Req::post("/api/admin/scan")
                .token(&token)
                .json(json!({"root": "cloud"})),
        )
        .await;
    assert_error(
        status,
        &body,
        StatusCode::SERVICE_UNAVAILABLE,
        "storage_unavailable",
    );
    let (status, body) = app
        .send(
            Req::post("/api/admin/scan")
                .token(&token)
                .raw_body("not json"),
        )
        .await;
    assert_error(status, &body, StatusCode::BAD_REQUEST, "invalid_request");
}
