//! Manual metadata edits, the missing list and cleanup.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::{Value, json};

async fn setup() -> (App, i64, String) {
    let app = app().await;
    let admin = app.user("admin", true).await;
    let token = app.login("admin").await;
    app.exec(
        r#"INSERT INTO roots (id, display_name) VALUES ('lib', 'Library'), ('cloud', 'Cloud');
           INSERT INTO circles (id, name) VALUES (3, 'Known Circle');
           INSERT INTO vas (id, name) VALUES ('va-1', 'Known Voice');
           INSERT INTO tags (id, name) VALUES (7, 'ASMR');
           INSERT INTO tag_aliases VALUES ('ASMR', 7), ('Ear Licking', 7);
           INSERT INTO works (id, code, title, metadata_src, created_at, updated_at) VALUES
             (1, 'RJ01000001', 'dir name', 'none', 0, 0),
             (2, 'RJ01000002', 'gone', 'dlsite', 0, 0),
             (3, 'RJ01000003', 'half gone', 'dlsite', 0, 0);
           UPDATE works SET missing_at = 50 WHERE id = 2;
           INSERT INTO work_locations (id, work_id, root_id, rel_path, display_path, missing_at) VALUES
             (10, 2, 'lib', X'02', 'gone', 50),
             (11, 3, 'lib', X'03', 'here', NULL),
             (12, 3, 'cloud', X'04', 'there', 60);
           INSERT INTO tracks (id, work_id, location_id, rel_path, display_path, sort_key, kind, size, missing_at, duplicate_of)
             VALUES (100, 2, 10, X'61', 'a.mp3', 'a', 'audio', 1, 50, NULL),
                    (110, 3, 11, X'61', 'a.mp3', 'a', 'audio', 1, NULL, NULL),
                    (120, 3, 12, X'61', 'a.mp3', 'a', 'audio', 1, 60, 110),
                    (121, 3, 12, X'62', 'b.mp3', 'b', 'audio', 1, 60, NULL);
           INSERT INTO favorites VALUES (1, 2, 0);"#,
    )
    .await;
    (app, admin, token)
}

