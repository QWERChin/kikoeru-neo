#![allow(dead_code)]

use std::net::SocketAddr;

use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use http_body_util::BodyExt;
use neo_core::dto::{RootKind, RootSettings, RootSource};
use neo_db::{Db, users};
use neo_server::{
    datadir::DataDir,
    http::{AppState, RootEntry, router},
    password,
    token::{Audience, Tokens, now_secs},
};
use serde_json::Value;
use tower::ServiceExt;

pub const KEY: &[u8] = b"test signing key, 32 bytes long!";
pub const PASSWORD: &str = "correct horse battery";

pub struct App {
    dir: tempfile::TempDir,
    pub db: Db,
    pub state: AppState,
    pub router: Router,
}

pub async fn app() -> App {
    let app = app_not_ready().await;
    app.state.set_ready();
    app
}

pub async fn app_not_ready() -> App {
    app_with_free_space_floor(0, |s, _| s).await
}

/// A ready app with extra state, such as a metadata source. `configure`
/// gets the state and the data directory.
pub async fn app_with(configure: impl FnOnce(AppState, &std::path::Path) -> AppState) -> App {
    let app = app_with_free_space_floor(0, configure).await;
    app.state.set_ready();
    app
}

/// A ready app whose `cloud` root is served by `storage`.
pub async fn app_with_cloud(storage: std::sync::Arc<dyn neo_storage::Storage>) -> App {
    app_with_cloud_and(storage, |s, _| s).await
}

/// `app_with_cloud` with extra state, such as different media limits.
pub async fn app_with_cloud_and(
    storage: std::sync::Arc<dyn neo_storage::Storage>,
    configure: impl FnOnce(AppState, &std::path::Path) -> AppState,
) -> App {
    let app = build(0, Some(storage), configure).await;
    app.state.set_ready();
    app
}

/// A ready app whose transcode cache refuses new work below `floor` bytes
/// of free space. `u64::MAX` simulates a full disk.
pub async fn app_low_disk() -> App {
    let app = app_with_free_space_floor(u64::MAX, |s, _| s).await;
    app.state.set_ready();
    app
}

async fn app_with_free_space_floor(
    floor: u64,
    configure: impl FnOnce(AppState, &std::path::Path) -> AppState,
) -> App {
    build(floor, None, configure).await
}

async fn build(
    floor: u64,
    cloud: Option<std::sync::Arc<dyn neo_storage::Storage>>,
    configure: impl FnOnce(AppState, &std::path::Path) -> AppState,
) -> App {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = DataDir::prepare(dir.path()).unwrap();
    let db = Db::open_sqlite(&data_dir.database()).await.unwrap();
    let state = AppState::new(
        db.clone(),
        Tokens::new(KEY),
        vec!["127.0.0.1".parse().unwrap()],
    )
    .with_data_dir(data_dir)
    .with_roots(vec![
        RootEntry {
            id: "lib".into(),
            kind: RootKind::Local,
            source: RootSource::Config,
            settings: RootSettings::Local {
                path: dir.path().join("library").display().to_string(),
                watch: false,
                auto_scan_interval: None,
            },
        },
        RootEntry {
            id: "cloud".into(),
            kind: RootKind::OpenList,
            source: RootSource::Config,
            settings: RootSettings::OpenList {
                url: "http://openlist.test".into(),
                base_path: "/cloud".into(),
                username: "test".into(),
                password_set: true,
                list_concurrency: None,
                link_rate: None,
                auto_scan_interval: None,
            },
        },
    ])
    .with_storages(
        [(
            "lib".to_owned(),
            std::sync::Arc::new(neo_storage::LocalStorage::new(dir.path().join("library"))) as _,
        )]
        .into_iter()
        .chain(cloud.map(|storage| ("cloud".to_owned(), storage)))
        .collect(),
    );
    std::fs::create_dir_all(dir.path().join("library")).unwrap();
    let state = state
        .with_media(neo_server::http::Media {
            cache: neo_media::TranscodeCache::open(&dir.path().join("cache"), u64::MAX, floor)
                .unwrap(),
            probe_slots: std::sync::Arc::new(tokio::sync::Semaphore::new(4)),
            queue_limit: 32,
            job_timeout: std::time::Duration::from_secs(60),
            remote_analysis_delay: std::time::Duration::from_millis(500),
            video_transcode: true,
        })
        // Short, so a test can watch a stalled download give up its slot.
        .with_transfer_stall(std::time::Duration::from_millis(300));
    let state = configure(state, dir.path());
    App {
        dir,
        router: router(state.clone()),
        db,
        state,
    }
}

