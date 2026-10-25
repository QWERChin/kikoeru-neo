//! The admin task list, cancel, moving a queued scan to the front and the
//! event stream.

mod common;

use std::time::Duration;

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode, header},
};
use common::*;
use futures_util::FutureExt;
use http_body_util::BodyExt;
use neo_server::tasks::{Job, JobError, Lane, TaskSpec};
use serde_json::{Value, json};
use tokio::sync::oneshot;
use tower::ServiceExt;

fn spec(key: &str) -> TaskSpec {
    TaskSpec {
        kind: "scan",
        dedupe_key: Some(key.to_owned()),
        payload: json!({}),
        lane: Lane::Heavy,
        timeout: None,
        queue_limit: None,
    }
}

/// A scan in the scan lane, where one runs at a time.
fn scan_spec(key: &str) -> TaskSpec {
    TaskSpec {
        lane: Lane::Scan,
        ..spec(key)
    }
}

/// Runs until released or cancelled, reporting 0.5 progress first.
fn blocking(release: oneshot::Receiver<()>) -> Job {
    Box::new(move |ctx| {
        async move {
            ctx.progress().send_replace(0.5);
            tokio::select! {
                _ = release => Ok(None),
                () = ctx.cancel.cancelled() => Err(JobError("stopped".into())),
            }
        }
        .boxed()
    })
}

async fn state_of(app: &App, token: &str, id: i64) -> String {
    let (_, body) = app
        .send(Req::get(&format!("/api/tasks/{id}")).token(token))
        .await;
    body["state"].as_str().unwrap().to_owned()
}

