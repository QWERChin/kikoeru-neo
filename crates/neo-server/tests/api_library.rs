//! Library routes: search, filters, sorts, favorites, history, the user's
//! reviews and the circle, voice actor and tag lists, against synthetic rows.

mod common;

use std::collections::BTreeSet;

use axum::http::StatusCode;
use common::*;
use serde_json::Value;

/// Works 1 to 5. Work 5 is missing. Release dates and ratings leave gaps so
/// the "sorts last" rule is visible.
async fn seed(app: &App) {
    app.exec(
        r#"INSERT INTO roots (id, display_name) VALUES ('lib', 'Library'), ('cloud', 'Cloud');
           INSERT INTO circles (id, name) VALUES (1, 'Alpha Circle'), (2, 'Beta 50%_off');
           INSERT INTO vas (id, name) VALUES ('va-1', 'Voice One'), ('va-2', 'Voice Two');
           INSERT INTO tags (id, name) VALUES (7, 'ASMR'), (9, 'Whisper');
           INSERT INTO works (id, code, title, circle_id, release_date, rate_average,
                              metadata_src, created_at, updated_at) VALUES
             (1, 'RJ00000001', 'First story', 1, '2020-01-01', 4.5, 'dlsite', 10, 10),
             (2, 'RJ00000002', 'Second 100% sale', 2, NULL, 3.0, 'dlsite', 20, 20),
             (3, 'RJ00000003', 'Third', 1, '2022-06-01', NULL, 'dlsite', 30, 30),
             (4, 'BJ00000004', 'Fourth', NULL, '2021-03-03', 4.9, 'dlsite', 40, 40),
             (5, 'RJ00000005', 'Gone story', 1, '2023-01-01', 5.0, 'dlsite', 50, 50);
           UPDATE works SET missing_at = 60 WHERE id = 5;
           INSERT INTO work_vas VALUES (1, 'va-1'), (2, 'va-2'), (5, 'va-2');
           INSERT INTO work_tags VALUES (1, 7), (3, 7), (5, 9), (2, 9);
           INSERT INTO work_locations (work_id, root_id, rel_path, display_path, missing_at) VALUES
             (1, 'lib', X'01', 'a', NULL), (2, 'cloud', X'02', 'b', NULL),
             (3, 'lib', X'03', 'c', 5), (3, 'cloud', X'04', 'd', NULL),
             (4, 'lib', X'05', 'e', NULL), (5, 'lib', X'06', 'f', 60);"#,
    )
    .await;
}

async fn setup() -> (App, i64, String) {
    let app = app().await;
    let user = app.user("alice", true).await;
    let token = app.login("alice").await;
    seed(&app).await;
    (app, user, token)
}

fn ids(body: &Value) -> Vec<i64> {
    body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["id"].as_i64().unwrap())
        .collect()
}

