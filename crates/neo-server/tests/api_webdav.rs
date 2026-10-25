//! A WebDAV root added through `/api/admin/roots`, scanned and played,
//! against the fake WebDAV server from webdav-client's tests.

mod common;

#[path = "../../webdav-client/tests/common/mod.rs"]
mod fake;

use std::time::Duration;

use axum::http::StatusCode;
use common::*;
use serde_json::{Value, json};

async fn setup() -> (App, String, fake::Server) {
    let app = app().await;
    app.user("admin", true).await;
    app.exec("INSERT INTO roots (id, display_name) VALUES ('lib', 'Library'), ('cloud', 'Cloud')")
        .await;
    let token = app.login("admin").await;
    let server = fake::start().await;
    server.fake.dir("/voice");
    server.fake.dir("/voice/RJ123456");
    server.fake.dir("/voice/RJ123456/mp3");
    server
        .fake
        .file("/voice/RJ123456/mp3/01.mp3", b"0123456789");
    (app, token, server)
}

fn webdav(server: &fake::Server, password: &str) -> Value {
    json!({
        "id": "nas",
        "display_name": "NAS",
        "settings": {
            "type": "webdav",
            "url": server.url(),
            "base_path": "/voice",
            "username": fake::USER,
            "password": password,
        },
    })
}

async fn finished(app: &App, token: &str, id: i64) -> Value {
    for _ in 0..400 {
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

async fn scan(app: &App, token: &str) -> Value {
    let (status, body) = app
        .send(
            Req::post("/api/admin/scan")
                .token(token)
                .json(json!({"root": "nas"})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let task = finished(app, token, body["task_ids"][0].as_i64().unwrap()).await;
    assert_eq!(task["state"], "done", "{task}");
    task
}

#[tokio::test]
async fn a_webdav_root_is_added_scanned_and_played() {
    let (app, token, server) = setup().await;
    let (status, body) = app
        .send(
            Req::post("/api/admin/roots")
                .token(&token)
                .json(webdav(&server, fake::PASSWORD)),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["kind"], "webdav");
    assert_eq!(body["settings"]["type"], "webdav");
    assert_eq!(body["settings"]["password_set"], true);
    assert!(body["settings"].get("password").is_none());
    assert!(body["settings"].get("link_rate").is_none());
    assert_same_shape_with_nullable(
        &body,
        &sample_in("webdav", "roots.json")[0],
        "",
        &[
            "last_scan_at",
            "next_scan_at",
            "settings.list_concurrency",
            "settings.auto_scan_interval",
        ],
    );
    // The save checked base_path with one Depth 0 PROPFIND.
    assert!(
        server
            .fake
            .log
            .lock()
            .unwrap()
            .iter()
            .any(|l| l == "PROPFIND /voice 0")
    );

    scan(&app, &token).await;
    let (status, works) = app.send(Req::get("/api/works?page=1").token(&token)).await;
    assert_eq!(status, StatusCode::OK, "{works}");
    assert_eq!(works["total"], 1, "{works}");
    let id = works["items"][0]["id"].as_i64().unwrap();
    let (_, detail) = app
        .send(Req::get(&format!("/api/works/{id}")).token(&token))
        .await;
    let track = detail["tracks"][0]["id"].as_i64().unwrap();
    let (status, headers, bytes) = app
        .send_raw(
            Req::get(&format!("/api/tracks/{track}/stream"))
                .token(&token)
                .header("range", "bytes=3-5"),
        )
        .await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(headers["content-range"], "bytes 3-5/10");
    assert_eq!(bytes, b"345");
}

#[tokio::test]
async fn an_unchanged_work_is_skipped_until_its_etag_changes() {
    let (app, token, server) = setup().await;
    let (status, body) = app
        .send(
            Req::post("/api/admin/roots")
                .token(&token)
                .json(webdav(&server, fake::PASSWORD)),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    // Only a deep walk lists the work's subfolder.
    let walks = |server: &fake::Server| {
        server
            .fake
            .log
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.as_str() == "PROPFIND /voice/RJ123456/mp3 1")
            .count()
    };
    scan(&app, &token).await;
    assert_eq!(walks(&server), 1);
    scan(&app, &token).await;
    assert_eq!(walks(&server), 1, "the unchanged work is not walked");
    // A server that changes a folder's ETag when a file below it changes
    // gets the work walked again.
    server.fake.set_etag("/voice/RJ123456/mp3", "\"changed\"");
    scan(&app, &token).await;
    assert_eq!(walks(&server), 2);
}

#[tokio::test]
async fn a_wrong_password_is_refused_unless_the_check_is_skipped() {
    let (app, token, server) = setup().await;
    let (status, body) = app
        .send(
            Req::post("/api/admin/roots")
                .token(&token)
                .json(webdav(&server, "wrong")),
        )
        .await;
    assert_error(
        status,
        &body,
        StatusCode::SERVICE_UNAVAILABLE,
        "storage_unavailable",
    );
    let mut request = webdav(&server, "wrong");
    request["skip_check"] = json!(true);
    let (status, body) = app
        .send(Req::post("/api/admin/roots").token(&token).json(request))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // Without a password a new WebDAV root is a bad request.
    let mut request = webdav(&server, "x");
    request["id"] = json!("other");
    request["settings"]
        .as_object_mut()
        .unwrap()
        .remove("password");
    let (status, body) = app
        .send(Req::post("/api/admin/roots").token(&token).json(request))
        .await;
    assert_error(status, &body, StatusCode::BAD_REQUEST, "invalid_request");
}
