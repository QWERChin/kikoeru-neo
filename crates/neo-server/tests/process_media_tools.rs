#![cfg(unix)]

mod process;

use std::{os::unix::fs::PermissionsExt, time::Duration};

#[tokio::test]
async fn only_serve_checks_tools_and_readiness_does_not_wait_for_them() {
    let dir = process::data_dir();
    let fake = dir.path().join("ffmpeg");
    std::fs::write(&fake, "#!/bin/sh\necho $$ > \"$0.pid\"\nexec sleep 30\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        format!("listen = \"127.0.0.1:0\"\n[media]\nffmpeg = {fake:?}\nffprobe = {fake:?}\n"),
    )
    .unwrap();

    process::create_admin(dir.path(), "admin").await;
    assert!(
        !fake.with_extension("pid").exists(),
        "user create ran a media check"
    );
    // start() requires HTTP 200 from /api/ready before returning.
    let mut server = process::Server::start(dir.path()).await;
    let log = dir.path().join("server.log");
    assert!(!std::fs::read_to_string(&log).unwrap().contains("timed out"));
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let log = std::fs::read_to_string(&log).unwrap();
            if log.matches("-version timed out after 5s").count() == 2 {
                assert!(log.contains("media.ffmpeg"));
                assert!(log.contains("media.ffprobe"));
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("serve did not log both timeouts");
    assert_eq!(
        server.request("GET", "/api/ready", None, None).await.0,
        axum::http::StatusCode::OK
    );
    server.crash().await;
}
