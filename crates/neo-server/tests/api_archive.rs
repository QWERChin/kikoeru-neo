mod common;

use std::io::Read;

use axum::http::StatusCode;
use common::{App, Req, app, assert_error};

/// A work with two locations in `lib`. `RJ001` holds two mp3 files, a
/// cover and a missing file. `RJ001-extra` holds a duplicate of one mp3
/// (hidden) and a different file with the same path as the cover.
async fn library() -> (App, String) {
    let app = app().await;
    app.user("admin", true).await;
    app.user("alice", false).await;
    let first = app.library().join("RJ001");
    let second = app.library().join("RJ001-extra");
    std::fs::create_dir_all(first.join("mp3")).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    std::fs::write(first.join("mp3/01.mp3"), b"first track").unwrap();
    std::fs::write(first.join("mp3/02.mp3"), b"second track").unwrap();
    std::fs::write(first.join("cover.jpg"), b"cover one").unwrap();
    std::fs::write(second.join("cover.jpg"), b"cover two").unwrap();
    app.exec(
        "INSERT INTO roots (id, display_name) VALUES ('lib', 'Lib');
         INSERT INTO works (id, code, title, metadata_src, created_at, updated_at)
           VALUES (1, 'RJ000001', 'Work one', 'scan', 0, 0);
         INSERT INTO work_locations (id, work_id, root_id, rel_path, display_path) VALUES
           (1, 1, 'lib', CAST('RJ001' AS BLOB), 'RJ001'),
           (2, 1, 'lib', CAST('RJ001-extra' AS BLOB), 'RJ001-extra');
         INSERT INTO tracks (id, work_id, location_id, rel_path, display_path, sort_key, kind, missing_at, duplicate_of) VALUES
           (1, 1, 1, CAST('mp3/01.mp3' AS BLOB), 'mp3/01.mp3', 'b1', 'audio', NULL, NULL),
           (2, 1, 1, CAST('mp3/02.mp3' AS BLOB), 'mp3/02.mp3', 'b2', 'audio', NULL, NULL),
           (3, 1, 1, CAST('cover.jpg' AS BLOB), 'cover.jpg', 'a', 'image', NULL, NULL),
           (4, 1, 1, CAST('mp3/gone.mp3' AS BLOB), 'mp3/gone.mp3', 'b3', 'audio', 1, NULL),
           (5, 1, 2, CAST('mp3/01.mp3' AS BLOB), 'mp3/01.mp3', 'b1', 'audio', NULL, 1),
           (6, 1, 2, CAST('cover.jpg' AS BLOB), 'cover.jpg', 'a', 'image', NULL, NULL);",
    )
    .await;
    let token = app.login("alice").await;
    (app, token)
}

fn entries(bytes: Vec<u8>) -> Vec<(String, Vec<u8>)> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    (0..archive.len())
        .map(|i| {
            let mut file = archive.by_index(i).unwrap();
            let mut data = Vec::new();
            file.read_to_end(&mut data).unwrap();
            (file.name().unwrap().into_owned(), data)
        })
        .collect()
}

#[tokio::test]
async fn the_whole_work_downloads_without_missing_files_or_duplicates() {
    let (app, token) = library().await;
    let (status, headers, bytes) = app
        .send_raw(Req::get("/api/works/1/download").token(&token))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "application/zip");
    assert!(
        headers["content-disposition"]
            .to_str()
            .unwrap()
            .contains("RJ000001.zip")
    );
    let got = entries(bytes);
    let names: Vec<&str> = got.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        ["cover.jpg", "cover (2).jpg", "mp3/01.mp3", "mp3/02.mp3"]
    );
    assert_eq!(got[2].1, b"first track");
    assert_eq!(got[1].1, b"cover two");
}

#[tokio::test]
async fn a_group_downloads_only_its_directory() {
    let (app, token) = library().await;
    let (status, headers, bytes) = app
        .send_raw(Req::get("/api/works/1/download?group=mp3").token(&token))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers["content-disposition"]
            .to_str()
            .unwrap()
            .contains("RJ000001-mp3.zip")
    );
    let names: Vec<String> = entries(bytes).into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["mp3/01.mp3", "mp3/02.mp3"]);
}