async fn until_state(app: &App, token: &str, id: i64, want: &str) {
    for _ in 0..200 {
        if state_of(app, token, id).await == want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("task {id} never reached {want}");
}

#[tokio::test]
async fn admins_list_tasks_newest_first() {
    let app = app().await;
    app.user("admin", true).await;
    app.user("bob", false).await;
    let token = app.login("admin").await;
    let bob = app.login("bob").await;
    app.exec(
        "INSERT INTO tasks (id, kind, state, payload, progress, message, created_at, updated_at) VALUES
           (1, 'scan', 'done', '{}', 1.0, NULL, 10, 20),
           (2, 'metadata', 'failed', '{}', NULL, 'DLsite 暂时无法访问', 30, 31),
           (3, 'scan', 'running', '{}', 0.35, '同步轨道 410/1200', 30, 40)",
    )
    .await;

    let (status, body) = app.send(Req::get("/api/tasks").token(&token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_same_shape_with_nullable(
        &body,
        &sample_in("m4", "tasks-page.json"),
        "tasks",
        &[
            "items[].progress",
            "items[].message",
            "items[].result",
            "items[].queue_position",
        ],
    );
    let ids: Vec<i64> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids, [3, 2, 1]);
    assert_eq!(body["items"][0]["message"], "同步轨道 410/1200");

    let (_, body) = app
        .send(Req::get("/api/tasks?state=failed").token(&token))
        .await;
    assert_eq!(body["total"], 1);
    let (_, body) = app
        .send(Req::get("/api/tasks?page_size=1&page=2").token(&token))
        .await;
    assert_eq!(body["items"][0]["id"], 2);

    let (status, err) = app
        .send(Req::get("/api/tasks?state=sleeping").token(&token))
        .await;
    assert_error(status, &err, StatusCode::BAD_REQUEST, "invalid_request");
    let (status, err) = app.send(Req::get("/api/tasks").token(&bob)).await;
    assert_error(status, &err, StatusCode::FORBIDDEN, "forbidden");
}

#[tokio::test]
async fn the_list_can_leave_out_one_kind() {
    let app = app().await;
    app.user("admin", true).await;
    let token = app.login("admin").await;
    app.exec(
        "INSERT INTO tasks (id, kind, state, payload, created_at, updated_at) VALUES
           (1, 'scan', 'done', '{}', 10, 20),
           (2, 'analysis', 'done', '{\"track_id\": 7}', 20, 21),
           (3, 'transcode', 'failed', '{}', 30, 31),
           (4, 'analysis', 'failed', '{\"track_id\": 8}', 40, 41),
           (5, 'analysis', 'done', '{\"track_id\": 9}', 50, 51)",
    )
    .await;
    let ids = |body: &Value| -> Vec<i64> {
        body["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_i64().unwrap())
            .collect()
    };

    let (status, body) = app
        .send(Req::get("/api/tasks?exclude_kind=analysis").token(&token))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(ids(&body), [3, 1]);
    assert_eq!(body["total"], 2);

    // The total counts with both filters, so paging stops at the last row.
    let (_, body) = app
        .send(Req::get("/api/tasks?exclude_kind=analysis&state=failed").token(&token))
        .await;
    assert_eq!(ids(&body), [3]);
    assert_eq!(body["total"], 1);
    let (_, body) = app
        .send(Req::get("/api/tasks?exclude_kind=scan&page_size=2&page=2").token(&token))
        .await;
    assert_eq!(ids(&body), [3, 2]);
    assert_eq!(body["total"], 4);

    let (_, body) = app.send(Req::get("/api/tasks").token(&token)).await;
    assert_eq!(body["total"], 5);
    let (status, err) = app
        .send(Req::get("/api/tasks?exclude_kind=loudness").token(&token))
        .await;
    assert_error(status, &err, StatusCode::BAD_REQUEST, "invalid_request");
}

#[tokio::test]
async fn cancel_running_and_refuse_finished_tasks() {
    let app = app().await;
    app.user("admin", true).await;
    let token = app.login("admin").await;
    let (_release, wait) = oneshot::channel();
    let id = app
        .state
        .scheduler()
        .submit(spec("a"), blocking(wait))
        .await
        .unwrap()
        .id;
    until_state(&app, &token, id, "running").await;

    let (status, _) = app
        .send(Req::post(&format!("/api/tasks/{id}/cancel")).token(&token))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    until_state(&app, &token, id, "cancelled").await;

    let (status, err) = app
        .send(Req::post(&format!("/api/tasks/{id}/cancel")).token(&token))
        .await;
    assert_error(status, &err, StatusCode::BAD_REQUEST, "invalid_request");
    let (status, err) = app
        .send(Req::post("/api/tasks/999/cancel").token(&token))
        .await;
    assert_error(status, &err, StatusCode::NOT_FOUND, "not_found");
}

async fn position_of(app: &App, token: &str, id: i64) -> Value {
    let (_, body) = app
        .send(Req::get(&format!("/api/tasks/{id}")).token(token))
        .await;
    body["queue_position"].clone()
}

async fn move_to_front(app: &App, token: &str, id: i64) -> (StatusCode, Value) {
    app.send(Req::post(&format!("/api/tasks/{id}/move-to-front")).token(token))
        .await
}

#[tokio::test]
async fn a_queued_scan_moves_to_the_front_and_runs_next() {
    let app = app().await;
    app.user("admin", true).await;
    app.user("bob", false).await;
    let token = app.login("admin").await;
    let scheduler = app.state.scheduler().clone();
    let (release_a, wait_a) = oneshot::channel();
    let a = scheduler
        .submit(scan_spec("a"), blocking(wait_a))
        .await
        .unwrap()
        .id;
    until_state(&app, &token, a, "running").await;
    let (_release_b, wait_b) = oneshot::channel();
    let b = scheduler
        .submit(scan_spec("b"), blocking(wait_b))
        .await
        .unwrap()
        .id;
    let (_release_c, wait_c) = oneshot::channel();
    let c = scheduler
        .submit(scan_spec("c"), blocking(wait_c))
        .await
        .unwrap()
        .id;

    assert_eq!(position_of(&app, &token, a).await, Value::Null);
    assert_eq!(position_of(&app, &token, b).await, 1);
    let (_, queued) = app
        .send(Req::get(&format!("/api/tasks/{c}")).token(&token))
        .await;
    assert_eq!(queued["queue_position"], 2);
    assert_same_shape(&queued, &sample_in("m4", "task-queued-scan.json"), "queued");

    let (status, body) = move_to_front(&app, &token, c).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(position_of(&app, &token, c).await, 1);
    assert_eq!(position_of(&app, &token, b).await, 2);
    let (_, list) = app
        .send(Req::get("/api/tasks?state=queued").token(&token))
        .await;
    let positions: Vec<(i64, i64)> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["id"].as_i64().unwrap(),
                t["queue_position"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(positions, [(c, 1), (b, 2)], "newest first, with positions");
    let (status, _) = move_to_front(&app, &token, c).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "already first");

    // The running scan keeps running, and the moved one starts after it.
    let (status, err) = move_to_front(&app, &token, a).await;
    assert_error(status, &err, StatusCode::BAD_REQUEST, "invalid_request");
    assert_eq!(state_of(&app, &token, a).await, "running");
    release_a.send(()).unwrap();
    until_state(&app, &token, c, "running").await;
    assert_eq!(state_of(&app, &token, b).await, "queued");
    assert_eq!(position_of(&app, &token, b).await, 1);
    assert_eq!(position_of(&app, &token, c).await, Value::Null);

    let (status, err) = move_to_front(&app, &token, a).await;
    assert_error(status, &err, StatusCode::BAD_REQUEST, "invalid_request");
    let (status, err) = move_to_front(&app, &token, 999).await;
    assert_error(status, &err, StatusCode::NOT_FOUND, "not_found");
    let (status, err) = app
        .send(Req::post(&format!("/api/tasks/{b}/move-to-front")).token(&app.login("bob").await))
        .await;
    assert_error(status, &err, StatusCode::FORBIDDEN, "forbidden");
}

#[tokio::test]
async fn only_scans_can_move_to_the_front() {
    let app = app().await;
    app.user("admin", true).await;
    let token = app.login("admin").await;
    let scheduler = app.state.scheduler().clone();
    let (_release, wait) = oneshot::channel();
    let running = scheduler
        .submit(
            TaskSpec {
                kind: "transcode",
                ..spec("t1")
            },
            blocking(wait),
        )
        .await
        .unwrap()
        .id;
    until_state(&app, &token, running, "running").await;
    let (_release2, wait2) = oneshot::channel();
    let queued = scheduler
        .submit(
            TaskSpec {
                kind: "transcode",
                ..spec("t2")
            },
            blocking(wait2),
        )
        .await
        .unwrap()
        .id;
    assert_eq!(state_of(&app, &token, queued).await, "queued");
    assert_eq!(position_of(&app, &token, queued).await, Value::Null);
    let (status, err) = move_to_front(&app, &token, queued).await;
    assert_error(status, &err, StatusCode::BAD_REQUEST, "invalid_request");
}

/// Reads SSE frames until the stream ends, with a deadline.
async fn read_events(app: &App, token: &str, id: i64, after_first: impl FnOnce()) -> Vec<Value> {
    let mut request = Request::builder()
        .uri(format!("/api/tasks/{id}/events"))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    request
        .extensions_mut()
        .insert(ConnectInfo::<std::net::SocketAddr>(
            "192.0.2.10:1".parse().unwrap(),
        ));
    let response = app.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/event-stream"
    );
    let mut body = response.into_body();
    let mut text = String::new();
    let mut events = Vec::new();
    let mut after_first = Some(after_first);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let frame = tokio::time::timeout_at(deadline, body.frame())
            .await
            .expect("the stream ended in time");
        let Some(frame) = frame else { break };
        if let Ok(data) = frame.unwrap().into_data() {
            text.push_str(std::str::from_utf8(&data).unwrap());
        }
        while let Some(end) = text.find("\n\n") {
            let chunk: String = text.drain(..end + 2).collect();
            if let Some(data) = chunk.lines().find_map(|l| l.strip_prefix("data: ")) {
                assert!(chunk.contains("event: snapshot"), "{chunk}");
                events.push(serde_json::from_str(data).unwrap());
                if let Some(f) = after_first.take() {
                    f();
                }
            }
        }
    }
    events
}

#[tokio::test]
async fn events_stream_snapshots_until_the_task_ends() {
    let app = app().await;
    app.user("admin", true).await;
    let token = app.login("admin").await;
    let (release, wait) = oneshot::channel();
    let id = app
        .state
        .scheduler()
        .submit(spec("b"), blocking(wait))
        .await
        .unwrap()
        .id;
    until_state(&app, &token, id, "running").await;

    let events = read_events(&app, &token, id, move || {
        let _ = release.send(());
    })
    .await;
    assert!(events.len() >= 2, "{events:?}");
    assert_eq!(events[0]["state"], "running");
    assert_eq!(events.last().unwrap()["state"], "done");
    for event in &events {
        assert_same_shape_with_nullable(
            event,
            &sample_in("m2", "task-running.json"),
            "snapshot",
            &["progress", "message", "result", "queue_position"],
        );
    }

    // A finished task answers with one final snapshot and ends.
    let events = read_events(&app, &token, id, || {}).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["state"], "done");

    let (status, err) = app
        .send(Req::get("/api/tasks/999/events").token(&token))
        .await;
    assert_error(status, &err, StatusCode::NOT_FOUND, "not_found");
}

