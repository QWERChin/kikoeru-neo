mod common;

use axum::http::StatusCode;
use common::{App, Req, app};
use serde_json::{Value, json};

/// Two synthetic works. Work 1 has a group `mp3/SE有` with three audio
/// tracks (the last by natural order is 13), a missing track sorted after
/// them, a subdirectory, and a text file. Work 2 has one track.
async fn library() -> App {
    let app = app().await;
    app.user("admin", true).await;
    app.user("alice", false).await;
    app.exec(
        "INSERT INTO roots (id, display_name) VALUES ('local', 'Local');
         INSERT INTO works (id, code, title, metadata_src, created_at, updated_at) VALUES
           (1, 'RJ000001', 'Work one', 'scan', 0, 0),
           (2, 'RJ000002', 'Work two', 'scan', 0, 0);
         INSERT INTO work_locations (id, work_id, root_id, rel_path, display_path) VALUES
           (1, 1, 'local', X'524A303031', 'RJ001'),
           (2, 2, 'local', X'524A303032', 'RJ002');
         INSERT INTO tracks (id, work_id, location_id, rel_path, display_path, sort_key, kind, missing_at) VALUES
           (11, 1, 1, X'01', 'mp3/SE有/Tr1.mp3',  'mp3/se有/tr0000000001.mp3', 'audio', NULL),
           (12, 1, 1, X'02', 'mp3/SE有/Tr2.mp3',  'mp3/se有/tr0000000002.mp3', 'audio', NULL),
           (13, 1, 1, X'03', 'mp3/SE有/Tr10.mp3', 'mp3/se有/tr0000000010.mp3', 'audio', NULL),
           (14, 1, 1, X'04', 'mp3/SE有/Tr11.mp3', 'mp3/se有/tr0000000011.mp3', 'audio', 1),
           (15, 1, 1, X'05', 'mp3/SE有/z/Tr1.mp3', 'mp3/se有/z/tr0000000001.mp3', 'audio', NULL),
           (16, 1, 1, X'06', 'mp3/SE有/readme.txt', 'mp3/se有/readme.txt', 'text', NULL),
           (17, 1, 1, X'07', 'top.mp3', 'top.mp3', 'audio', NULL),
           (21, 2, 2, X'08', 'only.mp3', 'only.mp3', 'audio', NULL);",
    )
    .await;
    app
}

async fn progress(app: &App, user: &str, work: i64) -> Value {
    let (status, body) = app
        .send(Req::get("/api/works?page=1").token(&app.login(user).await))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["id"] == work)
        .unwrap()["progress"]
        .clone()
}

