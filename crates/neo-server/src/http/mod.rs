//! The axum application: the endpoints of book/src/contract.md.

mod access;
mod admin;
mod analysis;
mod archive;
mod auth;
mod covers;
mod edit;
mod error;
mod extract;
mod library;
pub mod limiter;
mod media;
mod metadata;
mod playback;
mod progress;
mod roots;
mod server_settings;
mod system;
mod tasks;
mod video;
mod web;
mod works;

pub use web::{Asset as WebAsset, WebSource};

use std::{
    net::IpAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    extract::{Request, State},
    handler::Handler,
    http::{Extensions, HeaderMap, HeaderValue, StatusCode, Uri, Version, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use futures_util::FutureExt;
use neo_core::dto::{ErrorCode, RootKind, RootSettings, RootSource};
use neo_db::Db;
use neo_storage::Storage;
use tokio::sync::Semaphore;
use tower_http::compression::{CompressionLayer, Predicate, predicate::SizeAbove};

pub use error::HttpError;
pub use limiter::LoginLimiter;
pub use metadata::rebuild_metadata_task;
pub use playback::{Media, probe_job, probe_spec, rebuild_task};

/// Queues a background ffprobe for a track a scan found new or changed.
///
/// The dedupe key includes the source version, size and mtime. A probe still running
/// for the previous version of the file would have its result rejected, so
/// the new version needs its own task rather than being folded into that one.
pub fn probe_queue(state: &AppState) -> crate::scan::ProbeQueue {
    let state = state.clone();
    Arc::new(move |id: i64| {
        let state = state.clone();
        async move {
            let version = neo_db::media::track_version(state.db(), id).await;
            let mut spec = probe_spec(id);
            match version {
                Ok(Some((source_version, size, mtime))) => {
                    let size = size.unwrap_or(-1);
                    let mtime = mtime.unwrap_or(-1);
                    spec.dedupe_key = Some(format!("probe:{id}:{source_version}:{size}:{mtime}"));
                }
                Ok(None) => return,
                Err(error) => {
                    tracing::warn!(track = id, %error, "cannot queue a probe");
                    return;
                }
            }
            let submitted = state
                .scheduler()
                .submit(spec, probe_job(state.clone(), id))
                .await;
            if let Err(error) = submitted {
                tracing::warn!(track = id, %error, "cannot queue a probe");
            }
        }
        .boxed()
    })
}

use crate::{datadir::DataDir, tasks::Scheduler, token::Tokens};

#[derive(Default)]
struct Registry {
    roots: Vec<RootEntry>,
    storages: Vec<(String, Arc<dyn Storage>)>,
}

/// A root as the API reports it: where it is defined and how it connects.
#[derive(Debug, Clone)]
pub struct RootEntry {
    pub id: String,
    pub kind: RootKind,
    pub source: RootSource,
    pub settings: RootSettings,
}

#[derive(Clone)]
pub struct AppState(Arc<Inner>);

struct Inner {
    db: Db,
    tokens: Tokens,
    ready: AtomicBool,
    limiter: LoginLimiter,
    trusted_proxies: Vec<IpAddr>,
    data_dir: Option<DataDir>,
    configured_passwords: Vec<std::path::PathBuf>,
    /// Roots and their storage backends. Roots added, edited or removed
    /// through the API change it while the server runs; requests already
    /// holding a storage keep it until they finish.
    registry: std::sync::RwLock<Registry>,
    /// Serializes adding, editing and removing roots, which check the
    /// registry, write the database and the password file, then swap the
    /// registry entry.
    root_changes: Arc<tokio::sync::Mutex<()>>,
    /// Admission closes before shutdown waits for detached root edits.
    root_edits: tokio_util::task::TaskTracker,
    /// Serializes saving the server settings with applying them.
    settings_changes: tokio::sync::Mutex<()>,
    /// What local roots that leave `watch` unset get.
    watch_by_default: bool,
    automation: Arc<crate::autoscan::Automation>,
    media: Option<Media>,
    /// The metadata source; `None` in tests that do not need one.
    metadata: Option<crate::scan::MetadataEnv>,
    /// Where scans store covers from the works' own folders. The server
    /// always has it, with or without a metadata source.
    covers: Option<neo_scanner::metadata::CoverDirs>,
    /// Limits concurrent work downloads to `archive::MAX_ARCHIVES`.
    archives: Arc<Semaphore>,
    transfer_stall: Duration,
    /// Thumbnails are decoded and resized in memory, so at most two run at
    /// once (book/src/scanning.md).
    thumbnails: Arc<Semaphore>,
    /// Each open task event stream polls the database, so their number is
    /// capped at [`MAX_EVENT_STREAMS`].
    event_streams: Arc<Semaphore>,
    scheduler: Scheduler,
    /// The frontend's files when they do not come from `embed-web`.
    web: Option<WebSource>,
}

/// Open `GET /api/tasks/{id}/events` streams across all users.
pub const MAX_EVENT_STREAMS: usize = 16;

impl AppState {
    /// The state starts not ready. `serve` marks it ready once startup is
    /// done, and until then only the system endpoints answer normally.
    ///
    /// `trusted_proxies` lists the reverse proxies whose `X-Forwarded-For`
    /// header names the client. Requests from any other address are counted
    /// against their own socket address.
    pub fn new(db: Db, tokens: Tokens, trusted_proxies: Vec<IpAddr>) -> Self {
        let scheduler = Scheduler::new(db.clone(), 1, 4);
        Self(Arc::new(Inner {
            db,
            tokens,
            ready: AtomicBool::new(false),
            limiter: LoginLimiter::default(),
            trusted_proxies,
            data_dir: None,
            configured_passwords: Vec::new(),
            registry: std::sync::RwLock::new(Registry::default()),
            root_changes: Arc::new(tokio::sync::Mutex::new(())),
            root_edits: tokio_util::task::TaskTracker::new(),
            settings_changes: tokio::sync::Mutex::new(()),
            watch_by_default: false,
            automation: Arc::default(),
            media: None,
            metadata: None,
            covers: None,
            archives: archive::archive_slots(),
            transfer_stall: archive::STALL_TIMEOUT,
            thumbnails: Arc::new(Semaphore::new(2)),
            event_streams: Arc::new(Semaphore::new(MAX_EVENT_STREAMS)),
            scheduler,
            web: None,
        }))
    }

    /// The scheduler built from the configured job limits. `new` starts
    /// with one that has the default limits.
    pub fn with_scheduler(mut self, scheduler: Scheduler) -> Self {
        self.inner_mut().scheduler = scheduler;
        self
    }

    /// Where covers and the thumbnail cache live. Set before the state is
    /// shared.
    pub fn with_data_dir(mut self, data_dir: DataDir) -> Self {
        self.inner_mut().data_dir = Some(data_dir);
        self
    }

    /// Protects config-owned password files during runtime reclamation.
    pub fn with_configured_passwords(mut self, paths: Vec<std::path::PathBuf>) -> Self {
        self.inner_mut().configured_passwords = paths;
        self
    }

    /// The roots, config ones first in config order.
    pub fn with_roots(mut self, roots: Vec<RootEntry>) -> Self {
        self.inner_mut()
            .registry
            .get_mut()
            .expect("registry lock poisoned")
            .roots = roots;
        self
    }

    /// The storage backend of each root that can be read, in config order.
    /// A root whose backend failed to load, such as an OpenList root whose
    /// password cannot be read, is absent.
    pub fn with_storages(mut self, storages: Vec<(String, Arc<dyn Storage>)>) -> Self {
        self.inner_mut()
            .registry
            .get_mut()
            .expect("registry lock poisoned")
            .storages = storages;
        self
    }

    pub fn with_watch_by_default(mut self, on: bool) -> Self {
        self.inner_mut().watch_by_default = on;
        self
    }

    pub(crate) fn watch_by_default(&self) -> bool {
        self.0.watch_by_default
    }

    pub(crate) fn automation(&self) -> &crate::autoscan::Automation {
        &self.0.automation
    }

    /// The transcode cache and the media limits. Without them `prepare` and
    /// `playable` answer 500.
    pub fn with_media(mut self, media: Media) -> Self {
        self.inner_mut().media = Some(media);
        self
    }

    pub fn with_metadata(mut self, metadata: crate::scan::MetadataEnv) -> Self {
        self.inner_mut().metadata = Some(metadata);
        self
    }

    /// Lets scans take covers from the works' own folders. Pass the same
    /// directories as the metadata source, so both share one lock.
    pub fn with_covers(mut self, covers: neo_scanner::metadata::CoverDirs) -> Self {
        self.inner_mut().covers = Some(covers);
        self
    }

    /// Serves the frontend from `web` instead of the copy `embed-web`
    /// compiles in.
    pub fn with_web(mut self, web: WebSource) -> Self {
        self.inner_mut().web = Some(web);
        self
    }

    fn web(&self) -> Option<&WebSource> {
        self.0.web.as_ref()
    }

    fn metadata(&self) -> Option<&crate::scan::MetadataEnv> {
        self.0.metadata.as_ref()
    }

    fn settings_changes(&self) -> &tokio::sync::Mutex<()> {
        &self.0.settings_changes
    }

    fn covers(&self) -> Option<&neo_scanner::metadata::CoverDirs> {
        self.0
            .covers
            .as_ref()
            .or_else(|| self.metadata().map(|m| &m.dirs))
    }

    /// See `CoverDirs::lock`. When neither a metadata source nor folder
    /// covers are set up, nothing replaces covers while the server runs, so
    /// there is nothing to wait for.
    fn cover_lock(&self) -> Option<Arc<tokio::sync::RwLock<std::collections::HashSet<i64>>>> {
        self.covers().map(|c| c.lock.clone())
    }

    /// The scan environment for a scan started now.
    pub(crate) fn scan_env(&self) -> crate::scan::ScanEnv {
        let env = crate::scan::ScanEnv::new(self.db().clone(), &self.root_kinds())
            .with_probes(probe_queue(self))
            .with_covers(self.0.covers.clone());
        match self.metadata() {
            Some(meta) => env.with_metadata(meta.clone()),
            None => env,
        }
    }

    /// How long a work download or a proxied remote file may wait for a
    /// client that stopped reading. Tests shorten it.
    pub fn with_transfer_stall(mut self, stall: Duration) -> Self {
        self.inner_mut().transfer_stall = stall;
        self
    }

    fn transfer_stall(&self) -> Duration {
        self.0.transfer_stall
    }

    fn archives(&self) -> &Arc<Semaphore> {
        &self.0.archives
    }

    fn media(&self) -> Result<&Media, HttpError> {
        self.0
            .media
            .as_ref()
            .ok_or_else(|| HttpError::internal("media is not configured"))
    }

    pub fn storage(&self, root_id: &str) -> Option<Arc<dyn Storage>> {
        self.registry()
            .storages
            .iter()
            .find(|(id, _)| id == root_id)
            .map(|(_, storage)| Arc::clone(storage))
    }

    fn registry(&self) -> std::sync::RwLockReadGuard<'_, Registry> {
        self.0.registry.read().expect("registry lock poisoned")
    }

    /// Adds a root, or replaces the one with the same id, with its storage.
    /// `None` leaves it without a backend, as for an OpenList root whose
    /// password cannot be read.
    pub(crate) fn put_root(&self, entry: RootEntry, storage: Option<Arc<dyn Storage>>) {
        let mut registry = self.0.registry.write().expect("registry lock poisoned");
        let id = entry.id.clone();
        match registry.roots.iter_mut().find(|r| r.id == id) {
            Some(slot) => *slot = entry,
            None => registry.roots.push(entry),
        }
        registry.storages.retain(|(sid, _)| *sid != id);
        if let Some(storage) = storage {
            registry.storages.push((id, storage));
        }
        drop(registry);
        self.0.automation.roots_changed();
    }

    /// Owned, so a change that runs on after its request is dropped keeps
    /// the lock until it finishes.
    pub(crate) async fn lock_root_changes(&self) -> tokio::sync::OwnedMutexGuard<()> {
        Arc::clone(&self.0.root_changes).lock_owned().await
    }

    pub(crate) fn track_root_change(
        &self,
    ) -> Result<tokio_util::task::task_tracker::TaskTrackerToken, HttpError> {
        // Register before checking closure: shutdown either waits for this
        // token, or the closed tracker rejects the edit before it can write.
        let token = self.0.root_edits.token();
        if self.0.root_edits.is_closed() {
            return Err(HttpError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::NotReady,
                "服务正在关闭",
            ));
        }
        Ok(token)
    }

    pub(crate) fn close_root_edits(&self) -> tokio_util::task::TaskTracker {
        self.0.root_edits.close();
        self.0.root_edits.clone()
    }

    pub(crate) fn remove_root(&self, id: &str) {
        let mut registry = self.0.registry.write().expect("registry lock poisoned");
        registry.roots.retain(|r| r.id != id);
        registry.storages.retain(|(sid, _)| sid != id);
        drop(registry);
        self.0.automation.roots_changed();
    }

    fn inner_mut(&mut self) -> &mut Inner {
        Arc::get_mut(&mut self.0).expect("AppState is configured before it is cloned")
    }

    fn data_dir(&self) -> Result<&DataDir, HttpError> {
        self.0
            .data_dir
            .as_ref()
            .ok_or_else(|| HttpError::internal("the data directory is not configured"))
    }

    pub(crate) fn roots(&self) -> Vec<RootEntry> {
        self.registry().roots.clone()
    }

    /// Root ids with their kinds, for the scan environment.
    pub(crate) fn root_kinds(&self) -> Vec<(String, RootKind)> {
        self.registry()
            .roots
            .iter()
            .map(|r| (r.id.clone(), r.kind))
            .collect()
    }

    fn event_streams(&self) -> Arc<Semaphore> {
        self.0.event_streams.clone()
    }

    fn thumbnails(&self) -> Arc<Semaphore> {
        self.0.thumbnails.clone()
    }

    pub fn scheduler(&self) -> &Scheduler {
        &self.0.scheduler
    }

    pub fn set_ready(&self) {
        self.0.ready.store(true, Ordering::Release);
    }

    fn is_ready(&self) -> bool {
        self.0.ready.load(Ordering::Acquire)
    }

    pub(crate) fn db(&self) -> &Db {
        &self.0.db
    }

    pub(crate) fn tokens(&self) -> &Tokens {
        &self.0.tokens
    }

    fn limiter(&self) -> &LoginLimiter {
        &self.0.limiter
    }

    fn trusted_proxies(&self) -> &[IpAddr] {
        &self.0.trusted_proxies
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(system::health))
        .route("/api/ready", get(system::ready))
        .route("/api/version", get(system::version))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/me", get(auth::me))
        .route("/api/auth/media-token", post(auth::media_token))
        .route("/api/auth/password", put(auth::change_password))
        .route("/api/auth/logout-all", post(auth::logout_all))
        .route("/api/works", get(works::list))
        .route("/api/me/favorites", get(library::favorites))
        .route(
            "/api/me/favorites/{work_id}",
            put(library::add_favorite).delete(library::remove_favorite),
        )
        .route("/api/me/history", get(library::history))
        .route("/api/me/reviews", get(library::reviewed))
        .route(
            "/api/me/reviews/{work_id}",
            put(library::put_review).delete(library::delete_review),
        )
        .route(
            "/api/me/settings",
            get(analysis::get_settings).put(analysis::put_settings),
        )
        .route("/api/circles", get(library::circles))
        .route("/api/vas", get(library::vas))
        .route("/api/tags", get(library::tags))
        .route(
            "/api/me/progress/{work_id}",
            put(progress::report).delete(progress::clear),
        )
        .route("/api/me/progress/{work_id}/finish", post(progress::finish))
        .route("/api/works/{id}", get(works::detail))
        .route(
            "/api/works/{id}/cover",
            get(covers::cover).layer(middleware::map_response(access::routine)),
        )
        .route("/api/works/{id}/metadata", put(edit::set_metadata))
        .route("/api/admin/missing", get(edit::missing))
        .route("/api/admin/missing/cleanup", post(edit::cleanup))
        .route("/api/works/{id}/refresh", post(metadata::refresh))
        .route("/api/admin/roots", get(admin::roots).post(roots::create))
        .route(
            "/api/admin/roots/{id}",
            put(roots::update).delete(roots::remove),
        )
        .route(
            "/api/admin/settings",
            get(server_settings::get).put(server_settings::put),
        )
        .route(
            "/api/admin/settings/proxy-check",
            post(server_settings::check),
        )
        .route("/api/admin/scan", post(admin::scan))
        .route("/api/admin/tags/merge", post(admin::merge_tags))
        .route("/api/tasks/{id}", get(tasks::get))
        .route("/api/tasks", get(tasks::list))
        .route("/api/tasks/{id}/cancel", post(tasks::cancel))
        .route("/api/tasks/{id}/move-to-front", post(tasks::move_to_front))
        .route("/api/tasks/{id}/events", get(tasks::events))
        .route(
            "/api/tracks/{id}/stream",
            get(media::stream).layer(middleware::map_response(access::routine)),
        )
        .route("/api/tracks/{id}/download", get(media::download))
        .route("/api/tracks/{id}/subtitle", get(media::subtitle))
        .route("/api/tracks/{id}/fonts", get(media::fonts))
        .route("/api/tracks/{id}/hls/index.m3u8", get(video::playlist))
        .route(
            "/api/tracks/{id}/hls/{set}/{segment}",
            get(video::segment).layer(middleware::map_response(access::routine)),
        )
        .route("/api/tracks/{id}/prepare", post(playback::prepare))
        // Axum dispatches HEAD to GET by default, but these handlers can queue analysis.
        .route(
            "/api/tracks/{id}/loudness",
            get(analysis::loudness).head(method_not_allowed),
        )
        .route(
            "/api/tracks/{id}/waveform",
            get(analysis::waveform).head(method_not_allowed),
        )
        .route("/api/works/{id}/download", get(archive::download))
        .route("/api/tracks/{id}/playable", get(playback::playable))
        // Page paths get the embedded frontend. Unknown /api paths and
        // missing files still get the JSON 404.
        .route_layer(text_compression())
        .fallback(
            web::fallback
                .layer(web::compression())
                .layer(middleware::map_response(access::routine)),
        )
        .method_not_allowed_fallback(method_not_allowed)
        .layer(middleware::from_fn_with_state(state.clone(), require_ready))
        .layer(middleware::from_fn(security_headers))
        .layer(access::layer())
        .with_state(state)
}