#[tokio::test]
async fn events_stream_ends_at_shutdown_for_a_queued_task() {
    let app = app().await;
    app.user("admin", true).await;
    let token = app.login("admin").await;
    let (_release, wait) = oneshot::channel();
    let running = app
        .state
        .scheduler()
        .submit(spec("a"), blocking(wait))
        .await
        .unwrap()
        .id;
    until_state(&app, &token, running, "running").await;
    let (_release2, wait2) = oneshot::channel();
    let queued = app
        .state
        .scheduler()
        .submit(spec("b"), blocking(wait2))
        .await
        .unwrap()
        .id;
    assert_eq!(state_of(&app, &token, queued).await, "queued");

    // Shutdown leaves the queued task queued, so only the shutdown signal
    // can end this stream.
    let scheduler = app.state.scheduler().clone();
    let events = read_events(&app, &token, queued, move || scheduler.begin_shutdown()).await;
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["state"], "queued");
}

#[tokio::test]
async fn event_streams_are_limited() {
    let app = app().await;
    app.user("admin", true).await;
    let token = app.login("admin").await;
    let (_release, wait) = oneshot::channel();
    let id = app
        .state
        .scheduler()
        .submit(spec("limit"), blocking(wait))
        .await
        .unwrap()
        .id;
    let open = || {
        let mut request = Request::builder()
            .uri(format!("/api/tasks/{id}/events"))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo::<std::net::SocketAddr>(
                "192.0.2.10:1".parse().unwrap(),
            ));
        app.router.clone().oneshot(request)
    };
    let mut streams = Vec::new();
    for _ in 0..neo_server::http::MAX_EVENT_STREAMS {
        let response = open().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        streams.push(response);
    }
    let response = open().await.unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let body: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["error"]["code"], "queue_full");

    // A closed stream frees its slot.
    streams.pop();
    assert_eq!(open().await.unwrap().status(), StatusCode::OK);
}

