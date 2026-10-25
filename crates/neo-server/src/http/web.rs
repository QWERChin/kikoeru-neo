//! Serves the SvelteKit single-page app at the site root.
//!
//! The build in `web/build` is compiled into the binary only with the
//! `embed-web` feature, so backend work does not need a frontend build.
//! Without it every path outside the API router gets the JSON 404.

use std::{borrow::Cow, sync::Arc};

use axum::{
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use neo_core::dto::ErrorCode;
use tower_http::compression::{
    CompressionLayer,
    predicate::{NotForContentType, Predicate, SizeAbove},
};

use super::{AppState, HttpError};

/// Looks up a file of the frontend build by its path relative to the build
/// root, such as `index.html`. The desktop app passes the copy Tauri embeds
/// for its own pages, so it builds without `embed-web` and carries the build
/// once instead of twice. A missing file must be `None`: the routing rules
/// below decide which paths get `index.html`.
pub type WebSource = Arc<dyn Fn(&str) -> Option<Asset> + Send + Sync>;

/// One file of the frontend build.
pub struct Asset {
    pub body: Cow<'static, [u8]>,
    pub mime: String,
    /// A quoted entity tag derived from the file contents.
    pub etag: String,
}

/// SvelteKit puts a content hash into every file name under this prefix, so
/// a changed file always gets a new URL and browsers may keep these forever.
const IMMUTABLE_PREFIX: &str = "_app/immutable/";
const IMMUTABLE: &str = "public, max-age=31536000, immutable";
/// Everything else, `index.html` and a future service worker included, keeps
/// its URL across releases and must be revalidated on every load.
const REVALIDATE: &str = "no-cache";
const INDEX: &str = "index.html";

#[cfg(feature = "embed-web")]
mod embedded {
    use super::Asset;

    // Release builds embed the files. Debug builds read them from this
    // folder at run time, so `pnpm build` takes effect without recompiling.
    #[derive(rust_embed::Embed)]
    #[folder = "../../web/build"]
    struct Build;

    pub fn get(path: &str) -> Option<Asset> {
        let file = Build::get(path)?;
        let hash: String = file.metadata.sha256_hash()[..16]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        Some(Asset {
            mime: file.metadata.mimetype().to_owned(),
            etag: format!("\"{hash}\""),
            body: file.data,
        })
    }
}

#[cfg(feature = "embed-web")]
fn lookup(path: &str) -> Option<Asset> {
    embedded::get(path)
}

#[cfg(not(feature = "embed-web"))]
fn lookup(_path: &str) -> Option<Asset> {
    None
}

/// Gzip for the frontend's files when the client accepts it. A server on
/// the LAN is often reached over plain HTTP with no reverse proxy, and then
/// nothing else compresses them. Images other than SVG are skipped, and so
/// are fonts, because woff2 is compressed already. The size floor is the
/// API's, so the JSON 404 this fallback gives an unknown `/api` path is
/// treated like any other short API answer.
pub fn compression() -> CompressionLayer<impl Predicate> {
    CompressionLayer::new().compress_when(
        SizeAbove::new(super::MIN_COMPRESSED_BYTES)
            .and(NotForContentType::IMAGES)
            .and(NotForContentType::const_new("font/")),
    )
}

/// The router's fallback: anything no API route matched ends up here.
pub async fn fallback(State(state): State<AppState>, req: Request) -> Response {
    let (method, path, headers) = (req.method(), req.uri().path(), req.headers());
    match state.web() {
        Some(source) => respond(method, path, headers, |p| source(p)),
        None => respond(method, path, headers, lookup),
    }
}

fn respond(
    method: &Method,
    path: &str,
    headers: &HeaderMap,
    lookup: impl Fn(&str) -> Option<Asset>,
) -> Response {
    // An unknown API path must stay a JSON error. Answering it with the app
    // page would hand the client HTML where it expects an error body.
    if path == "/api" || path.starts_with("/api/") {
        return HttpError::not_found().into_response();
    }
    if method != Method::GET && method != Method::HEAD {
        return HttpError::new(
            StatusCode::METHOD_NOT_ALLOWED,
            ErrorCode::InvalidRequest,
            "页面只支持 GET 和 HEAD",
        )
        .into_response();
    }
    let rel = path.trim_start_matches('/');
    // Debug builds read assets from disk, so never let a dot segment reach
    // the lookup. Release builds only know the embedded names anyway.
    if rel.split('/').any(|s| s == "..") || rel.contains('\\') {
        return HttpError::not_found().into_response();
    }
    let found = if rel.is_empty() {
        lookup(INDEX).map(|a| (a, REVALIDATE))
    } else if let Some(asset) = lookup(rel) {
        let cache = if rel.starts_with(IMMUTABLE_PREFIX) {
            IMMUTABLE
        } else {
            REVALIDATE
        };
        Some((asset, cache))
    } else if is_page_path(rel) {
        lookup(INDEX).map(|a| (a, REVALIDATE))
    } else {
        None
    };
    match found {
        Some((asset, cache)) => asset_response(asset, cache, headers),
        None => HttpError::not_found().into_response(),
    }
}

/// A client-side route such as `/library` or `/works/12`. A path with a file
/// extension or under `_app/` asks for a file, and a missing file must be a
/// 404 so a stale chunk fails loudly instead of being parsed as HTML.
fn is_page_path(rel: &str) -> bool {
    let last = rel.rsplit('/').next().unwrap_or(rel);
    !rel.starts_with("_app/") && !last.contains('.')
}

fn asset_response(asset: Asset, cache: &'static str, headers: &HeaderMap) -> Response {
    let mut response = if etag_matches(headers, &asset.etag) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        let mut r = asset.body.into_owned().into_response();
        if let Ok(mime) = HeaderValue::from_str(&asset.mime) {
            r.headers_mut().insert(header::CONTENT_TYPE, mime);
        }
        r
    };
    let h = response.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    // Weak, because the same tag stands for the gzip and the identity form
    // that compression() may send, and a strong tag names one byte sequence.
    // If-None-Match compares weakly, so revalidation still gets a 304.
    if let Ok(etag) = HeaderValue::from_str(&format!("W/{}", asset.etag)) {
        h.insert(header::ETAG, etag);
    }
    // Set here so a 304, which compression() leaves alone, says it too.
    h.insert(header::VARY, HeaderValue::from_static("accept-encoding"));
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

pub(super) fn etag_matches(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|t| t.trim().trim_start_matches("W/"))
        .any(|t| t == "*" || t == etag)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(path: &str) -> Option<Asset> {
        let (body, mime): (&'static [u8], &str) = match path {
            "index.html" => (b"<!doctype html><html></html>", "text/html"),
            "favicon.svg" => (b"<svg/>", "image/svg+xml"),
            "_app/immutable/entry/app.abc123.js" => (b"export{}", "text/javascript"),
            "_app/version.json" => (b"{}", "application/json"),
            _ => return None,
        };
        Some(Asset {
            body: Cow::Borrowed(body),
            mime: mime.to_owned(),
            etag: format!("\"{}\"", path.len()),
        })
    }

    fn get(path: &str) -> Response {
        respond(&Method::GET, path, &HeaderMap::new(), fake)
    }

    fn header(r: &Response, name: header::HeaderName) -> &str {
        r.headers().get(name).unwrap().to_str().unwrap()
    }

    fn assert_json_404(r: &Response) {
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
        assert_eq!(header(r, header::CONTENT_TYPE), "application/json");
    }

    #[test]
    fn page_paths_fall_back_to_index() {
        for path in ["/", "/library", "/settings", "/library/", "/works/12"] {
            let r = get(path);
            assert_eq!(r.status(), StatusCode::OK, "{path}");
            assert_eq!(header(&r, header::CONTENT_TYPE), "text/html", "{path}");
            assert_eq!(header(&r, header::CACHE_CONTROL), REVALIDATE, "{path}");
        }
    }

    #[test]
    fn api_paths_never_get_the_page() {
        for path in ["/api", "/api/", "/api/nope", "/api/works/1/nope"] {
            assert_json_404(&get(path));
        }
    }

    #[test]
    fn missing_files_are_404() {
        for path in [
            "/_app/immutable/missing.js",
            "/_app/immutable/nodes",
            "/favicon-nope.png",
            "/library/cover.jpg",
            "/../library",
            "/library/../settings",
            "/library\\settings",
        ] {
            assert_json_404(&get(path));
        }
    }

    #[test]
    fn cache_policy_follows_the_file_name() {
        let r = get("/_app/immutable/entry/app.abc123.js");
        assert_eq!(header(&r, header::CONTENT_TYPE), "text/javascript");
        assert_eq!(header(&r, header::CACHE_CONTROL), IMMUTABLE);
        for path in ["/favicon.svg", "/_app/version.json", "/index.html"] {
            assert_eq!(header(&get(path), header::CACHE_CONTROL), REVALIDATE);
        }
        assert_eq!(
            header(&get("/favicon.svg"), header::X_CONTENT_TYPE_OPTIONS),
            "nosniff"
        );
    }

    #[test]
    fn matching_etag_gets_304() {
        let etag = header(&get("/favicon.svg"), header::ETAG).to_owned();
        assert!(etag.starts_with("W/\""), "{etag}");
        // The weak tag as received, and the strong form a client may send.
        for sent in [etag.clone(), etag.trim_start_matches("W/").to_owned()] {
            let mut headers = HeaderMap::new();
            headers.insert(
                header::IF_NONE_MATCH,
                format!("\"other\", {sent}").parse().unwrap(),
            );
            let r = respond(&Method::GET, "/favicon.svg", &headers, fake);
            assert_eq!(r.status(), StatusCode::NOT_MODIFIED, "{sent}");
            assert_eq!(header(&r, header::ETAG), etag);
            assert_eq!(header(&r, header::VARY), "accept-encoding");
        }
    }

    #[test]
    fn other_methods_are_rejected() {
        let r = respond(&Method::POST, "/library", &HeaderMap::new(), fake);
        assert_eq!(r.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_json_404(&respond(
            &Method::POST,
            "/api/nope",
            &HeaderMap::new(),
            fake,
        ));
    }

    #[test]
    fn without_a_build_everything_is_404() {
        for path in ["/", "/library", "/favicon.svg"] {
            assert_json_404(&respond(&Method::GET, path, &HeaderMap::new(), |_| None));
        }
    }
}
