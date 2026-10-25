//! Metadata refresh and the scan's metadata stage, against a local fake
//! DLsite. Nothing here touches the network.

mod common;

use std::{sync::Arc, time::Duration};

use axum::{Json, Router, extract::Query, http::StatusCode, routing::get};
use common::*;
use neo_scanner::metadata::{CoverDirs, DlsiteConfig, DlsiteProvider};
use neo_server::scan::MetadataEnv;
use serde_json::{Value, json};

async fn fake_dlsite() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    let img = format!("{base}img/main.jpg");
    let app = Router::new()
        .route(
            "/maniax/api/=/product.json",
            get(
                move |Query(q): Query<std::collections::HashMap<String, String>>| {
                    let img = img.clone();
                    async move {
                        let code = q.get("workno").cloned().unwrap_or_default();
                        if code == "RJ404404" {
                            return Json(json!([]));
                        }
                        Json(json!([{
                            "product_name": format!("Fetched {code}"),
                            "maker_id": "RG100",
                            "maker_name": "Fetched Circle",
                            "age_category": 1,
                            "genres": [{"id": 497, "name": "ASMR"}],
                            "image_main": {"url": img},
                        }]))
                    }
                },
            ),
        )
        .route(
            "/maniax/product/info/ajax",
            get(|| async { Json(json!({})) }),
        )
        .route("/img/main.jpg", get(|| async { &b"jpeg"[..] }));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    base
}

async fn setup() -> (App, String) {
    let (app, token, _) = setup_with_dirs().await;
    (app, token)
}

async fn setup_with_dirs() -> (App, String, CoverDirs) {
    let base = fake_dlsite().await;
    let dirs = std::sync::Arc::new(std::sync::Mutex::new(None));
    let kept = dirs.clone();
    let app = app_with(move |state, dir| {
        let provider = DlsiteProvider::new(DlsiteConfig {
            base: base.parse().unwrap(),
            spacing: Duration::ZERO,
            backoff: Duration::from_millis(1),
            ..DlsiteConfig::default()
        })
        .unwrap();
        let cover_dirs = CoverDirs::new(dir.join("covers"), dir.join("cache/tmp"));
        *kept.lock().unwrap() = Some(cover_dirs.clone());
        state.with_metadata(MetadataEnv {
            provider: Arc::new(provider),
            dirs: cover_dirs,
        })
    })
    .await;
    app.user("admin", true).await;
    let token = app.login("admin").await;
    app.exec("INSERT INTO roots (id, display_name) VALUES ('lib', 'Library')")
        .await;
    let dirs = dirs.lock().unwrap().take().unwrap();
    (app, token, dirs)
}