#[tokio::test]
async fn a_cancelled_scan_keeps_its_place_until_its_row_ends() {
    let app = app().await;
    app.user("admin", true).await;
    let token = app.login("admin").await;
    let scheduler = app.state.scheduler().clone();
    let (_release_a, wait_a) = oneshot::channel();
    let a = scheduler
        .submit(scan_spec("a"), blocking(wait_a))
        .await
        .unwrap()
        .id;
    until_state(&app, &token, a, "running").await;
    let (_release_b, wait_b) = oneshot::channel();
    let b = scheduler
        .submit(scan_spec("b"), blocking(wait_b))
        .await
        .unwrap()
        .id;
    // The only write connection is taken, so b's end cannot be written.
    let writer = app.db.sqlite().unwrap().write.acquire().await.unwrap();
    assert!(scheduler.cancel(b).await.unwrap());
    tokio::time::sleep(Duration::from_millis(300)).await;
    // Its row still says queued, so it must still have a place.
    assert_eq!(scheduler.queue_position(b), Some(1));
    drop(writer);
    until_state(&app, &token, b, "cancelled").await;
    // The place goes when the runner returns, just after the row ends.
    for _ in 0..200 {
        if scheduler.queue_position(b).is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(scheduler.queue_position(b), None);
}

#[tokio::test]
async fn events_stream_a_new_snapshot_when_the_queue_position_changes() {
    let app = app().await;
    app.user("admin", true).await;
    let token = app.login("admin").await;
    let scheduler = app.state.scheduler().clone();
    let (_release_a, wait_a) = oneshot::channel();
    let a = scheduler
        .submit(scan_spec("a"), blocking(wait_a))
        .await
        .unwrap()
        .id;
    until_state(&app, &token, a, "running").await;
    let (_release_b, wait_b) = oneshot::channel();
    let b = scheduler
        .submit(scan_spec("b"), blocking(wait_b))
        .await
        .unwrap()
        .id;
    let (_release_c, wait_c) = oneshot::channel();
    let c = scheduler
        .submit(scan_spec("c"), blocking(wait_c))
        .await
        .unwrap()
        .id;

    // Nothing in b's row changes when c moves ahead of it.
    let events = read_events(&app, &token, b, move || {
        tokio::spawn(async move {
            scheduler.move_to_front(c).await.unwrap();
            // Long enough for a stream poll to see the move.
            tokio::time::sleep(Duration::from_millis(1500)).await;
            scheduler.cancel(b).await.unwrap();
        });
    })
    .await;
    let seen: Vec<(&str, &Value)> = events
        .iter()
        .map(|e| (e["state"].as_str().unwrap(), &e["queue_position"]))
        .collect();
    assert_eq!(
        seen,
        [
            ("queued", &json!(1)),
            ("queued", &json!(2)),
            ("cancelled", &Value::Null)
        ]
    );
}