#[tokio::test]
async fn a_report_is_stored_and_shows_up_in_the_work_list() {
    let app = library().await;
    let token = app.login("alice").await;
    let (status, body) = app
        .send(
            Req::put("/api/me/progress/1")
                .token(&token)
                .json(json!({"track_id": 12, "position_ms": 812000, "group_path": "mp3/SE有"})),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let p = progress(&app, "alice", 1).await;
    assert_eq!(p["track_id"], 12);
    assert_eq!(p["position_ms"], 812000);
    assert_eq!(p["group_path"], "mp3/SE有");
    assert_eq!(p["finished"], false);
    assert_eq!(
        progress(&app, "admin", 1).await,
        Value::Null,
        "progress is per user"
    );
}

#[tokio::test]
async fn a_track_from_another_work_is_rejected() {
    let app = library().await;
    let token = app.login("alice").await;
    for track in [21, 999] {
        let (status, body) = app
            .send(
                Req::put("/api/me/progress/1")
                    .token(&token)
                    .json(json!({"track_id": track, "position_ms": 1, "group_path": ""})),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["error"]["code"], "invalid_request");
    }
}

#[tokio::test]
async fn finishing_requires_the_last_playable_track_of_the_group() {
    let app = library().await;
    let token = app.login("alice").await;
    let finish = |track: i64, group: &str| {
        Req::post("/api/me/progress/1/finish")
            .token(&token)
            .json(json!({"track_id": track, "group_path": group, "manual": false}))
    };
    // Track 12 is not last, 14 is missing, 15 is in a subdirectory.
    for track in [11, 12, 14, 15, 16] {
        let (status, body) = app.send(finish(track, "mp3/SE有")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "track {track}: {body}");
    }
    let (status, body) = app.send(finish(13, "mp3/SE有")).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let p = progress(&app, "alice", 1).await;
    assert_eq!(p["finished"], true);
    assert_eq!(p["replaying"], false);
    assert_eq!(p["track_id"], 13);

    // The top directory is the group "", and a trailing slash is accepted.
    let (status, _) = app.send(finish(17, "")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = app.send(finish(13, "mp3/SE有/")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

/// A player that queued the tracks before a scan hid them as copies of
/// another location's files still reports their old IDs. Those land on the
/// copy the scan kept, which the detail page lists, so the work resumes and
/// finishes as if the player had used it.
#[tokio::test]
async fn reports_on_a_hidden_duplicate_go_to_the_kept_copy() {
    let app = library().await;
    app.exec(
        "INSERT INTO roots (id, display_name) VALUES ('remote', 'Remote');
         INSERT INTO work_locations (id, work_id, root_id, rel_path, display_path) VALUES
           (3, 1, 'remote', X'524A303031', 'RJ001');
         UPDATE tracks SET size = 1 WHERE id IN (12, 13);
         INSERT INTO tracks (id, work_id, location_id, rel_path, display_path, sort_key, kind, size, duplicate_of) VALUES
           (32, 1, 3, X'02', 'mp3/SE有/Tr2.mp3',  'mp3/se有/tr0000000002.mp3', 'audio', 1, 12),
           (33, 1, 3, X'03', 'mp3/SE有/Tr10.mp3', 'mp3/se有/tr0000000010.mp3', 'audio', 1, 13);",
    )
    .await;
    let token = app.login("alice").await;
    let (status, body) = app
        .send(
            Req::put("/api/me/progress/1")
                .token(&token)
                .json(json!({"track_id": 32, "position_ms": 5000, "group_path": "mp3/SE有"})),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let p = progress(&app, "alice", 1).await;
    assert_eq!(
        (&p["track_id"], &p["position_ms"]),
        (&json!(12), &json!(5000))
    );

    // The hidden copy of the group's last track ends the group too.
    let (status, body) = app
        .send(
            Req::post("/api/me/progress/1/finish")
                .token(&token)
                .json(json!({"track_id": 33, "group_path": "mp3/SE有", "manual": false})),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let p = progress(&app, "alice", 1).await;
    assert_eq!((&p["finished"], &p["track_id"]), (&json!(true), &json!(13)));
}

#[tokio::test]
async fn manual_marking_skips_the_last_track_check() {
    let app = library().await;
    let token = app.login("alice").await;
    let (status, body) = app
        .send(
            Req::post("/api/me/progress/1/finish")
                .token(&token)
                .json(json!({"track_id": 11, "group_path": "mp3/SE有", "manual": true})),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    assert_eq!(progress(&app, "alice", 1).await["finished"], true);
}

#[tokio::test]
async fn playing_a_finished_work_marks_it_as_replaying_until_finished_again() {
    let app = library().await;
    let token = app.login("alice").await;
    let report = |track: i64, position: i64| {
        Req::put("/api/me/progress/1")
            .token(&token)
            .json(json!({"track_id": track, "position_ms": position, "group_path": "mp3/SE有"}))
    };
    app.send(report(13, 5000)).await;
    app.send(
        Req::post("/api/me/progress/1/finish")
            .token(&token)
            .json(json!({"track_id": 13, "group_path": "mp3/SE有"})),
    )
    .await;
    let p = progress(&app, "alice", 1).await;
    assert_eq!(
        (p["finished"].clone(), p["replaying"].clone()),
        (json!(true), json!(false))
    );
    assert_eq!(p["position_ms"], 5000, "the finished position is kept");

    app.send(report(11, 1000)).await;
    let p = progress(&app, "alice", 1).await;
    assert_eq!(
        (p["finished"].clone(), p["replaying"].clone()),
        (json!(true), json!(true))
    );

    app.send(
        Req::post("/api/me/progress/1/finish")
            .token(&token)
            .json(json!({"track_id": 13, "group_path": "mp3/SE有"})),
    )
    .await;
    assert_eq!(progress(&app, "alice", 1).await["replaying"], false);
}

#[tokio::test]
async fn clearing_removes_the_progress_and_is_idempotent() {
    let app = library().await;
    let token = app.login("alice").await;
    app.send(
        Req::put("/api/me/progress/1")
            .token(&token)
            .json(json!({"track_id": 11, "position_ms": 1, "group_path": "mp3/SE有"})),
    )
    .await;
    for _ in 0..2 {
        let (status, _) = app
            .send(Req::delete("/api/me/progress/1").token(&token))
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }
    assert_eq!(progress(&app, "alice", 1).await, Value::Null);
}

#[tokio::test]
async fn progress_endpoints_need_an_api_token_and_valid_input() {
    let app = library().await;
    let (status, _) = app.send(Req::delete("/api/me/progress/1")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let token = app.login("alice").await;
    let (status, body) = app
        .send(
            Req::put("/api/me/progress/1")
                .token(&token)
                .json(json!({"track_id": 11, "position_ms": -5, "group_path": ""})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = app
        .send(
            Req::put("/api/me/progress/abc")
                .token(&token)
                .json(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"]["code"], "invalid_request");
}

#[tokio::test]
async fn group_paths_have_a_length_limit() {
    let app = library().await;
    let token = app.login("alice").await;
    let long = "a".repeat(4097);
    for (path, body) in [
        (
            "/api/me/progress/1",
            json!({"track_id": 11, "position_ms": 1, "group_path": long}),
        ),
        (
            "/api/me/progress/1/finish",
            json!({"track_id": 11, "group_path": long, "manual": true}),
        ),
    ] {
        let request = if path.ends_with("finish") {
            Req::post(path)
        } else {
            Req::put(path)
        };
        let (status, err) = app.send(request.token(&token).json(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{err}");
        assert_eq!(err["error"]["code"], "invalid_request");
    }
}

const SESSION: &str = "a3f0d725817c4b98b6e12c049d35af70";

fn ordered_report(sequence: i64, position: i64) -> Value {
    json!({
        "track_id": 12, "position_ms": position, "group_path": "mp3/SE有",
        "report_order": {"session_id": SESSION, "sequence": sequence}
    })
}

#[tokio::test]
async fn stale_reports_do_not_replace_newer_progress() {
    let app = library().await;
    let token = app.login("alice").await;
    for (sequence, position) in [(2, 110_000), (1, 100_000), (2, 90_000)] {
        let (status, body) = app
            .send(
                Req::put("/api/me/progress/1")
                    .token(&token)
                    .json(ordered_report(sequence, position)),
            )
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
        let p = progress(&app, "alice", 1).await;
        assert_eq!(p["position_ms"], 110_000);
        assert!(p.get("report_order").is_none());
        assert!(p.get("session_id").is_none());
        assert!(p.get("sequence").is_none());
        assert!(p.get("last_sequence").is_none());
    }
}

#[tokio::test]
async fn invalid_report_order_is_rejected_before_stale_detection() {
    let app = library().await;
    let token = app.login("alice").await;
    let put = |body| Req::put("/api/me/progress/1").token(&token).json(body);
    assert_eq!(
        app.send(put(ordered_report(2, 1))).await.0,
        StatusCode::NO_CONTENT
    );
    let mut invalid = vec![
        json!({}),
        json!(true),
        json!([]),
        json!("order"),
        json!({"session_id": SESSION}),
        json!({"sequence": 1}),
        json!({"session_id": null, "sequence": 1}),
        json!({"session_id": 123, "sequence": 1}),
    ];
    for session in [
        "".to_owned(),
        "a".repeat(31),
        "a".repeat(33),
        "g".repeat(32),
        SESSION.to_uppercase(),
        "é".repeat(16),
    ] {
        invalid.push(json!({"session_id": session, "sequence": 1}));
    }
    for sequence in [
        json!(0),
        json!(-1),
        json!(1.5),
        json!("1"),
        json!(null),
        json!(true),
        json!(9_007_199_254_740_992_i64),
    ] {
        invalid.push(json!({"session_id": SESSION, "sequence": sequence}));
    }
    for order in invalid {
        let mut report = ordered_report(1, 0);
        report["report_order"] = order.clone();
        let (status, body) = app.send(put(report)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{order}: {body}");
        assert_eq!(body["error"]["code"], "invalid_request");
    }
}

#[tokio::test]
async fn report_sequences_allow_gaps_and_the_safe_integer_limit_per_user_and_work() {
    let app = library().await;
    let alice = app.login("alice").await;
    let admin = app.login("admin").await;
    for (token, work, track, sequence, position) in [
        (&alice, 1, 12, 2, 110_000),
        (&alice, 1, 12, 9_007_199_254_740_991_i64, 90_000),
        (&admin, 1, 11, 1, 50_000),
        (&alice, 2, 21, 1, 40_000),
    ] {
        let mut report = ordered_report(sequence, position);
        report["track_id"] = json!(track);
        let (status, body) = app
            .send(
                Req::put(&format!("/api/me/progress/{work}"))
                    .token(token)
                    .json(report),
            )
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    }
    assert_eq!(progress(&app, "alice", 1).await["position_ms"], 90_000);
    assert_eq!(progress(&app, "admin", 1).await["position_ms"], 50_000);
    assert_eq!(progress(&app, "alice", 2).await["position_ms"], 40_000);
}

#[tokio::test]
async fn legacy_reports_preserve_the_ordered_watermark() {
    let app = library().await;
    let token = app.login("alice").await;
    let put = |body| Req::put("/api/me/progress/1").token(&token).json(body);
    assert_eq!(
        app.send(put(ordered_report(2, 110_000))).await.0,
        StatusCode::NO_CONTENT
    );
    for omit in [false, true] {
        let mut legacy = ordered_report(1, 80_000);
        if omit {
            legacy.as_object_mut().unwrap().remove("report_order");
        } else {
            legacy["report_order"] = Value::Null;
        }
        assert_eq!(app.send(put(legacy)).await.0, StatusCode::NO_CONTENT);
        assert_eq!(progress(&app, "alice", 1).await["position_ms"], 80_000);
        assert_eq!(
            app.send(put(ordered_report(1, 100_000))).await.0,
            StatusCode::NO_CONTENT
        );
        assert_eq!(progress(&app, "alice", 1).await["position_ms"], 80_000);
    }
}

#[tokio::test]
async fn stale_reports_still_require_authentication_and_valid_progress() {
    let app = library().await;
    let token = app.login("alice").await;
    let put = |body| Req::put("/api/me/progress/1").token(&token).json(body);
    assert_eq!(
        app.send(put(ordered_report(2, 110_000))).await.0,
        StatusCode::NO_CONTENT
    );
    let (status, body) = app
        .send(Req::put("/api/me/progress/1").json(ordered_report(1, 0)))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    let (status, body) = app
        .send(Req::post("/api/auth/media-token").token(&token))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = app
        .send(
            Req::put("/api/me/progress/1")
                .token(body["token"].as_str().unwrap())
                .json(ordered_report(1, 0)),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    for (field, value) in [
        ("track_id", json!(21)),
        ("track_id", json!(999)),
        ("position_ms", json!(-1)),
        ("group_path", json!("a".repeat(4097))),
    ] {
        let mut report = ordered_report(1, 0);
        report[field] = value;
        let (status, body) = app.send(put(report)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["error"]["code"], "invalid_request");
    }
    assert_eq!(progress(&app, "alice", 1).await["position_ms"], 110_000);
}
