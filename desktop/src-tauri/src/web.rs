//! The frontend build, held once. Tauri embeds web/build for the bundled
//! picker, and the local backend serves its pages from that same copy
//! instead of compiling in its own through neo-server's `embed-web`, which
//! would put the whole build into the app twice.

use std::{
    borrow::Cow,
    hash::{DefaultHasher, Hasher},
    sync::Arc,
};

use neo_server::embed::{WebAsset, WebSource};
use tauri::{
    App, Assets, Context, Wry,
    utils::assets::{AssetKey, AssetsIter, CspHash},
};

/// Tauri's context. Everything that needs one calls this, because each
/// expansion of `generate_context!` embeds the frontend build again.
pub fn context() -> Context<Wry> {
    tauri::generate_context!()
}

/// Shares the context's assets with the local backend: the context keeps
/// them for the picker, and the returned source reads the same copy.
pub fn share(context: &mut Context<Wry>) -> WebSource {
    let assets: Arc<dyn Assets<Wry>> = Arc::from(context.set_assets(Box::new(NoAssets)));
    context.set_assets(Box::new(Shared(assets.clone())));
    Arc::new(move |path| {
        let body = assets.get(&AssetKey::from(path))?.into_owned();
        Some(WebAsset {
            mime: mime_guess::from_path(path)
                .first_or_octet_stream()
                .to_string(),
            etag: etag(&body),
            body: Cow::Owned(body),
        })
    })
}

/// A tag that changes with the content. The webview keeps it across
/// restarts, and `DefaultHasher::new` hashes the same way every run, so an
/// unchanged file still revalidates. A Rust update that changed the hash
/// would only cost one more download.
fn etag(body: &[u8]) -> String {
    let mut hasher = DefaultHasher::new();
    hasher.write(body);
    format!("\"{:016x}\"", hasher.finish())
}

struct Shared(Arc<dyn Assets<Wry>>);

impl Assets<Wry> for Shared {
    fn setup(&self, app: &App<Wry>) {
        self.0.setup(app);
    }

    fn get(&self, key: &AssetKey) -> Option<Cow<'_, [u8]>> {
        self.0.get(key)
    }

    fn iter(&self) -> Box<AssetsIter<'_>> {
        self.0.iter()
    }

    fn csp_hashes(&self, html_path: &AssetKey) -> Box<dyn Iterator<Item = CspHash<'_>> + '_> {
        self.0.csp_hashes(html_path)
    }
}

/// Stands in for the assets for the moment `share` has taken them out.
struct NoAssets;

impl Assets<Wry> for NoAssets {
    fn get(&self, _: &AssetKey) -> Option<Cow<'_, [u8]>> {
        None
    }

    fn iter(&self) -> Box<AssetsIter<'_>> {
        Box::new(std::iter::empty())
    }

    fn csp_hashes(&self, _: &AssetKey) -> Box<dyn Iterator<Item = CspHash<'_>> + '_> {
        Box::new(std::iter::empty())
    }
}
