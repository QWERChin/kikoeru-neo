//! M6 video: the new `TrackItem` fields, raw ass subtitles and the font
//! list for JASSUB. Every file is synthetic.

mod common;

use axum::http::StatusCode;
use common::{App, Req, app, app_with, assert_error};

/// "[Script Info]" and a line of Japanese, in Shift_JIS as many library ass
/// files are.
fn shift_jis_ass() -> Vec<u8> {
    let mut bytes = b"[Script Info]\r\nTitle: sample\r\n\r\n[Events]\r\n".to_vec();
    // Enough text for the encoding guess, as in a real subtitle file.
    for second in 1..=12 {
        bytes.extend_from_slice(
            format!("Dialogue: 0,0:00:{second:02}.00,0:00:{second:02}.90,Default,,0,0,0,,")
                .as_bytes(),
        );
        // 日本語の字幕です
        bytes.extend_from_slice(&[
            0x93, 0xFA, 0x96, 0x7B, 0x8C, 0xEA, 0x82, 0xCC, 0x8E, 0x9A, 0x96, 0x8B, 0x82, 0xC5,
            0x82, 0xB7,
        ]);
        bytes.extend_from_slice(b"\r\n");
    }
    bytes
}

const PROBE: &str = r#"{"format":"mov,mp4,m4a,3gp,3g2,mj2","duration_ms":3000,
    "audio":{"codec":"aac","profile":"LC","sample_rate":48000,"channels":2},
    "video":{"codec":"h264","width":1280,"height":720}}"#;

/// One work with a video, its vtt and ass subtitles, and fonts: two shown,
/// one hidden as a duplicate, one missing, and a text file that is no font.
async fn library() -> (App, String, String) {
    let app = app().await;
    app.user("admin", true).await;
    let alice = app.user("alice", false).await;
    let work = app.library().join("RJ001");
    std::fs::create_dir_all(work.join("v")).unwrap();
    std::fs::create_dir_all(work.join("fonts")).unwrap();
    std::fs::write(work.join("v/movie.mp4"), b"not really a video").unwrap();
    std::fs::write(work.join("v/movie.ass"), shift_jis_ass()).unwrap();
    std::fs::write(
        work.join("v/movie.vtt"),
        "WEBVTT\n\n00:01.000 --> 00:02.000\nvtt line\n",
    )
    .unwrap();
    app.exec(
        "INSERT INTO roots (id, display_name) VALUES ('lib', 'Lib');
         INSERT INTO works (id, code, title, metadata_src, created_at, updated_at)
           VALUES (1, 'RJ000001', 'Work one', 'scan', 0, 0);
         INSERT INTO work_locations (id, work_id, root_id, rel_path, display_path)
           VALUES (1, 1, 'lib', CAST('RJ001' AS BLOB), 'RJ001');
         INSERT INTO tracks (id, work_id, location_id, rel_path, display_path, sort_key, kind,
                             probe, duplicate_of, missing_at) VALUES
           (1, 1, 1, CAST('v/movie.mp4' AS BLOB), 'v/movie.mp4', 'a', 'video', NULL, NULL, NULL),
           (2, 1, 1, CAST('v/movie.ass' AS BLOB), 'v/movie.ass', 'b', 'subtitle', NULL, NULL, NULL),
           (3, 1, 1, CAST('v/movie.vtt' AS BLOB), 'v/movie.vtt', 'c', 'subtitle', NULL, NULL, NULL),
           (4, 1, 1, CAST('fonts/A.ttf' AS BLOB), 'fonts/A.ttf', 'd', 'other', NULL, NULL, NULL),
           (5, 1, 1, CAST('fonts/b.OTF' AS BLOB), 'fonts/b.OTF', 'e', 'other', NULL, NULL, NULL),
           (6, 1, 1, CAST('fonts/copy.ttf' AS BLOB), 'fonts/copy.ttf', 'f', 'other', NULL, 4, NULL),
           (7, 1, 1, CAST('fonts/gone.ttf' AS BLOB), 'fonts/gone.ttf', 'g', 'other', NULL, NULL, 1),
           (8, 1, 1, CAST('fonts/readme.txt' AS BLOB), 'fonts/readme.txt', 'h', 'text', NULL, NULL, NULL);",
    )
    .await;
    sqlx::query("UPDATE tracks SET probe = ? WHERE id = 1")
        .bind(PROBE)
        .execute(&app.db.sqlite().unwrap().write)
        .await
        .unwrap();
    let version: i64 = sqlx::query_scalar("SELECT token_version FROM users WHERE id = ?")
        .bind(alice)
        .fetch_one(&app.db.sqlite().unwrap().read)
        .await
        .unwrap();
    let media = app.media_token(alice, version);
    let token = app.login("alice").await;
    (app, token, media)
}

