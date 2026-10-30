//! Process exits bypass all destructors, leaving each publication step for
//! the next instance to recover. Every password and address is synthetic.

use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use neo_db::{Db, roots};
use serde_json::json;
use tower::ServiceExt;

use crate::{
    config::{Config, RootBackend},
    datadir::DataDir,
    http, serve,
    token::{self, Audience, Tokens},
};

static ARMED: AtomicBool = AtomicBool::new(false);

pub(crate) fn crash_at(step: &str) {
    if ARMED.load(Ordering::SeqCst) && std::env::var("NEO_TEST_SECRET_CRASH").as_deref() == Ok(step)
    {
        std::process::exit(86);
    }
}

fn config() -> Config {
    Config::parse(
        "listen = '127.0.0.1:0'\n[metadata]\nsource = 'none'",
        Path::new("config.toml"),
    )
    .unwrap()
}

async fn seed(path: &Path) -> DataDir {
    seed_layout(path, "webdav", false).await
}

async fn seed_layout(path: &Path, kind: &str, versioned: bool) -> DataDir {
    let dir = DataDir::prepare(path).unwrap();
    let db = Db::open_sqlite(&dir.database()).await.unwrap();
    neo_db::users::create(&db, "admin", "unused", true, 1, 0)
        .await
        .unwrap();
    let mut settings =
        json!({"type": kind, "url": "http://old.test/", "base_path": "/", "username": "u"});
    if versioned {
        settings["password_version"] =
            json!(super::write_password(dir.root(), "remote", "old-password").unwrap());
    } else {
        crate::private::dir()
            .create(dir.root().join("secrets/roots"))
            .unwrap();
        std::fs::write(super::password_path(dir.root(), "remote"), "old-password").unwrap();
        crate::private::restrict_file(&super::password_path(dir.root(), "remote")).unwrap();
    }
    roots::insert_stored(
        &db,
        &roots::StoredRoot {
            id: "remote".into(),
            display_name: "Remote".into(),
            kind: kind.into(),
            settings: settings.to_string(),
        },
        0,
    )
    .await
    .unwrap();
    db.close().await;
    dir
}

fn edit(dir: &DataDir) -> Request<Body> {
    request(dir, "webdav", "edit")
}

fn request(dir: &DataDir, kind: &str, operation: &str) -> Request<Body> {
    let key = dir.load_or_create_jwt_key().unwrap();
    let token = Tokens::new(&key)
        .issue(1, 1, Audience::Api, token::now_secs())
        .unwrap()
        .token;
    let (method, uri) = match operation {
        "create" => ("POST", "/api/admin/roots"),
        "delete" => ("DELETE", "/api/admin/roots/remote"),
        _ => ("PUT", "/api/admin/roots/remote"),
    };
    let settings = if operation == "local" {
        json!({"type": "local", "path": dir.root(), "watch": false})
    } else {
        json!({"type": kind, "url": "http://new.test/", "base_path": "/", "username": "u", "password": "new-password"})
    };
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "id": "remote", "display_name": "Remote", "skip_check": true, "settings": settings,
            })
            .to_string(),
        ))
        .unwrap();
    request.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:12345".parse::<std::net::SocketAddr>().unwrap(),
    ));
    request
}

#[tokio::test]
#[ignore = "subprocess helper for crash injection"]
async fn crash_child() {
    let path = std::env::var_os("NEO_TEST_SECRET_DIR").unwrap();
    let dir = DataDir::existing(Path::new(&path)).unwrap();
    let operation = std::env::var("NEO_TEST_SECRET_OPERATION").unwrap();
    if operation == "recover" {
        ARMED.store(true, Ordering::SeqCst);
    }
    let running = serve::start(dir.clone(), config(), None).await.unwrap();
    ARMED.store(true, Ordering::SeqCst);
    let kind = std::env::var("NEO_TEST_SECRET_KIND").unwrap();
    let response = http::router(running.state().clone())
        .oneshot(request(&dir, &kind, &operation))
        .await
        .unwrap();
    panic!("crash point was not reached: {}", response.status());
}

#[tokio::test]
async fn crash_before_database_commit_keeps_the_committed_credentials() {
    for kind in ["webdav", "openlist"] {
        for versioned in [false, true] {
            for step in [
                "password_created",
                "password_written",
                "password_synced",
                "password_published",
            ] {
                let tmp = tempfile::tempdir().unwrap();
                let dir = seed_layout(tmp.path(), kind, versioned).await;
                crash(&dir, step, kind, "edit").await;
                assert_reopened(&dir, "http://old.test/", "old-password").await;
            }
        }
    }
}

async fn crash(dir: &DataDir, step: &str, kind: &str, operation: &str) {
    let status = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "roots::crash_tests::crash_child",
            "--ignored",
            "--nocapture",
        ])
        .env("NEO_TEST_SECRET_DIR", dir.root())
        .env("NEO_TEST_SECRET_CRASH", step)
        .env("NEO_TEST_SECRET_KIND", kind)
        .env("NEO_TEST_SECRET_OPERATION", operation)
        .status()
        .await
        .unwrap();
    assert_eq!(
        status.code(),
        Some(86),
        "child must exit at the injected crash"
    );
}

