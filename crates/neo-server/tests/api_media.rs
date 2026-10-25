mod common;

use axum::http::StatusCode;
use common::{App, Req, app, assert_error};

const AUDIO: &[u8] = b"0123456789abcdefghij";

/// One work at `RJ001` in the local root `lib`, with an audio file, its lrc
/// subtitle, a track whose file is gone from disk, and a track in the
/// OpenList root `cloud`, which has no storage here, as when its backend
/// fails to load.
async fn library() -> (App, String) {
    let app = app().await;
    app.user("admin", true).await;
    app.user("alice", false).await;
    let work = app.library().join("RJ001/mp3");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(work.join("01 トラック.mp3"), AUDIO).unwrap();
    std::fs::write(
        work.join("01 トラック.lrc"),
        "[00:01.00]hello\n[00:02.00]world\n",
    )
    .unwrap();
    app.exec(
        "INSERT INTO roots (id, display_name) VALUES ('lib', 'Lib'), ('cloud', 'Cloud');
         INSERT INTO works (id, code, title, metadata_src, created_at, updated_at)
           VALUES (1, 'RJ000001', 'Work one', 'scan', 0, 0);
         INSERT INTO work_locations (id, work_id, root_id, rel_path, display_path) VALUES
           (1, 1, 'lib', CAST('RJ001' AS BLOB), 'RJ001'),
           (2, 1, 'cloud', CAST('RJ001' AS BLOB), 'RJ001');
         INSERT INTO tracks (id, work_id, location_id, rel_path, display_path, sort_key, kind, missing_at) VALUES
           (1, 1, 1, CAST('mp3/01 トラック.mp3' AS BLOB), 'mp3/01 トラック.mp3', 'a', 'audio', NULL),
           (2, 1, 1, CAST('mp3/01 トラック.lrc' AS BLOB), 'mp3/01 トラック.lrc', 'b', 'subtitle', NULL),
           (3, 1, 1, CAST('mp3/gone.mp3' AS BLOB), 'mp3/gone.mp3', 'c', 'audio', NULL),
           (4, 1, 1, CAST('mp3/marked.mp3' AS BLOB), 'mp3/marked.mp3', 'd', 'audio', 1),
           (5, 1, 2, CAST('remote.mp3' AS BLOB), 'remote.mp3', 'e', 'audio', NULL);",
    )
    .await;
    let token = app.login("alice").await;
    (app, token)
}

#[tokio::test]
async fn stream_serves_the_whole_file_with_validators() {
    let (app, token) = library().await;
    let (status, headers, body) = app
        .send_raw(Req::get("/api/tracks/1/stream").token(&token))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, AUDIO);
    assert_eq!(headers["content-type"], "audio/mpeg");
    assert_eq!(headers["accept-ranges"], "bytes");
    assert_eq!(headers["content-length"], "20");
    assert!(headers.contains_key("etag"));
    assert!(headers.contains_key("last-modified"));
}