#[tokio::test]
async fn work_detail_lists_subtitle_formats_and_the_picture() {
    let (app, token, _) = library().await;
    let (status, body) = app.send(Req::get("/api/works/1").token(&token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let video = &body["tracks"][0];
    assert_eq!(video["subtitle_formats"], serde_json::json!(["vtt", "ass"]));
    assert_eq!(video["has_subtitle"], true);
    assert_eq!(
        video["video"],
        serde_json::json!({"codec": "h264", "width": 1280, "height": 720})
    );
    let font = &body["tracks"][3];
    assert_eq!(font["subtitle_formats"], serde_json::json!([]));
    assert_eq!(font["video"], serde_json::Value::Null);
}

#[tokio::test]
async fn ass_is_returned_raw_as_utf8_with_a_media_token() {
    let (app, _, media) = library().await;
    let (status, headers, body) = app
        .send_raw(Req::get(&format!(
            "/api/tracks/1/subtitle?format=ass&token={media}"
        )))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "text/plain; charset=utf-8");
    let text = String::from_utf8(body).unwrap();
    assert!(text.starts_with("[Script Info]"), "{text}");
    assert!(
        text.contains("日本語の字幕です"),
        "decoded from Shift_JIS: {text}"
    );

    // Without a format the preferred subtitle, the vtt, comes as WebVTT.
    let (status, _, body) = app
        .send_raw(Req::get(&format!("/api/tracks/1/subtitle?token={media}")))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8(body).unwrap().contains("vtt line"));

    let (status, body) = app
        .send(Req::get(&format!(
            "/api/tracks/1/subtitle?format=srt&token={media}"
        )))
        .await;
    assert_error(status, &body, StatusCode::BAD_REQUEST, "invalid_request");
}

#[tokio::test]
async fn fonts_lists_the_work_s_font_files() {
    let (app, token, media) = library().await;
    let (status, body) = app
        .send(Req::get("/api/tracks/1/fonts").token(&token))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        serde_json::json!([
            {"id": 4, "name": "A.ttf", "url": "/api/tracks/4/stream"},
            {"id": 5, "name": "b.OTF", "url": "/api/tracks/5/stream"},
        ])
    );
    // The listed URL serves the font with a media token.
    std::fs::write(app.library().join("RJ001/fonts/A.ttf"), b"font bytes").unwrap();
    let (status, _, bytes) = app
        .send_raw(Req::get(&format!("/api/tracks/4/stream?token={media}")))
        .await;
    assert_eq!(
        (status, bytes.as_slice()),
        (StatusCode::OK, &b"font bytes"[..])
    );
    // The list itself is JSON for the app, not a media URL.
    let (status, _) = app
        .send(Req::get(&format!("/api/tracks/1/fonts?token={media}")))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, body) = app
        .send(Req::get("/api/tracks/99/fonts").token(&token))
        .await;
    assert_error(status, &body, StatusCode::NOT_FOUND, "not_found");
}

/// A test picture with a tone, encoded as `codecs`.
fn movie(path: &std::path::Path, seconds: u32, codecs: &[&str]) {
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg(format!("testsrc=size=320x240:rate=25:duration={seconds}"))
        .args(["-f", "lavfi", "-i"])
        .arg(format!("sine=duration={seconds}"))
        .args(codecs)
        .arg("-y")
        .arg(path)
        .status()
        .expect("ffmpeg must be on PATH for these tests");
    assert!(status.success());
}

/// A work with an H.264 + AAC mp4 and the same streams in matroska.
async fn videos() -> (App, String, String) {
    videos_in(app().await).await
}

/// [`videos`] on a server with `media.video_transcode = false`.
async fn videos_without_transcoding() -> (App, String, String) {
    let app = app_with(|state, dir| {
        state.with_media(neo_server::http::Media {
            cache: neo_media::TranscodeCache::open(&dir.join("cache"), u64::MAX, 0).unwrap(),
            probe_slots: std::sync::Arc::new(tokio::sync::Semaphore::new(4)),
            queue_limit: 32,
            job_timeout: std::time::Duration::from_secs(60),
            remote_analysis_delay: std::time::Duration::from_millis(500),
            video_transcode: false,
        })
    })
    .await;
    videos_in(app).await
}