#[tokio::test]
async fn unknown_works_and_empty_groups_are_not_found() {
    let (app, token) = library().await;
    let (status, body) = app
        .send(Req::get("/api/works/9/download").token(&token))
        .await;
    assert_error(status, &body, StatusCode::NOT_FOUND, "not_found");
    let (status, body) = app
        .send(Req::get("/api/works/1/download?group=wav").token(&token))
        .await;
    assert_error(status, &body, StatusCode::NOT_FOUND, "not_found");
    let (status, _) = app
        .send(Req::get("/api/works/1/download?group=mp").token(&token))
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a group is a whole directory name"
    );
    let (status, _) = app.send(Req::get("/api/works/1/download")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_client_that_stops_reading_frees_its_slot() {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let (app, token) = library().await;
    // The test app stops waiting for a stalled client after 300 ms.
    let router = app.router.clone();
    // Big enough to fill the pipe, so the writer blocks on the client.
    std::fs::write(app.library().join("RJ001/mp3/01.mp3"), vec![7u8; 4 << 20]).unwrap();
    let request = || {
        Request::get("/api/works/1/download")
            .header("authorization", format!("Bearer {token}"))
            .extension(axum::extract::ConnectInfo(
                "192.0.2.10:40000".parse::<std::net::SocketAddr>().unwrap(),
            ))
            .body(Body::empty())
            .unwrap()
    };
    let started = std::time::Instant::now();
    let mut stalled = Vec::new();
    for _ in 0..2 {
        let response = router.clone().oneshot(request()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        stalled.push(response);
    }
    let third = router.clone().oneshot(request()).await.unwrap();
    assert_full(third.status(), started);
    admitted_again(&router, request).await;
    drop(stalled);
}

/// Checks that a third download is turned away while two hold the slots.
/// A busy machine can take longer than the 300 ms stall timeout to send
/// the three requests, and then the first download has already given up
/// its slot, so 200 is also right once that much time has passed.
fn assert_full(status: StatusCode, started: std::time::Instant) {
    if status == StatusCode::OK && started.elapsed() >= std::time::Duration::from_millis(300) {
        return;
    }
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

/// Retries until a download is admitted again. The stall timeout is 300 ms
/// in the test app; a busy machine can take longer to notice, so the test
/// waits for the outcome instead of sleeping a fixed time, and only fails
/// if no slot frees up within 15 s.
async fn admitted_again(
    router: &axum::Router,
    request: impl Fn() -> axum::http::Request<axum::body::Body>,
) {
    use tower::ServiceExt;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let response = router.clone().oneshot(request()).await.unwrap();
        if response.status() == StatusCode::OK {
            return;
        }
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(
            std::time::Instant::now() < deadline,
            "the stalled downloads never gave up their slots"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn a_stall_during_headers_also_frees_the_slot() {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let (app, token) = library().await;
    add_many_empty_files(&app).await;
    let request = || {
        Request::get("/api/works/1/download?group=many")
            .header("authorization", format!("Bearer {token}"))
            .extension(axum::extract::ConnectInfo(
                "192.0.2.10:40000".parse::<std::net::SocketAddr>().unwrap(),
            ))
            .body(Body::empty())
            .unwrap()
    };
    let started = std::time::Instant::now();
    let mut stalled = Vec::new();
    for _ in 0..2 {
        stalled.push(app.router.clone().oneshot(request()).await.unwrap());
    }
    let full = app.router.clone().oneshot(request()).await.unwrap();
    assert_full(full.status(), started);
    // Header writes give up too.
    admitted_again(&app.router, request).await;
    drop(stalled);
}

/// Many empty files: their archive is almost all headers and central
/// directory, which fill the pipe without any file data.
async fn add_many_empty_files(app: &App) {
    let dir = app.library().join("RJ001/many");
    std::fs::create_dir_all(&dir).unwrap();
    let mut rows = Vec::new();
    for i in 0..4000 {
        std::fs::write(dir.join(format!("{i:04}.txt")), b"").unwrap();
        rows.push(format!(
            "({id}, 1, 1, CAST('many/{i:04}.txt' AS BLOB), 'many/{i:04}.txt', 'm{i:04}', 'text')",
            id = 100 + i
        ));
    }
    app.exec(&format!(
        "INSERT INTO tracks (id, work_id, location_id, rel_path, display_path, sort_key, kind) VALUES {}",
        rows.join(",")
    ))
    .await;
}

#[tokio::test]
async fn a_slow_client_that_keeps_reading_is_never_cut_off() {
    use http_body_util::BodyExt;
    let (app, token) = library().await;
    add_many_empty_files(&app).await;
    let response = open_download(&app, &token, "/api/works/1/download?group=many").await;
    let mut body = response.into_body();
    let mut bytes = Vec::new();
    let started = std::time::Instant::now();
    while let Some(frame) = body.frame().await {
        let frame = frame.expect("no stall error while the client keeps reading");
        if let Ok(data) = frame.into_data() {
            bytes.extend_from_slice(&data);
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert!(
        started.elapsed() > std::time::Duration::from_millis(300),
        "slower than the stall limit"
    );
    assert_eq!(entries(bytes).len(), 4000);
}

async fn open_download(app: &App, token: &str, uri: &str) -> axum::response::Response {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let request = Request::get(uri)
        .header("authorization", format!("Bearer {token}"))
        .extension(axum::extract::ConnectInfo(
            "192.0.2.10:40000".parse::<std::net::SocketAddr>().unwrap(),
        ))
        .body(Body::empty())
        .unwrap();
    app.router.clone().oneshot(request).await.unwrap()
}

#[tokio::test]
async fn a_file_gone_since_the_scan_fails_the_download_instead_of_ending_it() {
    use http_body_util::BodyExt;
    let (app, token) = library().await;
    std::fs::remove_file(app.library().join("RJ001/cover.jpg")).unwrap();
    let response = open_download(&app, &token, "/api/works/1/download").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.into_body().collect().await.is_err(),
        "the body must end in an error"
    );
}

#[tokio::test]
async fn a_file_truncated_while_archived_fails_the_download() {
    use http_body_util::BodyExt;
    let (app, token) = library().await;
    let path = app.library().join("RJ001/mp3/01.mp3");
    std::fs::write(&path, vec![1u8; 4 << 20]).unwrap();
    let response = open_download(&app, &token, "/api/works/1/download?group=mp3").await;
    let mut body = response.into_body();
    // The first frames arrive while the writer is still inside the big file.
    body.frame().await.unwrap().unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(10)
        .unwrap();
    assert!(
        body.collect().await.is_err(),
        "a shrunken file must not end a valid archive"
    );
}