#[tokio::test]
async fn ranges_and_if_range_follow_the_book() {
    let (app, token) = library().await;
    let get = |range: &str| {
        Req::get("/api/tracks/1/stream")
            .token(&token)
            .header("range", range)
    };
    let (status, headers, body) = app.send_raw(get("bytes=5-9")).await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(body, b"56789");
    assert_eq!(headers["content-range"], "bytes 5-9/20");

    let (status, _, body) = app.send_raw(get("bytes=-3")).await;
    assert_eq!(
        (status, body.as_slice()),
        (StatusCode::PARTIAL_CONTENT, &b"hij"[..])
    );

    let (status, headers, body) = app.send_raw(get("bytes=50-")).await;
    assert_eq!(status, StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(headers["content-range"], "bytes */20");
    assert!(body.is_empty());

    let (status, _, body) = app.send_raw(get("bytes=0-1,5-6")).await;
    assert_eq!(
        (status, body.len()),
        (StatusCode::OK, 20),
        "multi-range gets the whole file"
    );

    let (_, headers, _) = app.send_raw(get("bytes=0-0")).await;
    let etag = headers["etag"].to_str().unwrap().to_owned();
    let (status, _, _) = app
        .send_raw(get("bytes=0-0").header("if-range", &etag))
        .await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    let (status, _, _) = app
        .send_raw(get("bytes=0-0").header("if-range", "\"stale\""))
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn head_returns_headers_only() {
    let (app, token) = library().await;
    let (status, headers, body) = app
        .send_raw(Req::head("/api/tracks/1/stream").token(&token))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-length"], "20");
    assert!(body.is_empty());
    for range in ["bytes=0-0", "bytes=50-"] {
        let (status, headers, _) = app
            .send_raw(
                Req::head("/api/tracks/1/stream")
                    .token(&token)
                    .header("range", range),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "HEAD ignores Range: {range}");
        assert_eq!(headers["content-length"], "20");
    }
}

#[tokio::test]
async fn a_media_token_in_the_query_is_accepted() {
    let (app, _) = library().await;
    let alice = 2;
    let version: i64 = sqlx::query_scalar("SELECT token_version FROM users WHERE id = ?")
        .bind(alice)
        .fetch_one(&app.db.sqlite().unwrap().read)
        .await
        .unwrap();
    let media = app.media_token(alice, version);
    let (status, _, body) = app
        .send_raw(Req::get(&format!("/api/tracks/1/stream?token={media}")))
        .await;
    assert_eq!((status, body.as_slice()), (StatusCode::OK, AUDIO));
    let (status, _, _) = app.send_raw(Req::get("/api/tracks/1/stream")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn download_adds_an_attachment_name() {
    let (app, token) = library().await;
    let (status, headers, body) = app
        .send_raw(Req::get("/api/tracks/1/download").token(&token))
        .await;
    assert_eq!((status, body.as_slice()), (StatusCode::OK, AUDIO));
    let disposition = headers["content-disposition"].to_str().unwrap();
    assert!(disposition.starts_with("attachment; "), "{disposition}");
    assert!(
        disposition.contains("filename*=UTF-8''01%20%E3%83%88"),
        "{disposition}"
    );
}

#[tokio::test]
async fn missing_and_unknown_tracks_get_contract_errors() {
    let (app, token) = library().await;
    let (status, body) = app
        .send(Req::get("/api/tracks/4/stream").token(&token))
        .await;
    assert_error(status, &body, StatusCode::GONE, "missing");
    let (status, body) = app
        .send(Req::get("/api/tracks/3/stream").token(&token))
        .await;
    assert_error(status, &body, StatusCode::GONE, "missing");
    let (status, body) = app
        .send(Req::get("/api/tracks/99/stream").token(&token))
        .await;
    assert_error(status, &body, StatusCode::NOT_FOUND, "not_found");
    let (status, body) = app
        .send(Req::get("/api/tracks/5/stream").token(&token))
        .await;
    assert_error(
        status,
        &body,
        StatusCode::SERVICE_UNAVAILABLE,
        "storage_unavailable",
    );
    let (status, body) = app
        .send(Req::get("/api/tracks/x/stream").token(&token))
        .await;
    assert_error(status, &body, StatusCode::BAD_REQUEST, "invalid_request");
}

#[tokio::test]
async fn an_unmounted_root_is_unavailable_not_missing() {
    let (app, token) = library().await;
    std::fs::remove_dir_all(app.library()).unwrap();
    let (status, body) = app
        .send(Req::get("/api/tracks/1/stream").token(&token))
        .await;
    assert_error(
        status,
        &body,
        StatusCode::SERVICE_UNAVAILABLE,
        "storage_unavailable",
    );
}

#[tokio::test]
async fn a_symlink_swapped_in_is_never_followed() {
    let (app, token) = library().await;
    let path = app.library().join("RJ001/mp3/01 トラック.mp3");
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink("/etc/hostname", &path).unwrap();
    let (status, body) = app
        .send(Req::get("/api/tracks/1/stream").token(&token))
        .await;
    assert_error(status, &body, StatusCode::GONE, "missing");
}

#[tokio::test]
async fn subtitle_is_served_as_webvtt() {
    let (app, token) = library().await;
    let (status, headers, body) = app
        .send_raw(Req::get("/api/tracks/1/subtitle").token(&token))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "text/vtt; charset=utf-8");
    let text = String::from_utf8(body).unwrap();
    assert!(text.starts_with("WEBVTT\n"), "{text}");
    assert!(
        text.contains("00:00:01.000 --> 00:00:02.000\nhello"),
        "{text}"
    );

    // A track without a subtitle file next to it, and a missing one.
    let (status, body) = app
        .send(Req::get("/api/tracks/3/subtitle").token(&token))
        .await;
    assert_error(status, &body, StatusCode::NOT_FOUND, "not_found");
    let (status, body) = app
        .send(Req::get("/api/tracks/4/subtitle").token(&token))
        .await;
    assert_error(status, &body, StatusCode::GONE, "missing");
}

#[tokio::test]
async fn a_library_file_loads_nothing_but_a_pdf_still_opens() {
    let (app, token) = library().await;
    std::fs::write(
        app.library().join("RJ001/mp3/notes.html"),
        "<script>1</script>",
    )
    .unwrap();
    std::fs::write(app.library().join("RJ001/booklet.pdf"), "%PDF-1.4").unwrap();
    app.exec(
        "INSERT INTO tracks (id, work_id, location_id, rel_path, display_path, sort_key, kind) VALUES
           (6, 1, 1, CAST('mp3/notes.html' AS BLOB), 'mp3/notes.html', 'f', 'other'),
           (7, 1, 1, CAST('booklet.pdf' AS BLOB), 'booklet.pdf', 'g', 'other');",
    )
    .await;
    let policy = |headers: &axum::http::HeaderMap| {
        headers[axum::http::header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .to_owned()
    };
    let (status, headers, _) = app
        .send_raw(Req::get("/api/tracks/6/stream").token(&token))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        policy(&headers),
        "default-src 'none'; frame-ancestors 'none'"
    );
    // A browser's PDF viewer is a plugin, which default-src 'none' blocks.
    let (status, headers, _) = app
        .send_raw(Req::get("/api/tracks/7/stream").token(&token))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "application/pdf");
    assert_eq!(policy(&headers), "frame-ancestors 'none'");
}