async fn videos_in(app: App) -> (App, String, String) {
    app.user("admin", true).await;
    let alice = app.user("alice", false).await;
    let dir = app.library().join("RJ002");
    std::fs::create_dir_all(&dir).unwrap();
    let h264 = ["-c:v", "libx264", "-g", "25", "-c:a", "aac"];
    movie(&dir.join("a.mp4"), 8, &h264);
    movie(&dir.join("b.mkv"), 8, &h264);
    app.exec(
        "INSERT INTO roots (id, display_name) VALUES ('lib', 'Lib');
         INSERT INTO works (id, code, title, metadata_src, created_at, updated_at)
           VALUES (2, 'RJ000002', 'Videos', 'scan', 0, 0);
         INSERT INTO work_locations (id, work_id, root_id, rel_path, display_path)
           VALUES (2, 2, 'lib', CAST('RJ002' AS BLOB), 'RJ002');
         INSERT INTO tracks (id, work_id, location_id, rel_path, display_path, sort_key, kind) VALUES
           (21, 2, 2, CAST('a.mp4' AS BLOB), 'a.mp4', 'a', 'video'),
           (22, 2, 2, CAST('b.mkv' AS BLOB), 'b.mkv', 'b', 'video');",
    )
    .await;
    let version: i64 = sqlx::query_scalar("SELECT token_version FROM users WHERE id = ?")
        .bind(alice)
        .fetch_one(&app.db.sqlite().unwrap().read)
        .await
        .unwrap();
    let media = app.media_token(alice, version);
    let token = app.login("alice").await;
    (app, token, media)
}

fn browser() -> serde_json::Value {
    serde_json::json!({"can_play": ["video/mp4", "audio/mpeg", "audio/mp4"]})
}