impl App {
    pub fn data_dir(&self) -> &std::path::Path {
        self.dir.path()
    }

    /// The directory behind the local root `lib`.
    pub fn library(&self) -> std::path::PathBuf {
        self.dir.path().join("library")
    }

    pub fn media_token(&self, user_id: i64, token_version: i64) -> String {
        Tokens::new(KEY)
            .issue(user_id, token_version, Audience::Media, now_secs())
            .unwrap()
            .token
    }

    /// For responses that are not JSON, such as images.
    pub async fn send_raw(&self, req: Req) -> (StatusCode, HeaderMap, Vec<u8>) {
        let mut builder = Request::builder().method(req.method).uri(&req.uri);
        if let Some(token) = &req.token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        for (name, value) in &req.headers {
            builder = builder.header(name, value);
        }
        let mut request = builder.body(Body::empty()).unwrap();
        request.extensions_mut().insert(ConnectInfo(req.peer));
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, headers, bytes.to_vec())
    }

    pub async fn user(&self, name: &str, admin: bool) -> i64 {
        let hash = password::hash(PASSWORD).unwrap();
        users::create(
            &self.db,
            name,
            &hash,
            admin,
            neo_server::token::initial_version().unwrap(),
            0,
        )
        .await
        .unwrap()
    }

    pub async fn exec(&self, sql: &str) {
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql.to_owned()))
            .execute(&self.db.sqlite().unwrap().write)
            .await
            .unwrap();
    }

    pub async fn login(&self, name: &str) -> String {
        let (status, body) = self
            .send(Req::post("/api/auth/login").json(serde_json::json!({
                "name": name, "password": PASSWORD
            })))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["token"].as_str().unwrap().to_owned()
    }

    pub async fn send(&self, req: Req) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(req.method).uri(&req.uri);
        if let Some(token) = &req.token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        for (name, value) in &req.headers {
            builder = builder.header(name, value);
        }
        let body = match &req.body {
            Some(text) => {
                builder = builder.header(header::CONTENT_TYPE, "application/json");
                Body::from(text.clone())
            }
            None => Body::empty(),
        };
        let mut request = builder.body(body).unwrap();
        request.extensions_mut().insert(ConnectInfo(req.peer));
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|e| panic!("{status}: body is not JSON ({e}): {bytes:?}"))
        };
        (status, value)
    }
}

pub struct Req {
    method: Method,
    uri: String,
    token: Option<String>,
    body: Option<String>,
    peer: SocketAddr,
    headers: Vec<(String, String)>,
}

impl Req {
    fn new(method: Method, uri: &str) -> Self {
        Self {
            method,
            uri: uri.to_owned(),
            token: None,
            body: None,
            peer: "192.0.2.10:40000".parse().unwrap(),
            headers: vec![],
        }
    }

    pub fn get(uri: &str) -> Self {
        Self::new(Method::GET, uri)
    }

    pub fn post(uri: &str) -> Self {
        Self::new(Method::POST, uri)
    }

    pub fn head(uri: &str) -> Self {
        Self::new(Method::HEAD, uri)
    }

    pub fn delete(uri: &str) -> Self {
        Self::new(Method::DELETE, uri)
    }

    pub fn put(uri: &str) -> Self {
        Self::new(Method::PUT, uri)
    }

    pub fn token(mut self, token: &str) -> Self {
        self.token = Some(token.to_owned());
        self
    }

