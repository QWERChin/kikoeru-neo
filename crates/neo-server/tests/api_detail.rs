//! `GET /api/works/{id}`, `GET /api/works/{id}/cover` and
//! `GET /api/admin/roots` against synthetic rows and images.

mod common;

use axum::http::{StatusCode, header};
use common::*;
use image::{ImageFormat, RgbImage};

/// Work 1 in roots `lib` and `cloud`: a subtitle for Tr01, a copy of Tr01
/// in the second location, a missing wav and a cover image. Work 2 is
/// missing everywhere.
async fn seed(app: &App) {
    app.exec(
        r#"INSERT INTO roots (id, display_name, status, last_scan_at) VALUES
             ('lib', 'Library', 'ok', 1791699000000), ('cloud', '网盘', 'unknown', NULL);
           INSERT INTO circles (id, name) VALUES (3, 'Circle');
           INSERT INTO vas (id, name) VALUES ('va-2', 'Voice B'), ('va-1', 'Voice A');
           INSERT INTO tags (id, name) VALUES (9, 'Whisper'), (7, 'ASMR');
           INSERT INTO works (id, code, title, circle_id, release_date, nsfw, rate_average,
                              rate_count, metadata_src, cover_src, cover_version,
                              created_at, updated_at)
             VALUES (1, 'RJ01234567', 'Work', 3, '2024-05-01', 0, 4.62, 318, 'dlsite',
                     'imported', 2, 1, 1),
                    (2, 'RJ00000002', 'Gone', NULL, NULL, 0, NULL, NULL, 'none', 'none', 0, 2, 2);
           UPDATE works SET missing_at = 5 WHERE id = 2;
           INSERT INTO work_vas VALUES (1, 'va-2'), (1, 'va-1');
           INSERT INTO work_tags VALUES (1, 9), (1, 7);
           INSERT INTO work_locations (id, work_id, root_id, rel_path, display_path) VALUES
             (1, 1, 'lib', X'524A3031323334353637', 'RJ01234567'),
             (2, 1, 'cloud', X'78', 'x'),
             (3, 2, 'lib', X'52', 'R');
           UPDATE work_locations SET missing_at = 5 WHERE id = 3;
           INSERT INTO tracks (id, work_id, location_id, rel_path, display_path, sort_key, kind,
                               size, duration_ms, missing_at, duplicate_of) VALUES
             (101, 1, 1, CAST('mp3/Tr01.mp3' AS BLOB), 'mp3/Tr01.mp3', 'mp3/tr0101.mp3', 'audio', 100, 301000, NULL, NULL),
             (102, 1, 1, CAST('mp3/Tr02.mp3' AS BLOB), 'mp3/Tr02.mp3', 'mp3/tr0102.mp3', 'audio', 200, 1995000, NULL, NULL),
             (103, 1, 1, CAST('mp3/Tr01.lrc' AS BLOB), 'mp3/Tr01.lrc', 'mp3/tr0101.lrc', 'subtitle', 3, NULL, NULL, NULL),
             (104, 1, 1, CAST('wav/Tr01.wav' AS BLOB), 'wav/Tr01.wav', 'wav/tr0101.wav', 'audio', 400, NULL, 5, NULL),
             (105, 1, 1, CAST('cover.jpg' AS BLOB), 'cover.jpg', 'cover.jpg', 'image', 5, NULL, NULL, NULL),
             (106, 1, 2, CAST('mp3/Tr01.mp3' AS BLOB), 'mp3/Tr01.mp3', 'mp3/tr0101.mp3', 'audio', 100, 301000, NULL, 101);"#,
    )
    .await;
}

