//! The tag merge route.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::json;

#[tokio::test]
async fn merge_tags() {
    let app = app().await;
    app.user("admin", true).await;
    app.user("bob", false).await;
    let admin = app.login("admin").await;
    let bob = app.login("bob").await;
    app.exec(
        "INSERT INTO tags (id, name) VALUES (7, 'ASMR'), (9, 'Whisper');
         INSERT INTO tag_aliases VALUES ('ASMR', 7), ('Whisper', 9);",
    )
    .await;

    let (status, body) = app
        .send(
            Req::post("/api/admin/tags/merge")
                .token(&bob)
                .json(json!({"from": 9, "into": 7})),
        )
        .await;
    assert_error(status, &body, StatusCode::FORBIDDEN, "forbidden");
    for request in [
        json!({"from": 9, "into": 9}),
        json!({"from": 9, "into": 12345}),
    ] {
        let (status, body) = app
            .send(
                Req::post("/api/admin/tags/merge")
                    .token(&admin)
                    .json(request),
            )
            .await;
        assert_error(status, &body, StatusCode::BAD_REQUEST, "invalid_request");
    }
    let (status, _) = app
        .send(
            Req::post("/api/admin/tags/merge")
                .token(&admin)
                .json(sample_in("m4", "tag-merge-request.json")),
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let aliases: Vec<(String, i64)> =
        sqlx::query_as("SELECT alias, tag_id FROM tag_aliases ORDER BY alias")
            .fetch_all(&app.db.sqlite().unwrap().read)
            .await
            .unwrap();
    assert_eq!(aliases, [("ASMR".to_owned(), 7), ("Whisper".to_owned(), 7)]);
}