async fn assert_reopened(dir: &DataDir, expected_url: &str, expected_password: &str) {
    let running = serve::start(dir.clone(), config(), None).await.unwrap();
    let db = Db::open_sqlite(&dir.database()).await.unwrap();
    let rows = roots::list_stored(&db).await.unwrap();
    let (_, backend) = super::stored_entry(dir.root(), &rows[0], false).unwrap();
    let (url, password_file) = match backend {
        RootBackend::WebDav {
            url, password_file, ..
        }
        | RootBackend::OpenList {
            url, password_file, ..
        } => (url, password_file),
        _ => panic!("remote root"),
    };
    let password = std::fs::read_to_string(password_file).unwrap();
    assert!(running.state().storage("remote").is_some());
    running.stop().await.unwrap();
    db.close().await;
    assert_eq!(
        (url.as_str(), password.as_str()),
        (expected_url, expected_password)
    );
    assert_eq!(
        std::fs::read_dir(dir.root().join("secrets/roots"))
            .unwrap()
            .count(),
        1,
        "only the committed password survives recovery"
    );
}

#[tokio::test]
async fn crash_after_commit_or_during_cleanup_keeps_the_new_credentials() {
    for kind in ["webdav", "openlist"] {
        for versioned in [false, true] {
            for step in [
                "database_committed",
                "references_synced",
                "password_removed",
            ] {
                let tmp = tempfile::tempdir().unwrap();
                let dir = seed_layout(tmp.path(), kind, versioned).await;
                crash(&dir, step, kind, "edit").await;
                assert_reopened(&dir, "http://new.test/", "new-password").await;
            }
        }
    }
}

#[tokio::test]
async fn recovery_can_crash_again_without_removing_a_live_password() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = seed(tmp.path()).await;
    crash(&dir, "password_published", "webdav", "edit").await;
    crash(&dir, "password_removed", "webdav", "recover").await;
    assert_reopened(&dir, "http://old.test/", "old-password").await;
}

#[tokio::test]
async fn startup_cleanup_preserves_passwords_of_rows_shadowed_by_config() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = seed(tmp.path()).await;
    super::write_password(dir.root(), "orphan", "uncommitted-password").unwrap();
    let config = Config::parse(
        &format!("listen = '127.0.0.1:0'\n[[roots]]\nid = 'remote'\ndisplay_name = 'Local override'\ntype = 'local'\npath = {:?}\n[metadata]\nsource = 'none'", dir.root()),
        Path::new("config.toml"),
    ).unwrap();
    let running = serve::start(dir.clone(), config, None).await.unwrap();
    assert_eq!(
        running.state().roots()[0].source,
        neo_core::dto::RootSource::Config
    );
    assert_eq!(
        std::fs::read_to_string(super::password_path(dir.root(), "remote")).unwrap(),
        "old-password"
    );
    assert_eq!(
        std::fs::read_dir(dir.root().join("secrets/roots"))
            .unwrap()
            .count(),
        1
    );
    running.stop().await.unwrap();
}

#[tokio::test]
async fn creation_removal_and_local_conversion_recover_after_crashes() {
    for operation in ["create", "delete", "local"] {
        let steps = if operation == "create" {
            ["password_published", "database_committed"]
        } else {
            ["database_committed", "password_removed"]
        };
        for step in steps {
            let tmp = tempfile::tempdir().unwrap();
            // Deletion is only owned for versioned files. Legacy files without
            // a committed replacement are covered by password_cleanup tests.
            let dir = seed_layout(tmp.path(), "webdav", true).await;
            if operation == "create" {
                let db = Db::open_sqlite(&dir.database()).await.unwrap();
                let mut tx = db.begin().await.unwrap();
                roots::remove_stored(&mut tx, "remote", 1).await.unwrap();
                tx.commit().await.unwrap();
                super::cleanup_passwords(dir.root(), &db, &[])
                    .await
                    .unwrap();
                db.close().await;
            }
            crash(&dir, step, "webdav", operation).await;
            if operation == "create" && step == "database_committed" {
                assert_reopened(&dir, "http://new.test/", "new-password").await;
            } else {
                let running = serve::start(dir.clone(), config(), None).await.unwrap();
                let roots = running.state().roots();
                if operation == "local" {
                    assert_eq!(roots.len(), 1);
                    assert_eq!(roots[0].kind, neo_core::dto::RootKind::Local);
                } else {
                    assert!(roots.is_empty());
                }
                assert_eq!(
                    std::fs::read_dir(dir.root().join("secrets/roots"))
                        .unwrap()
                        .count(),
                    0
                );
                running.stop().await.unwrap();
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_keeps_the_instance_locked_while_an_edit_waits_for_secrets() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = seed(tmp.path()).await;
    let running = serve::start(dir.clone(), config(), None).await.unwrap();
    let state = running.state().clone();
    let secrets = super::lock_secrets(dir.root()).unwrap();
    let request = tokio::spawn(http::router(state.clone()).oneshot(edit(&dir)));
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if tokio::time::timeout(Duration::from_millis(50), state.lock_root_changes())
                .await
                .is_err()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("edit did not reach the secrets lock");
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());

    tokio::time::timeout(Duration::from_secs(25), running.stop())
        .await
        .unwrap()
        .unwrap();
    let still_locked = dir.lock_instance().is_err();
    if still_locked {
        let error = serve::start(dir.clone(), config(), None)
            .await
            .err()
            .expect("restart must be refused");
        assert!(error.to_string().contains("another kikoeru-neo server"));
        let late = http::router(state.clone())
            .oneshot(edit(&dir))
            .await
            .unwrap();
        assert_eq!(late.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
    drop(secrets);
    tokio::time::timeout(Duration::from_secs(5), async {
        while dir.lock_instance().is_err() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("instance did not release the lock after the edit finished");
    assert!(
        still_locked,
        "shutdown released the instance lock before the blocked edit finished"
    );
    assert_reopened(&dir, "http://new.test/", "new-password").await;
}
