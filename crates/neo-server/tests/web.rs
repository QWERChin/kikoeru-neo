//! Checks the real frontend build compiled in with `embed-web`. The routing
//! rules themselves are unit-tested in src/http/web.rs.
#![cfg(feature = "embed-web")]

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
    response::Response,
};
use common::app;
use http_body_util::BodyExt;
use tower::ServiceExt;

async fn get(app: &common::App, uri: &str) -> (Response, String) {
    let req = Request::get(uri).body(Body::empty()).unwrap();
    let response = app.router.clone().oneshot(req).await.unwrap();
    let (parts, body) = response.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    (Response::from_parts(parts, Body::empty()), text)
}

fn header(r: &Response, name: header::HeaderName) -> &str {
    r.headers().get(name).map_or("", |v| v.to_str().unwrap())
}

#[tokio::test]
async fn deep_links_get_the_app_page() {
    let app = app().await;
    for uri in ["/", "/library", "/settings", "/login", "/library?page=2"] {
        let (r, body) = get(&app, uri).await;
        assert_eq!(r.status(), StatusCode::OK, "{uri}");
        assert!(
            header(&r, header::CONTENT_TYPE).starts_with("text/html"),
            "{uri}"
        );
        assert_eq!(header(&r, header::CACHE_CONTROL), "no-cache", "{uri}");
        assert!(body.contains("/_app/immutable/"), "{uri}: {body}");
    }
}

#[tokio::test]
async fn hashed_assets_are_cached_for_good() {
    let app = app().await;
    let (_, index) = get(&app, "/").await;
    // index.html links its entry chunks with paths relative to the page.
    let start = index.find("_app/immutable/").expect("index links an asset");
    let path: String = index[start..]
        .chars()
        .take_while(|c| !matches!(c, '"' | '\'' | ')' | ' '))
        .collect();
    let (r, _) = get(&app, &format!("/{path}")).await;
    assert_eq!(r.status(), StatusCode::OK, "{path}");
    assert_eq!(
        header(&r, header::CACHE_CONTROL),
        "public, max-age=31536000, immutable"
    );
    assert!(!header(&r, header::CONTENT_TYPE).starts_with("text/html"));

    let (r, body) = get(&app, "/favicon.svg").await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(header(&r, header::CONTENT_TYPE), "image/svg+xml");
    assert!(body.contains("<svg"));
}

#[tokio::test]
async fn missing_files_and_api_paths_are_json_404() {
    let app = app().await;
    for uri in [
        "/api/nope",
        "/api",
        "/_app/immutable/missing.js",
        "/favicon-nope.png",
    ] {
        let (r, body) = get(&app, uri).await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND, "{uri}");
        assert_eq!(
            header(&r, header::CONTENT_TYPE),
            "application/json",
            "{uri}"
        );
        assert!(body.contains("\"not_found\""), "{uri}: {body}");
    }
}

#[tokio::test]
async fn the_page_carries_its_own_content_security_policy() {
    let app = app().await;
    let (r, body) = get(&app, "/library").await;
    assert_eq!(r.status(), StatusCode::OK);
    let meta = body
        .split("<meta http-equiv=\"content-security-policy\" content=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("the build writes the policy into the page");
    assert!(meta.contains("default-src 'self'"), "{meta}");
    assert!(
        meta.contains("'sha256-"),
        "SvelteKit's bootstrap is hashed: {meta}"
    );
    assert!(meta.contains("'wasm-unsafe-eval'"), "JASSUB: {meta}");
    assert!(!meta.contains("'unsafe-eval'"), "{meta}");
    // The header adds what a meta tag cannot carry, and nothing that would
    // narrow the page's policy.
    assert_eq!(
        header(&r, header::CONTENT_SECURITY_POLICY),
        "frame-ancestors 'none'"
    );
}

async fn get_with(
    app: &common::App,
    uri: &str,
    headers: &[(header::HeaderName, &str)],
) -> (Response, Vec<u8>) {
    let mut req = Request::get(uri);
    for (name, value) in headers {
        req = req.header(name, *value);
    }
    let response = app
        .router
        .clone()
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let (parts, body) = response.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes().to_vec();
    (Response::from_parts(parts, Body::empty()), bytes)
}

/// A file of the build under `_app/immutable/assets/` with this extension.
fn built_asset(extension: &str) -> String {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../web/build/_app/immutable/assets");
    let name = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .find(|n| n.ends_with(extension))
        .unwrap_or_else(|| panic!("no {extension} in {}", dir.display()));
    format!("/_app/immutable/assets/{name}")
}

#[tokio::test]
async fn scripts_and_wasm_are_gzipped_for_clients_that_accept_it() {
    let app = app().await;
    let gzip = [(header::ACCEPT_ENCODING, "gzip, deflate")];
    for uri in ["/", "/library", &built_asset(".wasm")] {
        let (plain, plain_body) = get_with(&app, uri, &[]).await;
        assert_eq!(header(&plain, header::CONTENT_ENCODING), "", "{uri}");
        let (r, body) = get_with(&app, uri, &gzip).await;
        assert_eq!(r.status(), StatusCode::OK, "{uri}");
        assert_eq!(header(&r, header::CONTENT_ENCODING), "gzip", "{uri}");
        assert_eq!(header(&r, header::VARY), "accept-encoding", "{uri}");
        assert!(body.len() < plain_body.len(), "{uri}");
        let mut unpacked = Vec::new();
        std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(&body[..]), &mut unpacked)
            .unwrap();
        assert_eq!(unpacked, plain_body, "{uri}");

        // Both forms share one weak tag, and either revalidates.
        let etag = header(&r, header::ETAG).to_owned();
        assert!(etag.starts_with("W/\""), "{uri}: {etag}");
        assert_eq!(header(&plain, header::ETAG), etag, "{uri}");
        let (again, _) = get_with(
            &app,
            uri,
            &[gzip[0].clone(), (header::IF_NONE_MATCH, &etag)],
        )
        .await;
        assert_eq!(again.status(), StatusCode::NOT_MODIFIED, "{uri}");
    }
}

#[tokio::test]
async fn fonts_and_short_api_answers_are_not_gzipped() {
    let app = app().await;
    let gzip = [(header::ACCEPT_ENCODING, "gzip")];
    let (r, _) = get_with(&app, &built_asset(".woff2"), &gzip).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(header(&r, header::CONTENT_ENCODING), "");
    let (r, _) = get_with(&app, "/api/version", &gzip).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(header(&r, header::CONTENT_ENCODING), "");
}