async fn works(app: &App, token: &str, query: &str) -> Value {
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
async fn search_and_filters() {
    let (app, _, token) = setup().await;
    let cases: &[(&str, &[i64])] = &[
        ("?q=story", &[1]),
        ("?q=STORY", &[1]),
        ("?q=%20%20", &[4, 3, 2, 1]),
        ("?q=rj00000003", &[3]),
        ("?q=alpha", &[3, 1]),
        ("?q=voice%20two", &[2]),
        // % and _ are literal characters, not wildcards.
        ("?q=100%25", &[2]),
        ("?q=50%25_", &[2]),
        ("?q=%25", &[2]),
        ("?q=_", &[2]),
        ("?tag=7", &[3, 1]),
        ("?circle=1", &[3, 1]),
        ("?va=va-2", &[2]),
        ("?root=lib", &[4, 1]),
        ("?root=cloud&tag=7", &[3]),
        ("?include_missing=true&circle=1", &[5, 3, 1]),
        ("?tag=999", &[]),
    ];
    for (query, want) in cases {
        let body = works(&app, &token, query).await;
        assert_eq!(ids(&body), *want, "{query}");
        assert_eq!(body["total"], want.len(), "{query}");
    }
}

#[tokio::test]
async fn sorts_keep_missing_values_last() {
    let (app, _, token) = setup().await;
    let cases: &[(&str, &[i64])] = &[
        ("?sort=release", &[3, 4, 1, 2]),
        ("?sort=release&order=asc", &[1, 4, 3, 2]),
        ("?sort=rating", &[4, 1, 2, 3]),
        ("?sort=rating&order=asc", &[2, 1, 4, 3]),
        ("?sort=added&order=asc", &[1, 2, 3, 4]),
    ];
    for (query, want) in cases {
        assert_eq!(ids(&works(&app, &token, query).await), *want, "{query}");
    }
}

#[tokio::test]
async fn random_order_is_stable_per_seed_and_pages_cleanly() {
    let (app, _, token) = setup().await;
    let all = ids(&works(&app, &token, "?sort=random&seed=42").await);
    assert_eq!(all.len(), 4);
    assert_eq!(
        all,
        ids(&works(&app, &token, "?sort=random&seed=42&order=asc").await),
        "order does not apply to random"
    );
    let mut paged = Vec::new();
    for page in 1..=4 {
        let body = works(
            &app,
            &token,
            &format!("?sort=random&seed=42&page_size=1&page={page}"),
        )
        .await;
        paged.extend(ids(&body));
    }
    assert_eq!(paged, all);
    // Different seeds give different orders.
    let mut seen = BTreeSet::new();
    for seed in [1_u64, 2, 3, 7, 1000, 99999, 4_294_967_296] {
        seen.insert(ids(&works(
            &app,
            &token,
            &format!("?sort=random&seed={seed}"),
        )
        .await));
    }
    assert!(seen.len() > 1, "{seen:?}");
}

#[tokio::test]
async fn bad_parameters() {
    let (app, _, token) = setup().await;
    for query in [
        "?sort=random",
        "?sort=random&seed=-1",
        "?sort=popular",
        "?tag=abc",
        "?circle=0",
        "?include_missing=yes",
        "?order=sideways",
    ] {
        let (status, body) = app
            .send(Req::get(&format!("/api/works{query}")).token(&token))
            .await;
        assert_error(status, &body, StatusCode::BAD_REQUEST, "invalid_request");
    }
}

#[tokio::test]
async fn favorites_and_history() {
    let (app, user, token) = setup().await;
    for id in [3, 1, 5] {
        let (status, _) = app
            .send(Req::put(&format!("/api/me/favorites/{id}")).token(&token))
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }
    // Fix the times so the order does not depend on the clock.
    app.exec(&format!(
        "UPDATE favorites SET created_at = work_id * 10 WHERE user_id = {user};
         UPDATE favorites SET created_at = 10 WHERE user_id = {user} AND work_id = 3;"
    ))
    .await;
    // A repeated PUT keeps the original time.
    app.send(Req::put("/api/me/favorites/1").token(&token))
        .await;

    let (status, body) = app.send(Req::get("/api/me/favorites").token(&token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_same_shape_with_nullable(
        &body,
        &sample_in("m3", "favorites-page.json"),
        "favorites",
        WORK_PAGE_NULLABLE,
    );
    assert_eq!(ids(&body), [5, 3, 1], "newest first, then by ID");
    assert_eq!(body["items"][0]["missing"], true, "missing works stay");
    assert_eq!(body["total"], 3);

    let (status, _) = app
        .send(Req::delete("/api/me/favorites/3").token(&token))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = app
        .send(Req::delete("/api/me/favorites/3").token(&token))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, body) = app.send(Req::get("/api/me/favorites").token(&token)).await;
    assert_eq!(ids(&body), [5, 1]);
    for req in [
        Req::put("/api/me/favorites/99"),
        Req::delete("/api/me/favorites/99"),
    ] {
        let (status, body) = app.send(req.token(&token)).await;
        assert_error(status, &body, StatusCode::NOT_FOUND, "not_found");
    }

    app.exec(&format!(
        "INSERT INTO play_progress (user_id, work_id, position_ms, updated_at)
         VALUES ({user}, 2, 5, 300), ({user}, 4, 5, 100), ({user}, 5, 5, 200)"
    ))
    .await;
    let (status, body) = app
        .send(Req::get("/api/me/history?page_size=2").token(&token))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_same_shape_with_nullable(
        &body,
        &sample_in("m3", "history-page.json"),
        "history",
        WORK_PAGE_NULLABLE,
    );
    assert_eq!(ids(&body), [2, 5]);
    assert_eq!(body["total"], 3);

    // Another user's state never leaks in.
    app.user("bob", false).await;
    let bob = app.login("bob").await;
    let (_, body) = app.send(Req::get("/api/me/favorites").token(&bob)).await;
    assert_eq!(body["total"], 0);
    let (_, body) = app.send(Req::get("/api/me/history").token(&bob)).await;
    assert_eq!(body["total"], 0);
}

#[tokio::test]
async fn reviews_are_kept_per_user_and_shown_in_lists() {
    let (app, user, token) = setup().await;
    let put = |id: i64, body: Value| {
        Req::put(&format!("/api/me/reviews/{id}"))
            .token(&token)
            .json(body)
    };
    let (status, body) = app
        .send(put(1, serde_json::json!({"rating": 4, "text": "  好听  "})))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_same_shape(&body, &sample_in("reviews", "review.json"), "review");
    assert_eq!(body["rating"], 4);
    assert_eq!(body["text"], "好听", "the text is trimmed");
    assert!(body["updated_at"].as_i64().unwrap() > 0);

    // A text-only review, one on a missing work, and a replacement that
    // drops the text.
    let (status, body) = app
        .send(put(
            3,
            serde_json::json!({"rating": null, "text": "only words"}),
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["rating"], Value::Null);
    let (status, _) = app
        .send(put(5, serde_json::json!({"rating": 2, "text": null})))
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, body) = app
        .send(put(1, serde_json::json!({"rating": 5, "text": ""})))
        .await;
    assert_eq!(body["rating"], 5);
    assert_eq!(body["text"], Value::Null, "an empty text clears it");

    app.exec(&format!(
        "UPDATE reviews SET updated_at = work_id * 10 WHERE user_id = {user};
         UPDATE reviews SET updated_at = NULL WHERE user_id = {user} AND work_id = 5;"
    ))
    .await;
    let (status, body) = app.send(Req::get("/api/me/reviews").token(&token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_same_shape_with_nullable(
        &body,
        &sample_in("reviews", "reviews-page.json"),
        "reviews",
        WORK_PAGE_NULLABLE,
    );
    assert_eq!(ids(&body), [3, 1, 5], "newest first, no time last");
    let ratings: Vec<Value> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["my_rating"].clone())
        .collect();
    assert_eq!(ratings, [Value::Null, 5.into(), 2.into()]);

    let list = works(&app, &token, "?sort=added&order=asc").await;
    assert_eq!(list["items"][0]["my_rating"], 5);
    assert_eq!(list["items"][1]["my_rating"], Value::Null);
    let (status, detail) = app.send(Req::get("/api/works/3").token(&token)).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["my_rating"], Value::Null);
    assert_eq!(detail["my_review"]["text"], "only words");
    assert_eq!(detail["my_review"]["updated_at"], 30);

    for body in [
        serde_json::json!({"rating": 0, "text": null}),
        serde_json::json!({"rating": 6, "text": null}),
        serde_json::json!({"rating": 4.5, "text": null}),
        serde_json::json!({"rating": null, "text": " \n "}),
        serde_json::json!({"rating": null, "text": "长".repeat(2001)}),
    ] {
        let (status, err) = app.send(put(1, body.clone())).await;
        assert_error(status, &err, StatusCode::BAD_REQUEST, "invalid_request");
    }
    let (status, _) = app
        .send(put(
            1,
            serde_json::json!({"rating": null, "text": "长".repeat(2000)}),
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "2000 characters fit");

    for _ in 0..2 {
        let (status, _) = app
            .send(Req::delete("/api/me/reviews/3").token(&token))
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }
    let (_, detail) = app.send(Req::get("/api/works/3").token(&token)).await;
    assert_eq!(detail["my_review"], Value::Null);
    for req in [
        put(99, serde_json::json!({"rating": 3, "text": null})),
        Req::delete("/api/me/reviews/99").token(&token),
    ] {
        let (status, body) = app.send(req).await;
        assert_error(status, &body, StatusCode::NOT_FOUND, "not_found");
    }

    app.user("bob", false).await;
    let bob = app.login("bob").await;
    let (_, body) = app.send(Req::get("/api/me/reviews").token(&bob)).await;
    assert_eq!(body["total"], 0);
    let (_, detail) = app.send(Req::get("/api/works/1").token(&bob)).await;
    assert_eq!(detail["my_rating"], Value::Null);
    assert_eq!(detail["my_review"], Value::Null);
}

#[tokio::test]
async fn filter_lists_count_present_works() {
    let (app, _, token) = setup().await;
    let (status, circles) = app.send(Req::get("/api/circles").token(&token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_same_shape(&circles, &sample_in("m3", "circles.json"), "circles");
    assert_eq!(
        circles,
        serde_json::json!([
            {"id": 1, "name": "Alpha Circle", "work_count": 2},
            {"id": 2, "name": "Beta 50%_off", "work_count": 1}
        ])
    );
    let (_, vas) = app.send(Req::get("/api/vas").token(&token)).await;
    assert_same_shape(&vas, &sample_in("m3", "vas.json"), "vas");
    assert_eq!(
        vas,
        serde_json::json!([
            {"id": "va-1", "name": "Voice One", "work_count": 1},
            {"id": "va-2", "name": "Voice Two", "work_count": 1}
        ])
    );
    let (_, tags) = app.send(Req::get("/api/tags").token(&token)).await;
    assert_same_shape(&tags, &sample_in("m3", "tags.json"), "tags");
    assert_eq!(
        tags,
        serde_json::json!([
            {"id": 7, "name": "ASMR", "work_count": 2},
            {"id": 9, "name": "Whisper", "work_count": 1}
        ])
    );
    let (status, body) = app.send(Req::get("/api/tags")).await;
    assert_error(status, &body, StatusCode::UNAUTHORIZED, "unauthorized");
}

#[tokio::test]
async fn an_overlong_search_is_refused() {
    let (app, _, token) = setup().await;
    let q = "a".repeat(201);
    let (status, err) = app
        .send(Req::get(&format!("/api/works?q={q}")).token(&token))
        .await;
    assert_error(status, &err, StatusCode::BAD_REQUEST, "invalid_request");
    let (status, _) = app
        .send(Req::get(&format!("/api/works?q={}", "a".repeat(200))).token(&token))
        .await;
    assert_eq!(status, StatusCode::OK);
}