async fn finished(app: &App, token: &str, id: i64) -> Value {
    for _ in 0..200 {
        let (_, body) = app
            .send(Req::get(&format!("/api/tasks/{id}")).token(token))
            .await;
        if !matches!(body["state"].as_str(), Some("queued" | "running")) {
            return body;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("task {id} did not finish");
}

async fn title(app: &App, id: i64) -> (String, String) {
    sqlx::query_as("SELECT title, metadata_src FROM works WHERE id = ?")
        .bind(id)
        .fetch_one(&app.db.sqlite().unwrap().read)
        .await
        .unwrap()
}

#[tokio::test]
async fn refresh_respects_manual_metadata_unless_forced() {
    let (app, token) = setup().await;
    app.exec(
        "INSERT INTO works (id, code, title, metadata_src, created_at, updated_at) VALUES
           (1, 'RJ01000001', 'hand made', 'manual', 0, 0),
           (2, 'RJ404404', 'unknown', 'none', 0, 0)",
    )
    .await;

    let (status, body) = app
        .send(Req::post("/api/works/1/refresh").token(&token))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_same_shape(&body, &sample_in("m4", "refresh-response.json"), "refresh");
    let task = finished(&app, &token, body["task_id"].as_i64().unwrap()).await;
    assert_eq!(task["state"], "done", "{task}");
    assert_eq!(title(&app, 1).await, ("hand made".into(), "manual".into()));

    let (_, body) = app
        .send(Req::post("/api/works/1/refresh?force=true").token(&token))
        .await;
    let task = finished(&app, &token, body["task_id"].as_i64().unwrap()).await;
    assert_eq!(task["state"], "done", "{task}");
    assert_eq!(
        title(&app, 1).await,
        ("Fetched RJ01000001".into(), "dlsite".into())
    );
    assert_eq!(
        std::fs::read(app.data_dir().join("covers/1.jpg")).unwrap(),
        b"jpeg"
    );

    let (_, body) = app
        .send(Req::post("/api/works/2/refresh").token(&token))
        .await;
    let task = finished(&app, &token, body["task_id"].as_i64().unwrap()).await;
    assert_eq!(task["state"], "failed");
    assert!(
        task["message"].as_str().unwrap().contains("DLsite"),
        "{task}"
    );

    for (path, status, code) in [
        ("/api/works/99/refresh", StatusCode::NOT_FOUND, "not_found"),
        (
            "/api/works/1/refresh?force=maybe",
            StatusCode::BAD_REQUEST,
            "invalid_request",
        ),
    ] {
        let (got, body) = app.send(Req::post(path).token(&token)).await;
        assert_error(got, &body, status, code);
    }
    app.user("bob", false).await;
    let bob = app.login("bob").await;
    let (got, body) = app
        .send(Req::post("/api/works/1/refresh").token(&bob))
        .await;
    assert_error(got, &body, StatusCode::FORBIDDEN, "forbidden");
}

#[tokio::test]
async fn a_scan_fetches_metadata_for_new_works() {
    let (app, token) = setup().await;
    std::fs::create_dir_all(app.library().join("RJ01000003")).unwrap();
    std::fs::write(app.library().join("RJ01000003/01.mp3"), b"audio").unwrap();
    let (_, body) = app
        .send(
            Req::post("/api/admin/scan")
                .token(&token)
                .json(json!({"root": "lib"})),
        )
        .await;
    let task = finished(&app, &token, body["task_ids"][0].as_i64().unwrap()).await;
    assert_eq!(task["state"], "done", "{task}");
    let (title, src): (String, String) =
        sqlx::query_as("SELECT title, metadata_src FROM works WHERE code = 'RJ01000003'")
            .fetch_one(&app.db.sqlite().unwrap().read)
            .await
            .unwrap();
    assert_eq!(
        (title.as_str(), src.as_str()),
        ("Fetched RJ01000003", "dlsite")
    );
}

#[tokio::test]
async fn a_cover_that_could_not_be_put_back_is_not_served() {
    let (app, token, dirs) = setup_with_dirs().await;
    app.exec(
        "INSERT INTO works (id, code, title, metadata_src, cover_src, created_at, updated_at)
         VALUES (1, 'RJ01000001', 't', 'dlsite', 'dlsite', 0, 0)",
    )
    .await;
    std::fs::write(app.data_dir().join("covers/1.jpg"), b"cover").unwrap();
    let image = |app: &App| {
        let token = token.clone();
        let router = app.router.clone();
        async move {
            use tower::ServiceExt;
            let request = axum::http::Request::get("/api/works/1/cover")
                .header("authorization", format!("Bearer {token}"))
                .extension(axum::extract::ConnectInfo(
                    "192.0.2.10:1".parse::<std::net::SocketAddr>().unwrap(),
                ))
                .body(axum::body::Body::empty())
                .unwrap();
            let response = router.oneshot(request).await.unwrap();
            response.headers()["content-type"]
                .to_str()
                .unwrap()
                .to_owned()
        }
    };
    assert_eq!(image(&app).await, "image/jpeg");
    dirs.lock.write().await.insert(1);
    assert_eq!(image(&app).await, "image/svg+xml");
}

#[tokio::test]
async fn a_scan_waiting_on_a_backup_can_be_cancelled() {
    let (app, token, dirs) = setup_with_dirs().await;
    std::fs::create_dir_all(app.library().join("RJ01000003")).unwrap();
    std::fs::write(app.library().join("RJ01000003/01.mp3"), b"audio").unwrap();
    let backup = neo_scanner::metadata::lock_covers(&dirs.covers).unwrap();
    let (_, body) = app
        .send(
            Req::post("/api/admin/scan")
                .token(&token)
                .json(json!({"root": "lib"})),
        )
        .await;
    let id = body["task_ids"][0].as_i64().unwrap();
    // The work is found, then its cover waits for the backup.
    for _ in 0..200 {
        let found: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE code = 'RJ01000003'")
            .fetch_one(&app.db.sqlite().unwrap().read)
            .await
            .unwrap();
        if found == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (status, _) = app
        .send(Req::post(&format!("/api/tasks/{id}/cancel")).token(&token))
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let task = finished(&app, &token, id).await;
    assert_eq!(task["state"], "cancelled", "{task}");
    drop(backup);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let src: String =
        sqlx::query_scalar("SELECT metadata_src FROM works WHERE code = 'RJ01000003'")
            .fetch_one(&app.db.sqlite().unwrap().read)
            .await
            .unwrap();
    assert_eq!(src, "none");
}

/// Holds the listing of the second work until DLsite has been asked about
/// a work. Metadata fetched only after the whole listing would never open
/// it, and the listing would give up after 10 s.
struct Gated {
    inner: neo_storage::LocalStorage,
    asked: Arc<std::sync::atomic::AtomicBool>,
    opened: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait::async_trait]
impl neo_storage::Storage for Gated {
    async fn list(
        &self,
        dir: &neo_storage::RawPath,
    ) -> Result<Vec<neo_storage::Entry>, neo_storage::StorageError> {
        use std::sync::atomic::Ordering::SeqCst;
        if dir.as_bytes().starts_with(b"RJ01000002") && !self.opened.load(SeqCst) {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while !self.asked.load(SeqCst) && std::time::Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            self.opened.store(self.asked.load(SeqCst), SeqCst);
        }
        self.inner.list(dir).await
    }

    async fn stat(
        &self,
        path: &neo_storage::RawPath,
    ) -> Result<neo_storage::Entry, neo_storage::StorageError> {
        self.inner.stat(path).await
    }

    async fn source(
        &self,
        path: &neo_storage::RawPath,
    ) -> Result<neo_storage::Source, neo_storage::StorageError> {
        self.inner.source(path).await
    }

    async fn access(
        &self,
        path: &neo_storage::RawPath,
    ) -> Result<neo_storage::Access, neo_storage::StorageError> {
        self.inner.access(path).await
    }
}

#[tokio::test]
async fn new_works_get_metadata_while_the_listing_goes_on() {
    use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
    let asked = Arc::new(AtomicBool::new(false));
    let opened = Arc::new(AtomicBool::new(false));
    // A DLsite that notes the first question.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    let flag = asked.clone();
    let dlsite = Router::new()
        .route(
            "/maniax/api/=/product.json",
            get(
                move |Query(q): Query<std::collections::HashMap<String, String>>| {
                    flag.store(true, SeqCst);
                    let code = q.get("workno").cloned().unwrap_or_default();
                    async move { Json(json!([{ "product_name": format!("Fetched {code}") }])) }
                },
            ),
        )
        .route(
            "/maniax/product/info/ajax",
            get(|| async { Json(json!({})) }),
        );
    tokio::spawn(async move { axum::serve(listener, dlsite).await.unwrap() });
    let gate_opened = opened.clone();
    let app = app_with(move |state, dir| {
        let provider = DlsiteProvider::new(DlsiteConfig {
            base: base.parse().unwrap(),
            spacing: Duration::ZERO,
            backoff: Duration::from_millis(1),
            ..DlsiteConfig::default()
        })
        .unwrap();
        state
            .with_metadata(MetadataEnv {
                provider: Arc::new(provider),
                dirs: CoverDirs::new(dir.join("covers"), dir.join("cache/tmp")),
            })
            .with_storages(vec![(
                "lib".to_owned(),
                Arc::new(Gated {
                    inner: neo_storage::LocalStorage::new(dir.join("library")),
                    asked,
                    opened: gate_opened,
                }) as _,
            )])
    })
    .await;
    app.user("admin", true).await;
    let token = app.login("admin").await;
    app.exec("INSERT INTO roots (id, display_name) VALUES ('lib', 'Library')")
        .await;
    for code in ["RJ01000001", "RJ01000002"] {
        std::fs::create_dir_all(app.library().join(code)).unwrap();
        std::fs::write(app.library().join(code).join("01.mp3"), b"audio").unwrap();
    }

    let (status, body) = app
        .send(
            Req::post("/api/admin/scan")
                .token(&token)
                .json(json!({"root": "lib"})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let task = finished(&app, &token, body["task_ids"][0].as_i64().unwrap()).await;
    assert_eq!(task["state"], "done", "{task}");
    assert!(opened.load(SeqCst), "metadata waited for the whole listing");
    let result: String = sqlx::query_scalar("SELECT result FROM tasks WHERE id = ?")
        .bind(task["id"].as_i64().unwrap())
        .fetch_one(&app.db.sqlite().unwrap().read)
        .await
        .unwrap();
    let result: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(
        result["metadata_fetched"], 2,
        "each work is asked once: {result}"
    );
    for id in [1, 2] {
        assert_eq!(title(&app, id).await.1, "dlsite");
    }
}

/// Listing a small local library ends long before DLsite has answered for
/// every work. The task then shows the metadata stage, not a finished
/// listing.
#[tokio::test]
async fn the_task_shows_the_metadata_stage_after_the_listing() {
    use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
    let seen = Arc::new(AtomicBool::new(false));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    let shown = seen.clone();
    // The second work's answer waits until the test has seen the stage.
    let dlsite = Router::new()
        .route(
            "/maniax/api/=/product.json",
            get(
                move |Query(q): Query<std::collections::HashMap<String, String>>| {
                    let shown = shown.clone();
                    async move {
                        let code = q.get("workno").cloned().unwrap_or_default();
                        if code == "RJ01000002" {
                            let deadline = std::time::Instant::now() + Duration::from_secs(10);
                            while !shown.load(SeqCst) && std::time::Instant::now() < deadline {
                                tokio::time::sleep(Duration::from_millis(10)).await;
                            }
                        }
                        Json(json!([{ "product_name": format!("Fetched {code}") }]))
                    }
                },
            ),
        )
        .route(
            "/maniax/product/info/ajax",
            get(|| async { Json(json!({})) }),
        );
    tokio::spawn(async move { axum::serve(listener, dlsite).await.unwrap() });
    let app = app_with(move |state, dir| {
        let provider = DlsiteProvider::new(DlsiteConfig {
            base: base.parse().unwrap(),
            spacing: Duration::ZERO,
            backoff: Duration::from_millis(1),
            ..DlsiteConfig::default()
        })
        .unwrap();
        state.with_metadata(MetadataEnv {
            provider: Arc::new(provider),
            dirs: CoverDirs::new(dir.join("covers"), dir.join("cache/tmp")),
        })
    })
    .await;
    app.user("admin", true).await;
    let token = app.login("admin").await;
    app.exec("INSERT INTO roots (id, display_name) VALUES ('lib', 'Library')")
        .await;
    for code in ["RJ01000001", "RJ01000002"] {
        std::fs::create_dir_all(app.library().join(code)).unwrap();
        std::fs::write(app.library().join(code).join("01.mp3"), b"audio").unwrap();
    }
    let (status, body) = app
        .send(
            Req::post("/api/admin/scan")
                .token(&token)
                .json(json!({"root": "lib"})),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let id = body["task_ids"][0].as_i64().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let message: Option<String> = sqlx::query_scalar("SELECT message FROM tasks WHERE id = ?")
            .bind(id)
            .fetch_one(&app.db.sqlite().unwrap().read)
            .await
            .unwrap();
        if message.as_deref().is_some_and(|m| m.contains("元数据")) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "never showed the metadata stage: {message:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    seen.store(true, SeqCst);
    let task = finished(&app, &token, id).await;
    assert_eq!(task["state"], "done", "{task}");
}