async fn method_not_allowed() -> HttpError {
    HttpError::new(
        StatusCode::METHOD_NOT_ALLOWED,
        ErrorCode::InvalidRequest,
        "这个接口不支持该请求方法",
    )
}

const ALWAYS_AVAILABLE: [&str; 3] = ["/api/health", "/api/ready", "/api/version"];

/// Pages and their assets. The page's own policy is a meta tag from the
/// frontend build, and a meta tag cannot set frame-ancestors. A stricter
/// header here would also apply inside JASSUB's worker, which compiles
/// WebAssembly.
const PAGE_CSP: &str = "frame-ancestors 'none'";
/// API answers and library files never need to run or load anything, so a
/// file opened on its own, a page smuggled into a library say, does nothing.
const API_CSP: &str = "default-src 'none'; frame-ancestors 'none'";

/// A browser's PDF viewer counts as a plugin, which `default-src 'none'`
/// would block.
fn is_pdf(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.starts_with("application/pdf"))
}

/// Headers every response gets (book/src/deployment.md, "安全"). The app
/// is never framed, so a page elsewhere on the LAN cannot overlay it and
/// trick an admin into a click. Media tokens travel in URLs, so no URL is
/// sent on as a Referer. API answers that set no caching rule of their own
/// carry tokens or personal data, so no cache stores them.
/// Below this, gzip's own header and the work to unpack it outweigh what it
/// saves. The frontend's files use the same floor.
const MIN_COMPRESSED_BYTES: u64 = 1024;