async fn wait_done(app: &App, token: &str, task_id: &serde_json::Value) -> serde_json::Value {
    for _ in 0..600 {
        let (_, task) = app
            .send(Req::get(&format!("/api/tasks/{task_id}")).token(token))
            .await;
        if !matches!(task["state"].as_str(), Some("queued" | "running")) {
            return task;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("task {task_id} did not finish");
}

#[tokio::test]
async fn an_mp4_the_browser_plays_is_streamed_as_it_is() {
    let (app, token, _) = videos().await;
    let (status, body) = app
        .send(
            Req::post("/api/tracks/21/prepare")
                .token(&token)
                .json(browser()),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["url"], "/api/tracks/21/stream");
    assert_eq!(body["mime_type"], "video/mp4");
}

#[tokio::test]
async fn matroska_is_remuxed_to_hls_with_signed_segments() {
    let (app, token, media) = videos().await;
    let (status, _) = app
        .send(Req::get("/api/tracks/22/hls/index.m3u8").token(&token))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "nothing published yet");

    let (status, body) = app
        .send(
            Req::post("/api/tracks/22/prepare")
                .token(&token)
                .json(browser()),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let task = wait_done(&app, &token, &body["task_id"]).await;
    assert_eq!(task["state"], "done", "{task}");
    assert_eq!(task["result"]["url"], "/api/tracks/22/hls/index.m3u8");
    assert_eq!(task["result"]["mime_type"], "application/vnd.apple.mpegurl");

    // A second prepare finds the published set.
    let (status, body) = app
        .send(
            Req::post("/api/tracks/22/prepare")
                .token(&token)
                .json(browser()),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["url"], "/api/tracks/22/hls/index.m3u8");

    // With a media token in the query, the segments carry that token.
    let (status, headers, playlist) = app
        .send_raw(Req::get(&format!(
            "/api/tracks/22/hls/index.m3u8?token={media}"
        )))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "application/vnd.apple.mpegurl");
    assert_eq!(headers["cache-control"], "private, no-store");
    let playlist = String::from_utf8(playlist).unwrap();
    let segments: Vec<&str> = playlist.lines().filter(|l| !l.starts_with('#')).collect();
    assert_eq!(segments.len(), 2, "8 s in 6 s segments: {playlist}");
    let (set, rest) = segments[0].split_once('/').unwrap();
    let (digest, generation) = set.split_once('-').unwrap();
    assert!(
        digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()),
        "{set}"
    );
    assert!(generation.bytes().all(|b| b.is_ascii_digit()), "{set}");
    assert_eq!(rest, format!("seg-00000.ts?token={media}"));

    // The URI as the player resolves it works without any other auth, and
    // answers Range requests.
    let first = format!("/api/tracks/22/hls/{}", segments[0]);
    let (status, headers, bytes) = app.send_raw(Req::get(&first)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["content-type"], "video/mp2t");
    assert!(bytes.len() > 188 && bytes[0] == 0x47, "an MPEG-TS packet");
    let (status, headers, part) = app
        .send_raw(Req::get(&first).header("range", "bytes=0-187"))
        .await;
    assert_eq!(status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(part.len(), 188);
    assert_eq!(
        headers["content-range"],
        format!("bytes 0-187/{}", bytes.len())
    );
}

#[tokio::test]
async fn an_api_token_is_never_copied_into_the_playlist() {
    let (app, token, _) = videos().await;
    let (_, body) = app
        .send(
            Req::post("/api/tracks/22/prepare")
                .token(&token)
                .json(browser()),
        )
        .await;
    wait_done(&app, &token, &body["task_id"]).await;
    let (status, _, playlist) = app
        .send_raw(Req::get("/api/tracks/22/hls/index.m3u8").token(&token))
        .await;
    assert_eq!(status, StatusCode::OK);
    let playlist = String::from_utf8(playlist).unwrap();
    assert!(!playlist.contains(&token), "{playlist}");
    let first = playlist.lines().find(|l| l.contains("/seg-")).unwrap();
    let (status, _, _) = app
        .send_raw(Req::get(&format!("/api/tracks/22/hls/{first}")))
        .await;
    assert_eq!(status, StatusCode::OK, "the minted media token works");
}

#[tokio::test]
async fn segment_names_are_checked_and_auth_is_required() {
    let (app, token, _) = videos().await;
    let (_, body) = app
        .send(
            Req::post("/api/tracks/22/prepare")
                .token(&token)
                .json(browser()),
        )
        .await;
    wait_done(&app, &token, &body["task_id"]).await;
    let set = current_set(&app, &token, 22).await;
    for bad in [
        format!("{set}/seg-1.ts"),
        format!("{set}/index.m3u8"),
        format!("{set}/..%2Fseg-00000.ts"),
        format!("{set}/seg-99999.ts"),
        "seg-00000.ts".to_owned(),
    ] {
        let (status, body) = app
            .send(Req::get(&format!("/api/tracks/22/hls/{bad}")).token(&token))
            .await;
        assert_error(status, &body, StatusCode::NOT_FOUND, "not_found");
    }
    // A set that does not exist, or is not a digest at all, is not ready.
    for missing in [&"0".repeat(64), "copy", "ABCDEF"] {
        let (status, body) = app
            .send(Req::get(&format!("/api/tracks/22/hls/{missing}/seg-00000.ts")).token(&token))
            .await;
        assert_error(status, &body, StatusCode::CONFLICT, "media_not_ready");
    }
    let (status, body) = app
        .send(Req::get("/api/tracks/x/hls/index.m3u8").token(&token))
        .await;
    assert_error(status, &body, StatusCode::BAD_REQUEST, "invalid_request");
    let (status, _) = app
        .send(Req::get(&format!("/api/tracks/22/hls/{set}/seg-00000.ts")))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// The set a new playlist for `track` points at.
async fn current_set(app: &App, token: &str, track: i64) -> String {
    let (_, _, playlist) = app
        .send_raw(Req::get(&format!("/api/tracks/{track}/hls/index.m3u8")).token(token))
        .await;
    let playlist = String::from_utf8(playlist).unwrap();
    let line = playlist.lines().find(|l| l.contains("/seg-")).unwrap();
    line.split_once('/').unwrap().0.to_owned()
}

#[tokio::test]
async fn a_fallback_request_transcodes_even_a_playable_mp4() {
    let (app, token, _) = videos().await;
    let mut request = browser();
    request["force_fallback"] = true.into();
    let (status, body) = app
        .send(
            Req::post("/api/tracks/21/prepare")
                .token(&token)
                .json(request),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let task = wait_done(&app, &token, &body["task_id"]).await;
    assert_eq!(task["state"], "done", "{task}");
    let (status, _, playlist) = app
        .send_raw(Req::get("/api/tracks/21/hls/index.m3u8").token(&token))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        String::from_utf8(playlist)
            .unwrap()
            .contains("#EXT-X-ENDLIST")
    );
}

#[tokio::test]
async fn with_video_transcoding_off_a_remux_plays_and_a_transcode_is_refused() {
    let (app, token, _) = videos_without_transcoding().await;
    let (status, body) = app
        .send(
            Req::post("/api/tracks/22/prepare")
                .token(&token)
                .json(browser()),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let task = wait_done(&app, &token, &body["task_id"]).await;
    assert_eq!(task["state"], "done", "{task}");

    let mut request = browser();
    request["force_fallback"] = true.into();
    let (status, body) = app
        .send(
            Req::post("/api/tracks/21/prepare")
                .token(&token)
                .json(request),
        )
        .await;
    assert_error(status, &body, StatusCode::FORBIDDEN, "forbidden");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("视频转码"),
        "{body}"
    );
}

#[tokio::test]
async fn a_playlist_keeps_its_rendition_when_a_transcode_appears() {
    let (app, token, media) = videos().await;
    let (_, body) = app
        .send(
            Req::post("/api/tracks/22/prepare")
                .token(&token)
                .json(browser()),
        )
        .await;
    wait_done(&app, &token, &body["task_id"]).await;
    let copy_set = current_set(&app, &token, 22).await;
    let copy_segment = format!("/api/tracks/22/hls/{copy_set}/seg-00000.ts?token={media}");
    let (_, _, before) = app.send_raw(Req::get(&copy_segment)).await;

    let mut fallback = browser();
    fallback["force_fallback"] = true.into();
    let (_, body) = app
        .send(
            Req::post("/api/tracks/22/prepare")
                .token(&token)
                .json(fallback),
        )
        .await;
    assert_eq!(
        wait_done(&app, &token, &body["task_id"]).await["state"],
        "done"
    );

    // The copy playlist's segments still come from the copy.
    let (status, _, after) = app.send_raw(Req::get(&copy_segment)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(before, after);
    // A new playlist now points at the transcode.
    assert_ne!(current_set(&app, &token, 22).await, copy_set);
    // And a remux request is answered from it instead of copying again.
    let (status, body) = app
        .send(
            Req::post("/api/tracks/22/prepare")
                .token(&token)
                .json(browser()),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn a_playlist_keeps_its_set_when_the_source_changes() {
    let (app, token, media) = videos().await;
    let (_, body) = app
        .send(
            Req::post("/api/tracks/22/prepare")
                .token(&token)
                .json(browser()),
        )
        .await;
    wait_done(&app, &token, &body["task_id"]).await;
    let old_set = current_set(&app, &token, 22).await;
    let old_segment = format!("/api/tracks/22/hls/{old_set}/seg-00000.ts?token={media}");
    let (_, _, before) = app.send_raw(Req::get(&old_segment)).await;

    // A different file now sits at the same path.
    movie(
        &app.library().join("RJ002/b.mkv"),
        14,
        &["-c:v", "libx264", "-g", "50", "-c:a", "aac"],
    );
    let (status, body) = app
        .send(
            Req::post("/api/tracks/22/prepare")
                .token(&token)
                .json(browser()),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    wait_done(&app, &token, &body["task_id"]).await;
    assert_ne!(current_set(&app, &token, 22).await, old_set);

    // The old playlist's segments still come from the old set.
    let (status, _, after) = app.send_raw(Req::get(&old_segment)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(before, after);
}

#[tokio::test]
async fn a_video_transcode_queued_under_other_settings_is_interrupted_on_restart() {
    let (app, token, _) = videos().await;
    // From before the 1080p limit (no params), and from other settings.
    app.exec(
        "INSERT INTO tasks (id, kind, dedupe_key, state, payload, created_at, updated_at) VALUES
           (60, 'transcode', 'a', 'queued',
            '{\"track_id\": 21, \"key\": \"k\", \"output\": \"hls-transcode\"}', 0, 0),
           (61, 'transcode', 'b', 'queued',
            '{\"track_id\": 21, \"key\": \"k\", \"output\": \"hls-transcode\", \"params\": \"hls-h264-aac-6s\"}', 0, 0)",
    )
    .await;
    let state = app.state.clone();
    app.state
        .scheduler()
        .recover(|row| neo_server::http::rebuild_task(&state, row))
        .await
        .unwrap();
    let admin = app.login("admin").await;
    for id in [60, 61] {
        let (_, body) = app
            .send(Req::get(&format!("/api/tasks/{id}")).token(&admin))
            .await;
        assert_eq!(body["state"], "interrupted", "{id}: {body}");
    }
    // The player's next prepare starts it again under the current settings.
    let mut request = browser();
    request["force_fallback"] = true.into();
    let (status, body) = app
        .send(
            Req::post("/api/tracks/21/prepare")
                .token(&token)
                .json(request),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
}
