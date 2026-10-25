//! Startup failures must not consume work queued by the previous server.

#[path = "../../webdav-client/tests/common/mod.rs"]
mod fake;

use std::{path::Path, time::Duration};

use neo_db::{Location, tasks};
use neo_server::{config::Config, datadir::DataDir, serve};
use tokio::net::TcpListener;

#[tokio::test]
async fn occupied_port_leaves_scans_queued_until_a_successful_start() {
    let dir = tempfile::tempdir().unwrap();
    let database = neo_db::testing::open().await;
    let (data_root, database_config) = match &database.location {
        Location::Sqlite(path) => (path.parent().unwrap().to_owned(), String::new()),
        #[cfg(feature = "postgres")]
        Location::Postgres(url) => (
            dir.path().join("data"),
            format!("[database]\nurl = {url:?}\n"),
        ),
    };
    let data_dir = DataDir::prepare(&data_root).unwrap();
    let media = fake::start().await;
    let password = dir.path().join("webdav-password");
    std::fs::write(&password, fake::PASSWORD).unwrap();
    let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = occupied.local_addr().unwrap();
    let config = format!(
        "listen = \"{address}\"\n\
         [[roots]]\nid = \"lib\"\ndisplay_name = \"Library\"\ntype = \"webdav\"\n\
         url = {:?}\nbase_path = \"/\"\nusername = {:?}\npassword_file = {:?}\n\
         [metadata]\nsource = \"none\"\n{database_config}",
        media.url(),
        fake::USER,
        password.to_str().unwrap(),
    );
    let config = || Config::parse(&config, Path::new("config.toml")).unwrap();
    let tasks::Enqueued::Created(id) = tasks::enqueue(
        &database.db,
        "scan",
        Some("scan:lib"),
        r#"{"root":"lib"}"#,
        None,
        0,
    )
    .await
    .unwrap() else {
        panic!("the scan was not queued");
    };

    let error = serve::start(data_dir.clone(), config(), None)
        .await
        .err()
        .expect("the occupied port must prevent startup");
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::AddrInUse,
    );
    // Keep polling on the same runtime: a detached recovery job would
    // otherwise disappear when the failed server's runtime shuts down.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    loop {
        let row = tasks::get(&database.db, id).await.unwrap().unwrap();
        assert_eq!(row.state, "queued", "a failed startup consumed the scan");
        assert!(media.fake.log.lock().unwrap().is_empty());
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    drop(occupied);
    let running = serve::start(data_dir, config(), None).await.unwrap();
    assert_eq!(running.address(), address);
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let row = tasks::get(&database.db, id).await.unwrap().unwrap();
            if !matches!(row.state.as_str(), "queued" | "running") {
                assert_eq!(row.state, "done", "{row:?}");
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the queued scan did not resume");
    assert!(media.fake.requests("PROPFIND") > 0);
    running.stop().await.unwrap();
    database.db.close().await;
}
