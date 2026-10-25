mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::{Value, json};

/// Inserts works `RJ00000001..=n` with `created_at` equal to the number, so
/// the newest work has the highest id.
async fn seed_works(app: &App, n: i64) {
    let mut sql = String::new();
    for i in 1..=n {
        sql.push_str(&format!(
            "INSERT INTO works (id, code, title, metadata_src, created_at, updated_at)
             VALUES ({i}, 'RJ{i:08}', 'Work {i}', 'none', {i}, {i});"
        ));
    }
    app.exec(&sql).await;
}

fn ids(body: &Value) -> Vec<i64> {
    body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["id"].as_i64().unwrap())
        .collect()
}

async fn list(app: &App, token: &str, query: &str) -> Value {
    let (status, body) = app
        .send(Req::get(&format!("/api/works{query}")).token(token))
        .await;
    assert_eq!(status, StatusCode::OK, "{query}: {body}");
    assert_same_shape_with_nullable(
        &body,
        &sample("works-page.json"),
        "works",
        WORK_PAGE_NULLABLE,
    );
    body
}

#[tokio::test]
async fn requires_api_token() {
    let app = app().await;
    let (status, body) = app.send(Req::get("/api/works")).await;
    assert_error(status, &body, StatusCode::UNAUTHORIZED, "unauthorized");
}

#[tokio::test]
async fn paging_and_order() {
    let app = app().await;
    app.user("alice", true).await;
    let token = app.login("alice").await;
    seed_works(&app, 45).await;

    let first = list(&app, &token, "").await;
    assert_eq!(first["page"], 1);
    assert_eq!(first["page_size"], 40);
    assert_eq!(first["total"], 45);
    assert_eq!(ids(&first), (6..=45).rev().collect::<Vec<_>>());

    let second = list(&app, &token, "?page=2").await;
    assert_eq!(ids(&second), (1..=5).rev().collect::<Vec<_>>());

    let asc = list(&app, &token, "?page=2&page_size=10&sort=added&order=asc").await;
    assert_eq!(ids(&asc), (11..=20).collect::<Vec<_>>());

    let beyond = list(&app, &token, "?page=9").await;
    assert_eq!(ids(&beyond), Vec::<i64>::new());
    assert_eq!(beyond["total"], 45);
}

#[tokio::test]
async fn equal_times_break_ties_by_id() {
    let app = app().await;
    app.user("alice", true).await;
    let token = app.login("alice").await;
    seed_works(&app, 6).await;
    app.exec("UPDATE works SET created_at = 100").await;
    let mut seen = vec![];
    for page in 1..=3 {
        seen.extend(ids(&list(
            &app,
            &token,
            &format!("?page={page}&page_size=2"),
        )
        .await));
    }
    assert_eq!(seen, [6, 5, 4, 3, 2, 1]);
    let asc = list(&app, &token, "?order=asc").await;
    assert_eq!(ids(&asc), [1, 2, 3, 4, 5, 6]);
}

#[tokio::test]
async fn page_size_is_capped() {
    let app = app().await;
    app.user("alice", true).await;
    let token = app.login("alice").await;
    seed_works(&app, 105).await;
    let body = list(&app, &token, "?page_size=500").await;
    assert_eq!(body["page_size"], 100);
    assert_eq!(ids(&body).len(), 100);
    let body = list(&app, &token, "?page=2&page_size=101").await;
    assert_eq!(ids(&body), (1..=5).rev().collect::<Vec<_>>());
}

#[tokio::test]
async fn invalid_parameters() {
    let app = app().await;
    app.user("alice", true).await;
    let token = app.login("alice").await;
    for query in [
        "?page=0",
        "?page=-1",
        "?page=abc",
        "?page=1.5",
        "?page_size=0",
        "?sort=random",
        "?sort=newest",
        "?order=up",
        "?page=1&page=2",
    ] {
        let (status, body) = app
            .send(Req::get(&format!("/api/works{query}")).token(&token))
            .await;
        assert_error(status, &body, StatusCode::BAD_REQUEST, "invalid_request");
    }
    // Unknown parameters are ignored rather than rejected.
    list(&app, &token, "?unknown=x").await;
}