    pub fn json(mut self, body: Value) -> Self {
        self.body = Some(body.to_string());
        self
    }

    pub fn raw_body(mut self, body: &str) -> Self {
        self.body = Some(body.to_owned());
        self
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    pub fn peer(mut self, peer: &str) -> Self {
        self.peer = peer.parse().unwrap();
        self
    }
}

pub fn sample(name: &str) -> Value {
    sample_in("m1", name)
}

pub fn sample_in(dir: &str, name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../contract")
        .join(dir)
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Asserts that `actual` has exactly the fields of `sample`, recursively,
/// with the same JSON types. A null sample does not specify a value's type.
pub fn assert_same_shape(actual: &Value, sample: &Value, path: &str) {
    assert_same_shape_with_nullable(actual, sample, path, &[]);
}

/// Allows null at the contract's nullable paths even when the sample has a
/// value. Paths are relative to the root; `[]` denotes any array element.
pub fn assert_same_shape_with_nullable(
    actual: &Value,
    sample: &Value,
    path: &str,
    nullable: &[&str],
) {
    check_shape(actual, sample, path, "", nullable);
}

fn check_shape(actual: &Value, sample: &Value, path: &str, field: &str, nullable: &[&str]) {
    if actual.is_null() && nullable.contains(&field) {
        return;
    }
    match (actual, sample) {
        (_, Value::Null) => {}
        (Value::Object(a), Value::Object(s)) => {
            let mut ak: Vec<_> = a.keys().collect();
            let mut sk: Vec<_> = s.keys().collect();
            ak.sort();
            sk.sort();
            assert_eq!(ak, sk, "fields of {path}");
            for (k, v) in s {
                let child = if field.is_empty() {
                    k.clone()
                } else {
                    format!("{field}.{k}")
                };
                check_shape(&a[k], v, &format!("{path}.{k}"), &child, nullable);
            }
        }
        (Value::Array(a), Value::Array(s)) => {
            if let Some(first) = s.iter().find(|v| !v.is_null()) {
                for (i, v) in a.iter().enumerate() {
                    check_shape(
                        v,
                        first,
                        &format!("{path}[{i}]"),
                        &format!("{field}[]"),
                        nullable,
                    );
                }
            }
        }
        (Value::Number(_), Value::Number(_))
        | (Value::String(_), Value::String(_))
        | (Value::Bool(_), Value::Bool(_)) => {}
        _ => panic!("{path}: {actual} does not have the type of {sample}"),
    }
}

// WorkSummary and WorkDetail nullability from book/src/contract.md. The
// first sample item has values even where other works may have null.
pub const WORK_PAGE_NULLABLE: &[&str] = &[
    "items[].circle",
    "items[].release_date",
    "items[].rate_average",
    "items[].rate_count",
    "items[].progress",
    "items[].progress.track_id",
    "items[].progress.track_duration_ms",
    "items[].progress.group_path",
    "items[].my_rating",
];

pub const WORK_DETAIL_NULLABLE: &[&str] = &[
    "circle",
    "release_date",
    "rate_average",
    "rate_count",
    "progress",
    "progress.track_id",
    "progress.track_duration_ms",
    "progress.group_path",
    "my_rating",
    "my_review",
    "my_review.rating",
    "my_review.text",
    "my_review.updated_at",
    "total_duration_ms",
    "locations",
    "tracks[].size",
    "tracks[].duration_ms",
    "tracks[].video",
    "tracks[].video.width",
    "tracks[].video.height",
    "tracks[].loudness",
    "tracks[].loudness.integrated_lufs",
    "tracks[].loudness.true_peak_dbtp",
];

pub fn assert_error(status: StatusCode, body: &Value, want_status: StatusCode, code: &str) {
    assert_eq!(status, want_status, "{body}");
    assert_same_shape(body, &sample("error-unauthorized.json"), "error");
    assert_eq!(body["error"]["code"], code, "{body}");
    assert!(!body["error"]["message"].as_str().unwrap().is_empty());
}
