//! Startup failures must not consume work queued by the previous server.

#[path = "../../webdav-client/tests/common/mod.rs"]
mod fake;

use std::{path::Path, time::Duration};

use neo_db::{Location, tasks};
use neo_server::{config::Config, datadir::DataDir, serve};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

#[tokio::test]
async fn startup_preserves_config_passwords_before_loading_their_clients() {
    for versioned in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let data = DataDir::prepare(dir.path()).unwrap();
        let db = neo_db::Db::open_sqlite(&data.database()).await.unwrap();
        let admin = neo_db::users::create(&db, "admin", "unused", true, 1, 0)
            .await
            .unwrap();
        db.close().await;
        let key = data.load_or_create_jwt_key().unwrap();
        let token = neo_server::token::Tokens::new(&key)
            .issue(
                admin,
                1,
                neo_server::token::Audience::Api,
                neo_server::token::now_secs(),
            )
            .unwrap()
            .token;
        let secrets = dir.path().join("secrets/roots");
        std::fs::create_dir_all(secrets.join("nested")).unwrap();
        let filename = if versioned {
            "cloud.0123456789abcdef0123456789abcdef.password"
        } else {
            "cloud.password"
        };
        let password = secrets.join(filename);
        std::fs::write(&password, fake::PASSWORD).unwrap();
        // Resolve both a parent component and a symlink before comparing
        // the configured path with files found in the managed directory.
        let configured = secrets.join("nested/..").join(filename);
        #[cfg(unix)]
        let configured = {
            let alias = dir.path().join("password-link");
            std::os::unix::fs::symlink(&configured, &alias).unwrap();
            alias
        };
        let orphan = secrets.join("unused.0123456789abcdef0123456789abcdef.password");
        std::fs::write(&orphan, "unused synthetic password").unwrap();
        let media = fake::start().await;
        let config = Config::parse(
            &format!(
                "listen = \"127.0.0.1:0\"\n\
                 [[roots]]\nid = \"cloud\"\ndisplay_name = \"Cloud\"\ntype = \"webdav\"\n\
                 url = {:?}\nbase_path = \"/\"\nusername = {:?}\npassword_file = {:?}\n\
                 [metadata]\nsource = \"none\"\n",
                media.url(),
                fake::USER,
                configured.to_str().unwrap(),
            ),
            Path::new("config.toml"),
        )
        .unwrap();
        let result = serve::start(data, config, None).await;
        assert!(
            result.is_ok(),
            "config password must survive startup: {:?}",
            result.err()
        );
        let running = result.unwrap();
        assert_eq!(std::fs::read_to_string(&password).unwrap(), fake::PASSWORD);
        assert!(!orphan.exists(), "startup must still reclaim owned orphans");
        // An API edit performs the same sweep after startup, when config
        // paths must still be available even though the registry hides them.
        std::fs::write(&orphan, "another unused synthetic password").unwrap();
        let body = serde_json::json!({
            "id": "extra", "display_name": "Extra",
            "settings": {"type": "local", "path": dir.path(), "watch": false},
        })
        .to_string();
        let request = format!(
            "POST /api/admin/roots HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len(),
        );
        let response = tokio::time::timeout(Duration::from_secs(5), async {
            let mut client = TcpStream::connect(running.address()).await.unwrap();
            client.write_all(request.as_bytes()).await.unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).await.unwrap();
            response
        })
        .await
        .unwrap();
        assert!(response.starts_with("HTTP/1.1 201"), "{response}");
        assert_eq!(std::fs::read_to_string(password).unwrap(), fake::PASSWORD);
        assert!(
            !orphan.exists(),
            "runtime cleanup must reclaim owned orphans"
        );
        running.stop().await.unwrap();
    }
}

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