#[tokio::test]
async fn detail_lists_shown_tracks_in_natural_order() {
    let app = app().await;
    let user = app.user("alice", true).await;
    let token = app.login("alice").await;
    seed(&app).await;
    app.exec(&format!(
        "INSERT INTO favorites VALUES ({user}, 1, 0);
         INSERT INTO play_progress (user_id, work_id, track_id, position_ms, group_path, updated_at)
         VALUES ({user}, 1, 102, 812000, 'mp3', 0);"
    ))
    .await;

    let (status, body) = app.send(Req::get("/api/works/1").token(&token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_same_shape_with_nullable(
        &body,
        &sample_in("m2", "work-detail.json"),
        "detail",
        WORK_DETAIL_NULLABLE,
    );
    assert_eq!(body["code"], "RJ01234567");
    assert_eq!(body["circle"]["name"], "Circle");
    assert_eq!(body["favorite"], true);
    assert_eq!(body["progress"]["track_id"], 102);
    assert_eq!(body["progress"]["group_path"], "mp3");
    assert_eq!(
        body["vas"][0]["name"], "Voice A",
        "voice actors sorted by name"
    );
    assert_eq!(body["tags"][0]["name"], "ASMR");
    let tracks = body["tracks"].as_array().unwrap();
    let ids: Vec<i64> = tracks.iter().map(|t| t["id"].as_i64().unwrap()).collect();
    assert_eq!(
        ids,
        [105, 103, 101, 102, 104],
        "the copy in cloud is hidden"
    );
    assert_eq!(tracks[2]["has_subtitle"], true);
    assert_eq!(tracks[3]["has_subtitle"], false);
    assert_eq!(tracks[4]["missing"], true);
    // The missing wav has no duration but cannot be played, so it does not
    // hold back the total.
    assert_eq!(body["total_duration_ms"], 301000 + 1995000);

    // Make the copy in the second location a different file: it is shown,
    // but the subtitle next to the first one is not its subtitle.
    app.exec("UPDATE tracks SET size = 999, duplicate_of = NULL WHERE id = 106")
        .await;
    let (_, body) = app.send(Req::get("/api/works/1").token(&token)).await;
    let other = body["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == 106)
        .unwrap()
        .clone();
    assert_eq!(other["has_subtitle"], false);
    app.exec("UPDATE tracks SET size = 100, duplicate_of = 101 WHERE id = 106")
        .await;

    app.exec("UPDATE tracks SET duration_ms = NULL WHERE id = 102")
        .await;
    let (_, body) = app.send(Req::get("/api/works/1").token(&token)).await;
    assert!(body["total_duration_ms"].is_null());

    // A work missing everywhere still answers, flagged as missing.
    let (status, body) = app.send(Req::get("/api/works/2").token(&token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["missing"], true);
    assert!(body["total_duration_ms"].is_null());
}

#[tokio::test]
async fn locations_are_shown_to_admins_only() {
    let app = app().await;
    app.user("admin", true).await;
    app.user("bob", false).await;
    seed(&app).await;
    // A missing location sorts after the present ones even though its root
    // ID and path both sort first.
    app.exec(
        "INSERT INTO work_locations (id, work_id, root_id, rel_path, display_path, missing_at)
         VALUES (4, 1, 'cloud', X'61', 'a/old', 7)",
    )
    .await;
    let admin = app.login("admin").await;
    let bob = app.login("bob").await;

    let (status, body) = app.send(Req::get("/api/works/1").token(&admin)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_same_shape_with_nullable(
        &body,
        &sample_in("m2", "work-detail.json"),
        "detail",
        WORK_DETAIL_NULLABLE,
    );
    assert_eq!(
        body["locations"],
        serde_json::json!([
            {"root_id": "cloud", "root_name": "网盘", "path": "x", "missing": false},
            {"root_id": "lib", "root_name": "Library", "path": "RJ01234567", "missing": false},
            {"root_id": "cloud", "root_name": "网盘", "path": "a/old", "missing": true},
        ])
    );
    let (_, body) = app.send(Req::get("/api/works/2").token(&admin)).await;
    assert_eq!(
        body["locations"],
        serde_json::json!([{"root_id": "lib", "root_name": "Library", "path": "R", "missing": true}]),
        "a missing work still says where it was"
    );

    let (status, body) = app.send(Req::get("/api/works/1").token(&bob)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["locations"].is_null(), "{body}");
}

#[tokio::test]
async fn detail_errors() {
    let app = app().await;
    let user = app.user("alice", true).await;
    let token = app.login("alice").await;
    seed(&app).await;

    let (status, body) = app.send(Req::get("/api/works/1")).await;
    assert_error(status, &body, StatusCode::UNAUTHORIZED, "unauthorized");
    let media = app.media_token(user, token_version(&app, user).await);
    let (status, body) = app.send(Req::get("/api/works/1").token(&media)).await;
    assert_error(status, &body, StatusCode::UNAUTHORIZED, "unauthorized");
    for path in ["/api/works/99", "/api/works/abc", "/api/works/0"] {
        let (status, body) = app.send(Req::get(path).token(&token)).await;
        assert_error(status, &body, StatusCode::NOT_FOUND, "not_found");
    }
    // A segment that does not decode gets the shared JSON 400.
    for path in ["/api/works/%FF", "/api/works/%FF/cover"] {
        let (status, body) = app.send(Req::get(path).token(&token)).await;
        assert_error(status, &body, StatusCode::BAD_REQUEST, "invalid_request");
    }

    app.exec(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 5000)
         INSERT INTO tracks (work_id, location_id, rel_path, display_path, sort_key, kind)
         SELECT 1, 1, CAST('f' || i AS BLOB), 'f' || i, 'f' || i, 'other' FROM n",
    )
    .await;
    let (status, body) = app.send(Req::get("/api/works/1").token(&token)).await;
    assert_error(status, &body, StatusCode::PAYLOAD_TOO_LARGE, "too_large");
}

async fn token_version(app: &App, user: i64) -> i64 {
    sqlx::query_scalar("SELECT token_version FROM users WHERE id = ?")
        .bind(user)
        .fetch_one(&app.db.sqlite().unwrap().read)
        .await
        .unwrap()
}

fn write_cover(app: &App, id: i64, width: u32, height: u32) -> Vec<u8> {
    let path = app.data_dir().join(format!("covers/{id}.jpg"));
    RgbImage::from_pixel(width, height, image::Rgb([200, 30, 30]))
        .save_with_format(&path, ImageFormat::Jpeg)
        .unwrap();
    std::fs::read(path).unwrap()
}

#[tokio::test]
async fn covers_thumbnails_and_placeholders() {
    let app = app().await;
    let user = app.user("alice", true).await;
    let token = app.login("alice").await;
    seed(&app).await;
    let original = write_cover(&app, 1, 1000, 500);

    let (status, headers, bytes) = app
        .send_raw(Req::get("/api/works/1/cover?v=2").token(&token))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "image/jpeg");
    // Revalidated on every load, so a signed-out browser cannot show it.
    assert_eq!(headers[header::CACHE_CONTROL], "private, no-cache");
    assert_eq!(bytes, original);
    let etag = headers[header::ETAG].to_str().unwrap().to_owned();
    let (status, headers, bytes) = app
        .send_raw(
            Req::get("/api/works/1/cover?v=2")
                .token(&token)
                .header("if-none-match", &etag),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert_eq!(headers[header::ETAG], etag.as_str());
    assert!(bytes.is_empty());
    // The thumbnail is another representation with its own tag.
    let (status, _, _) = app
        .send_raw(
            Req::get("/api/works/1/cover?size=thumb&v=2")
                .token(&token)
                .header("if-none-match", &etag),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // Image elements cannot send headers, so a media token in the URL works.
    let media = app.media_token(user, token_version(&app, user).await);
    let (status, _, bytes) = app
        .send_raw(Req::get(&format!(
            "/api/works/1/cover?size=thumb&token={media}"
        )))
        .await;
    assert_eq!(status, StatusCode::OK);
    let thumb = image::load_from_memory(&bytes).unwrap();
    assert_eq!((thumb.width(), thumb.height()), (480, 240));
    let cached = app.data_dir().join("cache/thumbs/1-2.jpg");
    assert_eq!(std::fs::read(&cached).unwrap(), bytes);
    assert_eq!(
        std::fs::read_dir(app.data_dir().join("cache/tmp"))
            .unwrap()
            .count(),
        0
    );
    // A second request is served from the cache file.
    std::fs::write(&cached, b"cached").unwrap();
    let (_, _, bytes) = app
        .send_raw(Req::get("/api/works/1/cover?size=thumb").token(&token))
        .await;
    assert_eq!(bytes, b"cached");

    // An API token in the URL would end up in logs, so it is refused.
    let (status, _, _) = app
        .send_raw(Req::get(&format!("/api/works/1/cover?token={token}")))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // No cover, or a cover file that is gone: the placeholder, not cached.
    // A browser still holding the old picture gets the placeholder too, not
    // a 304 that would keep showing it.
    for id in [2, 1] {
        if id == 1 {
            std::fs::remove_file(app.data_dir().join("covers/1.jpg")).unwrap();
        }
        let (status, headers, bytes) = app
            .send_raw(
                Req::get(&format!("/api/works/{id}/cover?v=0"))
                    .token(&token)
                    .header("if-none-match", &etag),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers[header::CONTENT_TYPE], "image/svg+xml");
        assert_eq!(headers[header::CACHE_CONTROL], "private, no-cache");
        assert!(bytes.starts_with(b"<svg"));
    }

    let (status, body) = app
        .send(Req::get("/api/works/1/cover?size=huge").token(&token))
        .await;
    assert_error(status, &body, StatusCode::BAD_REQUEST, "invalid_request");
    let (status, body) = app
        .send(Req::get("/api/works/99/cover").token(&token))
        .await;
    assert_error(status, &body, StatusCode::NOT_FOUND, "not_found");
}

#[tokio::test]
async fn roots_are_listed_for_admins_only() {
    let app = app().await;
    app.user("admin", true).await;
    app.user("bob", false).await;
    seed(&app).await;
    let admin = app.login("admin").await;
    let bob = app.login("bob").await;

    let (status, body) = app.send(Req::get("/api/admin/roots").token(&bob)).await;
    assert_error(status, &body, StatusCode::FORBIDDEN, "forbidden");

    let (status, body) = app.send(Req::get("/api/admin/roots").token(&admin)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Item by item: the settings of a local and an OpenList root have
    // different fields, and the sample lists one of each in this order.
    let sample = sample_in("m2", "roots.json");
    for i in 0..2 {
        assert_same_shape(&body[i], &sample[i], &format!("roots[{i}]"));
    }
    assert_eq!(body[0]["source"], "config");
    assert_eq!(body[1]["settings"]["type"], "openlist");
    assert!(
        body[1]["settings"].get("password").is_none(),
        "never the password"
    );
    assert_eq!(body[0]["id"], "lib");
    assert_eq!(body[0]["kind"], "local");
    assert_eq!(body[0]["status"], "ok");
    assert_eq!(body[0]["last_scan_at"], 1791699000000_i64);
    assert_eq!(
        body[0]["work_count"], 1,
        "the missing location does not count"
    );
    assert_eq!(body[1]["display_name"], "网盘");
    assert_eq!(body[1]["kind"], "openlist");
    assert_eq!(body[1]["work_count"], 1);
}

#[tokio::test]
async fn a_cover_takes_the_media_token_from_its_cookie() {
    let app = app().await;
    let user = app.user("alice", true).await;
    let token = app.login("alice").await;
    seed(&app).await;
    let original = write_cover(&app, 1, 600, 400);
    let media = app.media_token(user, token_version(&app, user).await);
    let get = |cookie: String| {
        Req::get("/api/works/1/cover?v=1").header("cookie", &format!("theme=dark; {cookie}"))
    };

    // The URL stays the same when the token is renewed, so it stays cached.
    let (status, _, bytes) = app.send_raw(get(format!("kikoeru-media={media}"))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, original);

    // Over HTTPS the cookie has its own name. A stale token in the plain
    // HTTP one next to it does not get in the way.
    for cookie in [
        format!("__Secure-kikoeru-media={media}"),
        format!("kikoeru-media=old; __Secure-kikoeru-media={media}"),
        format!("__Secure-kikoeru-media=old; kikoeru-media={media}"),
    ] {
        let (status, _, _) = app.send_raw(get(cookie.clone())).await;
        assert_eq!(status, StatusCode::OK, "{cookie}");
    }

    // Only a media token, and only a valid one.
    for bad in [
        format!("kikoeru-media={token}"),
        "kikoeru-media=nope".into(),
        "other=x".into(),
    ] {
        let (status, _, _) = app.send_raw(get(bad.clone())).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{bad}");
    }

    // Logging out everywhere revokes the cookie's token too.
    let (status, _) = app
        .send(Req::post("/api/auth/logout-all").token(&token))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, _) = app.send_raw(get(format!("kikoeru-media={media}"))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // A revoked token keeps a valid signature. It must not hide a fresh
    // token in the other cookie.
    let fresh = app.media_token(user, token_version(&app, user).await);
    for cookie in [
        format!("__Secure-kikoeru-media={media}; kikoeru-media={fresh}"),
        format!("kikoeru-media={media}; __Secure-kikoeru-media={fresh}"),
    ] {
        let (status, _, _) = app.send_raw(get(cookie.clone())).await;
        assert_eq!(status, StatusCode::OK, "{cookie}");
    }
}

#[tokio::test]
async fn the_cover_cookie_opens_no_other_media_route() {
    let app = app().await;
    let user = app.user("alice", true).await;
    app.login("alice").await;
    seed(&app).await;
    let media = app.media_token(user, token_version(&app, user).await);
    for uri in ["/api/tracks/1/stream", "/api/works/1/download"] {
        let (status, _, _) = app
            .send_raw(Req::get(uri).header("cookie", &format!("kikoeru-media={media}")))
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}");
    }
}

#[tokio::test]
async fn a_cover_or_thumbnail_that_is_a_symlink_is_not_followed() {
    let app = app().await;
    app.user("alice", true).await;
    let token = app.login("alice").await;
    seed(&app).await;
    let secret = app.data_dir().join("secret.bin");
    std::fs::write(&secret, b"not a cover").unwrap();
    std::os::unix::fs::symlink(&secret, app.data_dir().join("covers/1.jpg")).unwrap();
    let (_, headers, bytes) = app
        .send_raw(Req::get("/api/works/1/cover").token(&token))
        .await;
    assert_eq!(headers[header::CONTENT_TYPE], "image/svg+xml");
    assert_ne!(bytes, b"not a cover");

    // A cached thumbnail swapped for a link is not served either.
    std::fs::remove_file(app.data_dir().join("covers/1.jpg")).unwrap();
    write_cover(&app, 1, 1000, 500);
    std::os::unix::fs::symlink(&secret, app.data_dir().join("cache/thumbs/1-2.jpg")).unwrap();
    let (status, headers, bytes) = app
        .send_raw(Req::get("/api/works/1/cover?size=thumb").token(&token))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(bytes, b"not a cover");
    assert_eq!(headers[header::CONTENT_TYPE], "image/jpeg");
}