/// Gzip for the API's JSON and plain text, such as subtitles and the tag
/// dictionary export, when the client accepts it: a page of works is about
/// 18 KB and compresses about four to one. Media, covers, downloads and the
/// task event stream are left alone. So is any answer that offers byte
/// ranges, such as a text file through `stream`, because a range must index
/// the bytes as stored.
pub fn text_compression() -> CompressionLayer<impl Predicate> {
    CompressionLayer::new().compress_when(SizeAbove::new(MIN_COMPRESSED_BYTES).and(
        |_: StatusCode, _: Version, headers: &HeaderMap, _: &Extensions| {
            is_compressible_text(headers)
        },
    ))
}

fn is_compressible_text(headers: &HeaderMap) -> bool {
    let kind = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let text = ["application/json", "text/vtt", "text/plain"]
        .iter()
        .any(|t| kind.starts_with(t));
    text && !headers.contains_key(header::ACCEPT_RANGES)
}

async fn security_headers(req: Request, next: Next) -> Response {
    let api = req.uri().path().starts_with("/api/");
    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    if api && !headers.contains_key(header::CACHE_CONTROL) {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    let policy = if api && !is_pdf(headers) {
        API_CSP
    } else {
        PAGE_CSP
    };
    let defaults = [
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::X_FRAME_OPTIONS, "DENY"),
        (header::CONTENT_SECURITY_POLICY, policy),
        (header::REFERRER_POLICY, "no-referrer"),
    ];
    for (name, value) in defaults {
        if !headers.contains_key(&name) {
            headers.insert(name, HeaderValue::from_static(value));
        }
    }
    response
}

