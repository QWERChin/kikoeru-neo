//! The server settings (book/src/contract.md, "服务器设置"): saving the
//! metadata proxy, applying it without a restart, and checking a proxy.
//! DLsite is an `.invalid` host that never resolves, so metadata only
//! arrives through the fake proxy. Nothing here touches the network.

mod common;

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{Json, Router, http::StatusCode, routing::get};
use common::*;
use neo_scanner::metadata::{
    CoverDirs, DlsiteConfig, DlsiteProvider, MetadataError, MetadataProvider, ProxyChoice,
    WorkMetadata,
};
use neo_server::scan::MetadataEnv;
use serde_json::{Value, json};
use tokio::sync::Semaphore;

/// Answers as DLsite. An HTTP proxy request carries the full URL, which
/// axum routes by its path, so this is also the proxy.
async fn fake_proxy() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/", get(|| async { StatusCode::FORBIDDEN }))
        .route(
            "/maniax/api/=/product.json",
            get(|| async {
                Json(json!([{"product_name": "Through the proxy", "age_category": 1}]))
            }),
        )
        .route(
            "/maniax/product/info/ajax",
            get(|| async { Json(json!({})) }),
        );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    address
}

async fn setup(metadata: bool) -> (App, String) {
    let app = app_with(move |state, dir| {
        if !metadata {
            return state;
        }
        let provider = DlsiteProvider::new(DlsiteConfig {
            base: "http://dlsite.invalid/".parse().unwrap(),
            spacing: Duration::ZERO,
            retries: 0,
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
    (app, token)
}

async fn save(app: &App, token: &str, proxy: Value) -> (StatusCode, Value) {
    app.send(
        Req::put("/api/admin/settings")
            .token(token)
            .json(json!({"metadata_proxy": proxy})),
    )
    .await
}

async fn saved(app: &App, token: &str) -> Value {
    let (status, body) = app.send(Req::get("/api/admin/settings").token(token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

async fn check(app: &App, token: &str, proxy: Value) -> (StatusCode, Value) {
    app.send(
        Req::post("/api/admin/settings/proxy-check")
            .token(token)
            .json(json!({"metadata_proxy": proxy})),
    )
    .await
}

/// Refreshes work 1 and returns its title once the task has ended.
async fn refreshed_title(app: &App, token: &str) -> String {
    let (status, body) = app
        .send(Req::post("/api/works/1/refresh").token(token))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let id = body["task_id"].as_i64().unwrap();
    for _ in 0..200 {
        let (_, task) = app
            .send(Req::get(&format!("/api/tasks/{id}")).token(token))
            .await;
        if !matches!(task["state"].as_str(), Some("queued" | "running")) {
            let (_, detail) = app.send(Req::get("/api/works/1").token(token)).await;
            return detail["title"].as_str().unwrap().to_owned();
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the refresh did not end");
}

#[tokio::test]
async fn only_admins_see_or_change_the_settings() {
    let (app, _) = setup(true).await;
    app.user("listener", false).await;
    let token = app.login("listener").await;

    for req in [
        Req::get("/api/admin/settings"),
        Req::put("/api/admin/settings").json(json!({"metadata_proxy": null})),
        Req::post("/api/admin/settings/proxy-check").json(json!({"metadata_proxy": null})),
    ] {
        let (status, body) = app.send(req.token(&token)).await;
        assert_error(status, &body, StatusCode::FORBIDDEN, "forbidden");
    }
}

#[tokio::test]
async fn saves_the_proxy_in_its_normalised_form() {
    let (app, token) = setup(true).await;
    assert_eq!(saved(&app, &token).await, json!({"metadata_proxy": null}));

    let (status, body) = save(&app, &token, json!(" HTTP://LocalHost:7890/ ")).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_same_shape(
        &body,
        &sample_in("settings", "server-settings.json"),
        "settings",
    );
    assert_eq!(body, json!({"metadata_proxy": "http://localhost:7890"}));
    assert_eq!(saved(&app, &token).await, body);

    // An empty value and a body without the field both clear it.
    let (status, body) = save(&app, &token, json!("  ")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({"metadata_proxy": null}));
    save(&app, &token, json!("socks5h://127.0.0.1:1080")).await;
    let (status, body) = app
        .send(
            Req::put("/api/admin/settings")
                .token(&token)
                .json(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(saved(&app, &token).await, json!({"metadata_proxy": null}));
}

#[tokio::test]
async fn refuses_a_proxy_the_contract_rules_out_and_keeps_the_old_one() {
    let (app, token) = setup(true).await;
    save(&app, &token, json!("http://127.0.0.1:7890")).await;

    for proxy in [
        "127.0.0.1:7890",
        "ftp://127.0.0.1:21",
        "http://user:secret@127.0.0.1:7890",
        "http://127.0.0.1:7890/path",
    ] {
        let (status, body) = save(&app, &token, json!(proxy)).await;
        assert_error(status, &body, StatusCode::BAD_REQUEST, "invalid_request");
        let (status, body) = check(&app, &token, json!(proxy)).await;
        assert_error(status, &body, StatusCode::BAD_REQUEST, "invalid_request");
    }

    assert_eq!(
        saved(&app, &token).await,
        json!({"metadata_proxy": "http://127.0.0.1:7890"})
    );
}

#[tokio::test]
async fn a_saved_proxy_applies_to_the_next_fetch_without_a_restart() {
    let (app, token) = setup(true).await;
    let proxy = fake_proxy().await;
    app.exec(
        "INSERT INTO works (id, code, title, metadata_src, created_at, updated_at)
         VALUES (1, 'RJ01000001', 'folder name', 'none', 0, 0)",
    )
    .await;
    assert_eq!(refreshed_title(&app, &token).await, "folder name");

    let (status, body) = save(&app, &token, json!(proxy)).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    assert_eq!(refreshed_title(&app, &token).await, "Through the proxy");
}

#[tokio::test]
async fn checks_a_proxy_without_saving_it() {
    let (app, token) = setup(true).await;
    let proxy = fake_proxy().await;

    let (status, body) = check(&app, &token, json!(proxy)).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_same_shape(&body, &sample_in("settings", "proxy-check.json"), "check");
    assert_eq!(body["url"], "http://dlsite.invalid/");
    // Any answer counts as reaching DLsite, a refusal included.
    assert_eq!(body["status"], 403);
    assert_eq!(saved(&app, &token).await, json!({"metadata_proxy": null}));
}

#[tokio::test]
async fn a_check_that_gets_no_answer_says_why() {
    let (app, token) = setup(true).await;
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = closed.local_addr().unwrap();
    drop(closed);

    let (status, body) = check(&app, &token, json!(format!("http://{address}"))).await;

    assert_error(status, &body, StatusCode::BAD_GATEWAY, "upstream_error");
    let message = body["error"]["message"].as_str().unwrap();
    assert!(message.starts_with("连不上 DLsite："), "{message}");
}

#[tokio::test]
async fn without_a_metadata_source_a_check_has_nothing_to_reach() {
    let (app, token) = setup(false).await;

    let (status, body) = check(&app, &token, json!(null)).await;

    assert_error(
        status,
        &body,
        StatusCode::SERVICE_UNAVAILABLE,
        "upstream_error",
    );
    // Saving still works, for when a source is configured later.
    let (status, body) = save(&app, &token, json!("http://127.0.0.1:7890")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// A source whose switch to a new proxy waits for a permit, as asmr.one's
/// waits for a mirror probe that holds its lock.
struct SlowSwitch {
    permits: Semaphore,
    switched_to: Mutex<Option<ProxyChoice>>,
}

#[async_trait::async_trait]
impl MetadataProvider for SlowSwitch {
    async fn fetch(&self, _code: &str) -> Result<WorkMetadata, MetadataError> {
        Err(MetadataError::NotFound)
    }

    async fn cover(&self, _url: &url::Url) -> Result<Vec<u8>, MetadataError> {
        Err(MetadataError::NotFound)
    }

    async fn set_proxy(&self, proxy: &ProxyChoice) -> Result<(), MetadataError> {
        self.permits.acquire().await.unwrap().forget();
        *self.switched_to.lock().unwrap() = Some(proxy.clone());
        Ok(())
    }
}

#[tokio::test]
async fn a_save_finishes_switching_after_the_client_goes_away() {
    let source = Arc::new(SlowSwitch {
        permits: Semaphore::new(0),
        switched_to: Mutex::new(None),
    });
    let provider = source.clone();
    let app = app_with(move |state, dir| {
        state.with_metadata(MetadataEnv {
            provider,
            dirs: CoverDirs::new(dir.join("covers"), dir.join("cache/tmp")),
        })
    })
    .await;
    app.user("admin", true).await;
    let token = app.login("admin").await;

    // The client gives up while the switch waits, which drops the request.
    let gave_up = tokio::time::timeout(
        Duration::from_millis(200),
        save(&app, &token, json!("http://127.0.0.1:7890")),
    )
    .await;
    assert!(gave_up.is_err(), "the save did not wait for the switch");
    source.permits.add_permits(1);

    for _ in 0..200 {
        if source.switched_to.lock().unwrap().is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let switched = source.switched_to.lock().unwrap().clone();
    let Some(ProxyChoice::Fixed(url)) = switched else {
        panic!("the clients stayed on the old proxy: {switched:?}");
    };
    assert_eq!(url.as_str(), "http://127.0.0.1:7890");
    assert_eq!(
        saved(&app, &token).await,
        json!({"metadata_proxy": "http://127.0.0.1:7890"})
    );
}