#[tokio::test]
async fn missing_works_are_excluded() {
    let app = app().await;
    app.user("alice", true).await;
    let token = app.login("alice").await;
    seed_works(&app, 3).await;
    app.exec("UPDATE works SET missing_at = 5 WHERE id = 2")
        .await;
    let body = list(&app, &token, "").await;
    assert_eq!(ids(&body), [3, 1]);
    assert_eq!(body["total"], 2);
    assert!(
        body["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|w| w["missing"] == false)
    );
}

#[tokio::test]
async fn metadata_fields() {
    let app = app().await;
    app.user("alice", true).await;
    let token = app.login("alice").await;
    app.exec(
        "INSERT INTO circles (id, name) VALUES (3, 'Circle');
         INSERT INTO works (id, code, title, circle_id, release_date, nsfw, rate_average,
                            rate_count, metadata_src, cover_src, cover_version, created_at, updated_at)
         VALUES (1, 'RJ01234567', 'One', 3, '2024-05-01', 1, 4.5, 10, 'dlsite', 'dlsite', 2, 1, 1),
                (2, 'BJ01234567', 'Two', NULL, NULL, 0, NULL, NULL, 'none', 'none', 0, 2, 2);",
    )
    .await;
    let body = list(&app, &token, "").await;
    assert_eq!(
        body["items"],
        json!([
            {
                "id": 2, "code": "BJ01234567", "title": "Two", "circle": null,
                "release_date": null, "nsfw": false, "rate_average": null, "rate_count": null,
                "has_cover": false, "cover_version": 0, "missing": false, "favorite": false,
                "progress": null, "my_rating": null
            },
            {
                "id": 1, "code": "RJ01234567", "title": "One",
                "circle": {"id": 3, "name": "Circle"},
                "release_date": "2024-05-01", "nsfw": true, "rate_average": 4.5, "rate_count": 10,
                "has_cover": true, "cover_version": 2, "missing": false, "favorite": false,
                "progress": null, "my_rating": null
            }
        ])
    );
}

/// One work per row of the listening status table in data-model.md, plus a
/// favorite, each seen by alice. Bob's state must never leak into hers.
#[tokio::test]
async fn favorite_and_progress_follow_the_status_table() {
    let app = app().await;
    let alice = app.user("alice", true).await;
    let bob = app.user("bob", false).await;
    let token = app.login("alice").await;
    seed_works(&app, 7).await;
    app.exec(&format!(
        "INSERT INTO roots (id, display_name) VALUES ('r', 'R');
         INSERT INTO work_locations (id, work_id, root_id, rel_path, display_path)
         VALUES (1, 2, 'r', x'61', 'a'), (2, 4, 'r', x'62', 'b'),
                (3, 5, 'r', x'63', 'c'), (4, 6, 'r', x'64', 'd');
         INSERT INTO tracks (id, work_id, location_id, rel_path, display_path, sort_key, kind,
                             duration_ms, missing_at)
         VALUES (20, 2, 1, x'31', '1.mp3', '1', 'audio', 60000, NULL),
                (40, 4, 2, x'31', '1.mp3', '1', 'audio', NULL, NULL),
                (50, 5, 3, x'31', '1.mp3', '1', 'audio', 70000, NULL),
                (60, 6, 4, x'31', '1.mp3', '1', 'audio', 80000, 9);
         -- 1: no record. 2: listening. 3: imported, not matched to a track.
         -- 4: finished. 5: finished and replaying. 6: track marked missing.
         INSERT INTO play_progress (user_id, work_id, track_id, position_ms, finished_at,
                                    replaying, updated_at, group_path)
         VALUES ({alice}, 2, 20, 1000, NULL, 0, 1, 'mp3/SE有'),
                ({alice}, 3, NULL, 101000, NULL, 0, 1, NULL),
                ({alice}, 4, 40, 3000, 50, 0, 1, ''),
                ({alice}, 5, 50, 4000, 50, 1, 1, ''),
                ({alice}, 6, 60, 5000, NULL, 0, 1, ''),
                ({bob}, 1, NULL, 9, 50, 1, 1, NULL);
         -- 7: favorite only. Bob's favorite on 1 must not show.
         INSERT INTO favorites (user_id, work_id, created_at)
         VALUES ({alice}, 7, 1), ({alice}, 5, 1), ({bob}, 1, 1);"
    ))
    .await;
    let body = list(&app, &token, "?order=asc").await;
    let items = body["items"].as_array().unwrap();
    let state = |id: usize| (&items[id - 1]["favorite"], &items[id - 1]["progress"]);
    let progress = |track: Value, pos: i64, dur: Value, fin: bool, rep: bool, res: bool| {
        json!({
            "track_id": track, "position_ms": pos, "track_duration_ms": dur,
            "finished": fin, "replaying": rep, "resumable": res,
            "group_path": if track.is_null() { Value::Null } else { json!("") }
        })
    };
    assert_eq!(state(1), (&json!(false), &Value::Null));
    assert_eq!(
        state(2),
        (&json!(false), &{
            let mut p = progress(json!(20), 1000, json!(60000), false, false, true);
            p["group_path"] = json!("mp3/SE有");
            p
        })
    );
    assert_eq!(
        state(3),
        (
            &json!(false),
            &progress(Value::Null, 101000, Value::Null, false, false, false)
        )
    );
    assert_eq!(
        state(4),
        (
            &json!(false),
            &progress(json!(40), 3000, Value::Null, true, false, true)
        )
    );
    assert_eq!(
        state(5),
        (
            &json!(true),
            &progress(json!(50), 4000, json!(70000), true, true, true)
        )
    );
    assert_eq!(
        state(6),
        (
            &json!(false),
            &progress(json!(60), 5000, json!(80000), false, false, false)
        )
    );
    assert_eq!(state(7), (&json!(true), &Value::Null));
}

#[tokio::test]
async fn pages_are_gzipped_for_clients_that_accept_it() {
    let app = app().await;
    app.user("alice", true).await;
    let token = app.login("alice").await;
    seed_works(&app, 45).await;

    let req = || Req::get("/api/works").token(&token);
    let (status, plain_headers, plain) = app.send_raw(req()).await;
    assert_eq!(status, StatusCode::OK);
    assert!(plain_headers.get("content-encoding").is_none());
    let (status, headers, packed) = app.send_raw(req().header("accept-encoding", "gzip")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-encoding"], "gzip");
    assert_eq!(headers["vary"], "accept-encoding");
    assert!(
        packed.len() * 2 < plain.len(),
        "{} of {}",
        packed.len(),
        plain.len()
    );
    let mut unpacked = Vec::new();
    std::io::Read::read_to_end(
        &mut flate2::read::GzDecoder::new(&packed[..]),
        &mut unpacked,
    )
    .unwrap();
    assert_eq!(unpacked, plain);

    // A short answer is not worth it, including the JSON 404 that the
    // frontend fallback gives an unknown API path.
    for uri in ["/api/version", "/api/no-such-route"] {
        let (_, headers, _) = app
            .send_raw(Req::get(uri).header("accept-encoding", "gzip"))
            .await;
        assert!(headers.get("content-encoding").is_none(), "{uri}");
    }
}