async fn require_ready(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if state.is_ready() || ALWAYS_AVAILABLE.contains(&req.uri().path()) {
        next.run(req).await
    } else {
        HttpError::not_ready().into_response()
    }
}

/// Renders a URI for the access log without its `token` query parameter,
/// which carries media tokens. Keys are compared after percent-decoding, the
/// same way the query extractor reads them.
pub fn redact_token(uri: &Uri) -> String {
    let Some(query) = uri.query() else {
        return uri.path().to_owned();
    };
    let kept: Vec<&str> = query
        .split('&')
        .filter(|pair| {
            let key = url::form_urlencoded::parse(pair.as_bytes())
                .next()
                .map(|(k, _)| k.into_owned());
            key.as_deref() != Some("token")
        })
        .collect();
    if kept.is_empty() {
        uri.path().to_owned()
    } else {
        format!("{}?{}", uri.path(), kept.join("&"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_text_without_byte_ranges_is_compressed() {
        let headers = |pairs: &[(header::HeaderName, &'static str)]| {
            let mut h = HeaderMap::new();
            for (name, value) in pairs {
                h.insert(name, HeaderValue::from_static(value));
            }
            h
        };
        for kind in [
            "application/json",
            "text/vtt; charset=utf-8",
            "text/plain; charset=utf-8",
        ] {
            assert!(
                is_compressible_text(&headers(&[(header::CONTENT_TYPE, kind)])),
                "{kind}"
            );
        }
        for kind in [
            "audio/flac",
            "image/jpeg",
            "text/event-stream",
            "application/zip",
        ] {
            assert!(
                !is_compressible_text(&headers(&[(header::CONTENT_TYPE, kind)])),
                "{kind}"
            );
        }
        // A text file through `stream` keeps its bytes for range requests.
        assert!(!is_compressible_text(&headers(&[
            (header::CONTENT_TYPE, "text/plain"),
            (header::ACCEPT_RANGES, "bytes"),
        ])));
    }

    fn redact(uri: &str) -> String {
        redact_token(&uri.parse().unwrap())
    }

    #[test]
    fn token_is_removed_from_logged_uri() {
        assert_eq!(redact("/api/x"), "/api/x");
        assert_eq!(redact("/api/x?token=abc"), "/api/x");
        assert_eq!(redact("/api/x?a=1&token=abc&b=2"), "/api/x?a=1&b=2");
        assert_eq!(redact("/api/x?tok%65n=abc&b=2"), "/api/x?b=2");
        assert_eq!(redact("/api/x?token&tokens=1"), "/api/x?tokens=1");
    }
}
