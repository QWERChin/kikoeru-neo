//! The two places the app's pages come from: the bundle for the picker and
//! the embedded backend for local mode. Both serve the one copy of the SPA
//! that Tauri embeds, with its fallback page for routes such as `/desktop`.

use tauri::utils::assets::AssetKey;

const SPA_MARKER: &str = "__sveltekit";

#[test]
fn the_bundle_and_the_local_backend_serve_the_same_spa() {
    let mut context = crate::web::context();
    let web = crate::web::share(&mut context);
    let index = context
        .assets()
        .get(&AssetKey::from("index.html"))
        .expect("the bundle has the SPA's fallback page")
        .into_owned();
    assert!(String::from_utf8_lossy(&index).contains(SPA_MARKER));

    let dir = tempfile::tempdir().unwrap();
    tauri::async_runtime::block_on(async {
        let server = neo_server::embed::start(neo_server::embed::Options {
            data_dir: dir.path().to_path_buf(),
            ffmpeg: None,
            ffprobe: None,
            web: Some(web),
        })
        .await
        .unwrap();
        let base = format!("http://127.0.0.1:{}", server.port());
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let page = client.get(format!("{base}/desktop")).send().await.unwrap();
        assert_eq!(page.status(), 200);
        let kind = page.headers()["content-type"].to_str().unwrap().to_owned();
        assert!(kind.starts_with("text/html"), "{kind}");
        // Byte for byte, so the page's CSP hashes still match its scripts.
        assert_eq!(page.bytes().await.unwrap(), index);

        // An entry chunk the page links, with the hashed files' caching.
        let html = String::from_utf8_lossy(&index);
        let start = html.find("_app/immutable/").expect("index links a chunk");
        let chunk: String = html[start..]
            .chars()
            .take_while(|c| !matches!(c, '"' | '\'' | ')' | ' '))
            .collect();
        let script = client.get(format!("{base}/{chunk}")).send().await.unwrap();
        assert_eq!(script.status(), 200, "{chunk}");
        assert_eq!(
            script.headers()["cache-control"],
            "public, max-age=31536000, immutable"
        );
        let missing = client
            .get(format!("{base}/_app/immutable/gone.js"))
            .send()
            .await
            .unwrap();
        assert_eq!(missing.status(), 404);

        let version = client
            .get(format!("{base}/api/version"))
            .send()
            .await
            .unwrap();
        assert_eq!(version.status(), 200);
        server.stop().await.unwrap();
    });
}
