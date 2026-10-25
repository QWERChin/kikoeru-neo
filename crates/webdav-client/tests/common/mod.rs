//! A fake WebDAV server for the tests here and in neo-storage: an
//! in-memory tree under a configurable prefix, Basic auth, `PROPFIND` with
//! `Depth: 0` and `1`, ranged `GET`, and faults to inject per path.

#![allow(dead_code)]

use std::{
    collections::{BTreeMap, HashMap},
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, UNIX_EPOCH},
};

use axum::{
    Router,
    body::Body,
    extract::State,
    http::{HeaderMap, Method, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use webdav_client::{Client, Config};

pub const USER: &str = "reader";
pub const PASSWORD: &str = "secret";
/// The default prefix for the fake server.
pub const PREFIX: &str = "/dav";

#[derive(Clone)]
pub enum Node {
    Dir { etag: String },
    File { bytes: Vec<u8>, etag: String },
}

/// What the next requests for a path get instead of the normal answer.
#[derive(Clone)]
pub enum Fault {
    /// This status, `times` times, with an optional Retry-After.
    Status {
        status: u16,
        times: usize,
        retry_after: Option<u64>,
    },
    /// A redirect to `location`, every time.
    Redirect { location: String },
    /// A 207 whose body is this text, every time.
    Body(String),
}

#[derive(Default)]
pub struct Fake {
    prefix: &'static str,
    /// Decoded paths below the prefix, such as `/音声/RJ01`. The root is `""`.
    tree: Mutex<BTreeMap<String, Node>>,
    faults: Mutex<HashMap<String, Fault>>,
    /// Every request as "METHOD decoded-path depth".
    pub log: Mutex<Vec<String>>,
}

impl Fake {
    pub fn dir(&self, path: &str) {
        self.tree.lock().unwrap().insert(
            path.to_owned(),
            Node::Dir {
                etag: format!("\"d{}\"", path.len()),
            },
        );
    }

    pub fn file(&self, path: &str, bytes: &[u8]) {
        self.tree.lock().unwrap().insert(
            path.to_owned(),
            Node::File {
                bytes: bytes.to_vec(),
                etag: format!("\"f{}\"", bytes.len()),
            },
        );
    }

    pub fn remove(&self, path: &str) {
        self.tree
            .lock()
            .unwrap()
            .retain(|p, _| p != path && !p.starts_with(&format!("{path}/")));
    }

    pub fn set_etag(&self, path: &str, etag: &str) {
        let mut tree = self.tree.lock().unwrap();
        match tree.get_mut(path) {
            Some(Node::Dir { etag: e } | Node::File { etag: e, .. }) => *e = etag.to_owned(),
            None => panic!("no {path}"),
        }
    }

    pub fn fault(&self, path: &str, fault: Fault) {
        self.faults.lock().unwrap().insert(path.to_owned(), fault);
    }

    pub fn requests(&self, method: &str) -> usize {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.starts_with(method))
            .count()
    }
}

pub struct Server {
    pub fake: Arc<Fake>,
    pub addr: SocketAddr,
}

impl Server {
    pub fn url(&self) -> String {
        format!("http://{}{}", self.addr, self.fake.prefix)
    }

    pub fn config(&self) -> Config {
        let mut config = Config::new(self.url().parse().unwrap(), USER, PASSWORD);
        config.retry_delay = Duration::from_millis(5);
        config.cooldown_budget = Duration::from_secs(5);
        config
    }

    pub fn client(&self) -> Client {
        Client::new(self.config()).unwrap()
    }
}

pub async fn start() -> Server {
    start_at(PREFIX).await
}

pub async fn start_at(prefix: &'static str) -> Server {
    let fake = Arc::new(Fake {
        prefix,
        ..Fake::default()
    });
    fake.dir("");
    let app = Router::new().fallback(handle).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Server { fake, addr }
}

fn decode(path: &str) -> String {
    percent_encoding::percent_decode_str(path)
        .decode_utf8_lossy()
        .into_owned()
}

fn encode(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            segment
                .bytes()
                .map(|b| {
                    if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                        (b as char).to_string()
                    } else {
                        format!("%{b:02X}")
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/")
}

async fn handle(
    State(fake): State<Arc<Fake>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let raw = uri.path();
    let Some(below) = raw.strip_prefix(fake.prefix) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let path = decode(below.trim_end_matches('/'));
    let depth = headers
        .get("depth")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("-")
        .to_owned();
    fake.log
        .lock()
        .unwrap()
        .push(format!("{method} {path} {depth}"));
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    // "reader:secret" in base64.
    if auth != Some("Basic cmVhZGVyOnNlY3JldA==") {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if let Some(response) = fault(&fake, &path) {
        return response;
    }
    match method.as_str() {
        "PROPFIND" => propfind(&fake, &path, &depth),
        "GET" => get(&fake, &path, &headers),
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

fn fault(fake: &Fake, path: &str) -> Option<Response> {
    let mut faults = fake.faults.lock().unwrap();
    let fault = faults.get_mut(path)?;
    match fault {
        Fault::Status {
            status,
            times,
            retry_after,
        } => {
            if *times == 0 {
                return None;
            }
            *times -= 1;
            let mut response = StatusCode::from_u16(*status).unwrap().into_response();
            if let Some(seconds) = retry_after {
                response
                    .headers_mut()
                    .insert(header::RETRY_AFTER, seconds.to_string().parse().unwrap());
            }
            Some(response)
        }
        Fault::Redirect { location } => Some(
            (
                StatusCode::MOVED_PERMANENTLY,
                [(header::LOCATION, location.clone())],
            )
                .into_response(),
        ),
        Fault::Body(text) => Some(
            Response::builder()
                .status(207)
                .body(Body::from(text.clone()))
                .unwrap(),
        ),
    }
}

fn propfind(fake: &Fake, path: &str, depth: &str) -> Response {
    let tree = fake.tree.lock().unwrap();
    let Some(node) = tree.get(path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut body =
        String::from(r#"<?xml version="1.0" encoding="utf-8"?><D:multistatus xmlns:D="DAV:">"#);
    body.push_str(&response(fake.prefix, path, node));
    if depth == "1" && matches!(node, Node::Dir { .. }) {
        let prefix = format!("{path}/");
        for (child, node) in tree.iter() {
            let Some(name) = child.strip_prefix(&prefix) else {
                continue;
            };
            if !name.is_empty() && !name.contains('/') {
                body.push_str(&response(fake.prefix, child, node));
            }
        }
    }
    body.push_str("</D:multistatus>");
    Response::builder()
        .status(207)
        .header(header::CONTENT_TYPE, "application/xml; charset=utf-8")
        .body(Body::from(body))
        .unwrap()
}

fn response(prefix: &str, path: &str, node: &Node) -> String {
    let modified = httpdate::fmt_http_date(UNIX_EPOCH + Duration::from_secs(1_791_700_000));
    let (href, props) = match node {
        Node::Dir { etag } => (
            format!("{prefix}{}/", encode(path)),
            format!(
                "<D:resourcetype><D:collection/></D:resourcetype><D:getetag>{etag}</D:getetag><D:getlastmodified>{modified}</D:getlastmodified>"
            ),
        ),
        Node::File { bytes, etag } => (
            format!("{prefix}{}", encode(path)),
            format!(
                "<D:resourcetype/><D:getcontentlength>{}</D:getcontentlength><D:getetag>{etag}</D:getetag><D:getlastmodified>{modified}</D:getlastmodified>",
                bytes.len()
            ),
        ),
    };
    format!(
        "<D:response><D:href>{}</D:href><D:propstat><D:prop>{props}</D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>",
        href.replace('&', "&amp;")
    )
}

fn get(fake: &Fake, path: &str, headers: &HeaderMap) -> Response {
    let tree = fake.tree.lock().unwrap();
    let Some(Node::File { bytes, .. }) = tree.get(path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("bytes="))
        .and_then(|v| v.split_once('-'));
    let Some((start, end)) = range else {
        return (
            StatusCode::OK,
            [(header::CONTENT_LENGTH, bytes.len().to_string())],
            bytes.clone(),
        )
            .into_response();
    };
    let start: usize = start.parse().unwrap_or(0);
    if start >= bytes.len() {
        return (
            StatusCode::RANGE_NOT_SATISFIABLE,
            [(header::CONTENT_RANGE, format!("bytes */{}", bytes.len()))],
        )
            .into_response();
    }
    let end = end
        .parse::<usize>()
        .map_or(bytes.len() - 1, |e| e.min(bytes.len() - 1));
    (
        StatusCode::PARTIAL_CONTENT,
        [(
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{}", bytes.len()),
        )],
        bytes[start..=end].to_vec(),
    )
        .into_response()
}