#[tokio::test]
async fn manual_edits_replace_metadata_and_mark_it_manual() {
    let (app, _, token) = setup().await;
    let body = json!({
        "title": "  Fixed title ",
        "release_date": "2024-05-01",
        "nsfw": true,
        "circle_name": "New Circle",
        "va_names": ["Known Voice", "New Voice", "New Voice", " "],
        "tag_names": ["Ear Licking", "Brand New"],
    });
    let (status, detail) = app
        .send(Req::put("/api/works/1/metadata").token(&token).json(body))
        .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_same_shape_with_nullable(
        &detail,
        &sample_in("m2", "work-detail.json"),
        "detail",
        WORK_DETAIL_NULLABLE,
    );
    assert!(
        detail["locations"].is_array(),
        "the answer goes to an admin: {detail}"
    );
    assert_eq!(detail["title"], "Fixed title");
    assert_eq!(detail["release_date"], "2024-05-01");
    assert_eq!(detail["nsfw"], true);
    assert_eq!(detail["circle"]["name"], "New Circle");
    assert!(detail["circle"]["id"].as_i64().unwrap() >= 9_000_000_000);
    let vas: Vec<&str> = detail["vas"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["name"].as_str().unwrap())
        .collect();
    assert_eq!(vas, ["Known Voice", "New Voice"]);
    assert_eq!(
        detail["vas"][0]["id"], "va-1",
        "existing voice actors are reused"
    );
    let tag_ids: Vec<i64> = detail["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_i64().unwrap())
        .collect();
    assert!(
        tag_ids.contains(&7),
        "an alias resolves to its canonical tag"
    );
    assert!(tag_ids.iter().any(|id| *id >= 1_000_000));
    let src: String = sqlx::query_scalar("SELECT metadata_src FROM works WHERE id = 1")
        .fetch_one(&app.db.sqlite().unwrap().read)
        .await
        .unwrap();
    assert_eq!(src, "manual");

    // The sample request is accepted as is.
    let (status, _) = app
        .send(
            Req::put("/api/works/1/metadata")
                .token(&token)
                .json(sample_in("m4", "metadata-request.json")),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn bad_edits_are_refused() {
    let (app, _, token) = setup().await;
    let base = json!({"title": "t", "release_date": null, "nsfw": false,
                      "circle_name": null, "va_names": [], "tag_names": []});
    let with = |key: &str, value: Value| {
        let mut b = base.clone();
        b[key] = value;
        b
    };
    for body in [
        with("title", json!("   ")),
        with("release_date", json!("2024-13-40")),
        with("release_date", json!("2024-02-31")),
        with("release_date", json!("2023-02-29")),
        with("release_date", json!("+024-+1-+1")),
        with("va_names", json!(["bell\u{7}"])),
    ] {
        let (status, err) = app
            .send(Req::put("/api/works/1/metadata").token(&token).json(body))
            .await;
        assert_error(status, &err, StatusCode::BAD_REQUEST, "invalid_request");
    }
    let (status, err) = app
        .send(
            Req::put("/api/works/99/metadata")
                .token(&token)
                .json(base.clone()),
        )
        .await;
    assert_error(status, &err, StatusCode::NOT_FOUND, "not_found");
    app.user("bob", false).await;
    let bob = app.login("bob").await;
    let (status, err) = app
        .send(Req::put("/api/works/1/metadata").token(&bob).json(base))
        .await;
    assert_error(status, &err, StatusCode::FORBIDDEN, "forbidden");
}

#[tokio::test]
async fn missing_list_and_cleanup() {
    let (app, admin, token) = setup().await;
    app.exec(&format!(
        "INSERT INTO play_progress (user_id, work_id, track_id, updated_at) VALUES ({admin}, 3, 121, 0);
         INSERT INTO import_pending (user_id, work_id, source, state) VALUES ({admin}, 3, '{{}}', 'pending');
         -- A scan restored a location of work 4 but has not cleared the
         -- work's own flag yet.
         INSERT INTO works (id, code, title, metadata_src, missing_at, created_at, updated_at)
           VALUES (4, 'RJ01000004', 'coming back', 'none', 50, 0, 0);
         INSERT INTO work_locations (id, work_id, root_id, rel_path, display_path, missing_at)
           VALUES (13, 4, 'lib', X'05', 'back', NULL);"
    ))
    .await;
    std::fs::write(app.data_dir().join("covers/2.jpg"), b"cover").unwrap();
    std::fs::write(app.data_dir().join("cache/thumbs/2-0.jpg"), b"thumb").unwrap();

    let (status, report) = app.send(Req::get("/api/admin/missing").token(&token)).await;
    assert_eq!(status, StatusCode::OK, "{report}");
    assert_same_shape(&report, &sample_in("m4", "missing.json"), "missing");
    assert_eq!(report["works"][0]["id"], 2);
    assert_eq!(
        report["works"].as_array().unwrap().len(),
        1,
        "work 4 is coming back"
    );
    // Location 10 belongs to a work missing everywhere, so only 12 is listed.
    assert_eq!(report["locations"].as_array().unwrap().len(), 1);
    assert_eq!(report["locations"][0]["id"], 12);
    assert_eq!(report["locations"][0]["path"], "there");

    // Work 1 is not missing and location 11 is present: both are skipped.
    let (status, done) = app
        .send(
            Req::post("/api/admin/missing/cleanup")
                .token(&token)
                .json(json!({"work_ids": [2, 1, 4], "location_ids": [12, 11]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_same_shape(&done, &sample_in("m4", "cleanup-response.json"), "cleanup");
    assert_eq!(done, json!({"works_deleted": 1, "locations_deleted": 1}));

    let count = |sql: &'static str| {
        let pool = app.db.sqlite().unwrap().read.clone();
        async move {
            sqlx::query_scalar::<_, i64>(sql)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    assert_eq!(
        count("SELECT COUNT(*) FROM works").await,
        3,
        "work 4 is back"
    );
    assert_eq!(count("SELECT COUNT(*) FROM favorites").await, 0);
    assert_eq!(count("SELECT COUNT(*) FROM work_locations").await, 2);
    assert_eq!(
        count("SELECT COUNT(*) FROM import_pending WHERE work_id = 3 AND state = 'superseded'")
            .await,
        1,
        "progress on a deleted track still counts as newer playback"
    );
    assert_eq!(count("SELECT COUNT(*) FROM tracks").await, 1);
    assert_eq!(
        count("SELECT COUNT(*) FROM play_progress WHERE work_id = 3 AND track_id IS NULL").await,
        1,
        "progress on a deleted track keeps its row without the track"
    );
    assert!(!app.data_dir().join("covers/2.jpg").exists());
    assert!(!app.data_dir().join("cache/thumbs/2-0.jpg").exists());

    app.user("bob", false).await;
    let bob = app.login("bob").await;
    let (status, err) = app.send(Req::get("/api/admin/missing").token(&bob)).await;
    assert_error(status, &err, StatusCode::FORBIDDEN, "forbidden");
}

#[tokio::test]
async fn cleanup_waits_for_a_backup_of_the_covers() {
    let (app, _, token) = setup().await;
    std::fs::write(app.data_dir().join("covers/2.jpg"), b"cover").unwrap();
    // A backup holds this lock from its database copy to its covers copy.
    let backup = neo_scanner::metadata::lock_covers(&app.data_dir().join("covers")).unwrap();
    let cleanup = app.send(
        Req::post("/api/admin/missing/cleanup")
            .token(&token)
            .json(json!({"work_ids": [2], "location_ids": []})),
    );
    let during = async {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE id = 2")
            .fetch_one(&app.db.sqlite().unwrap().read)
            .await
            .unwrap();
        let cover = app.data_dir().join("covers/2.jpg").exists();
        drop(backup);
        (rows, cover)
    };
    let ((status, done), during) = tokio::join!(cleanup, during);
    assert_eq!(
        during,
        (1, true),
        "the work and its cover stay while the backup copies"
    );
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["works_deleted"], 1);
    assert!(!app.data_dir().join("covers/2.jpg").exists());
}

#[tokio::test]
async fn cleanup_refuses_while_a_scan_runs() {
    use futures_util::FutureExt;
    use neo_server::tasks::{Job, Lane, TaskSpec};
    let (app, _, token) = setup().await;
    let (release, wait) = tokio::sync::oneshot::channel::<()>();
    let job: Job = Box::new(move |_| {
        async move {
            let _ = wait.await;
            Ok(None)
        }
        .boxed()
    });
    let spec = TaskSpec {
        kind: "scan",
        dedupe_key: None,
        payload: json!({}),
        lane: Lane::Scan,
        timeout: None,
        queue_limit: None,
    };
    let id = app.state.scheduler().submit(spec, job).await.unwrap().id;
    for _ in 0..200 {
        let (_, task) = app
            .send(Req::get(&format!("/api/tasks/{id}")).token(&token))
            .await;
        if task["state"] == "running" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let cleanup = || {
        Req::post("/api/admin/missing/cleanup")
            .token(&token)
            .json(json!({"work_ids": [2], "location_ids": []}))
    };
    let (status, err) = app.send(cleanup()).await;
    assert_error(status, &err, StatusCode::CONFLICT, "scan_running");

    release.send(()).unwrap();
    for _ in 0..200 {
        let (status, done) = app.send(cleanup()).await;
        if status == StatusCode::OK {
            assert_eq!(done["works_deleted"], 1);
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("cleanup never got the scan slot");
}

#[tokio::test]
async fn editing_a_work_with_too_many_tracks_says_so() {
    let (app, _, token) = setup().await;
    app.exec(
        "INSERT INTO work_locations (id, work_id, root_id, rel_path, display_path) VALUES (20, 1, 'lib', X'01', 'big');
         WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 5001)
         INSERT INTO tracks (work_id, location_id, rel_path, display_path, sort_key, kind, size)
           SELECT 1, 20, CAST(i AS BLOB), i || '.mp3', printf('%05d', i), 'audio', 1 FROM n;",
    )
    .await;
    let body = json!({"title": "t", "release_date": null, "nsfw": false,
                      "circle_name": null, "va_names": [], "tag_names": []});
    let (status, err) = app
        .send(Req::put("/api/works/1/metadata").token(&token).json(body))
        .await;
    assert_error(status, &err, StatusCode::PAYLOAD_TOO_LARGE, "too_large");
}

#[tokio::test]
async fn cleanup_takes_a_bounded_number_of_ids() {
    let (app, _, token) = setup().await;
    let ids: Vec<i64> = (1..=1001).collect();
    let (status, err) = app
        .send(
            Req::post("/api/admin/missing/cleanup")
                .token(&token)
                .json(json!({"work_ids": ids, "location_ids": []})),
        )
        .await;
    assert_error(status, &err, StatusCode::BAD_REQUEST, "invalid_request");
}
