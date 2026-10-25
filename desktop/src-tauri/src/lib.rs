//! The kikoeru-neo desktop shell (book/src/desktop.md). It opens either the
//! bundled mode picker, the backend running in this process, or a remote
//! server, and gives each kind of page only the commands of "外壳给页面的
//! 命令".

mod config;
mod import;
mod local;
mod logs;
mod roots;
mod smoke;
#[cfg(test)]
mod spa_tests;
mod tools;
mod tray;
mod web;

use std::{
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use config::{DesktopConfig, Mode};
use neo_core::dto::{self, DesktopState};
use neo_server::embed::{Server, WebSource};
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder, ipc::CapabilityBuilder};

/// The picker's path in the bundled frontend.
const PICKER: &str = "desktop";

/// Tells the page that the macOS title bar is drawn over it, so the page
/// leaves room for the traffic lights and marks where the window drags.
#[cfg(target_os = "macos")]
const OVERLAY_TITLE_BAR: &str = "window.__KIKOERU_NEO_TITLE_BAR__ = 'overlay';";

/// Where the close button goes. The native title bar is a different height
/// on different macOS versions, so the buttons are pinned instead: their top
/// lands near `y` minus their own small default offset, and their bottom
/// stays inside the 40px strip that web/src/app.css leaves for them.
#[cfg(target_os = "macos")]
const TRAFFIC_LIGHTS: tauri::LogicalPosition<f64> = tauri::LogicalPosition { x: 16.0, y: 18.0 };

/// Lets a page move and zoom its window from a `data-tauri-drag-region`,
/// which the overlay title bar needs because the page covers the real one.
const WINDOW_DRAG: [&str; 2] = [
    "core:window:allow-start-dragging",
    "core:window:allow-internal-toggle-maximize",
];

struct Shell {
    config_file: PathBuf,
    config: Mutex<DesktopConfig>,
    /// The local backend while local mode is on.
    server: tokio::sync::Mutex<Option<Server>>,
    /// Held across every mode change and every import, so they never
    /// interleave: an import started before the mode was switched away is
    /// not applied to a backend that is stopping.
    transition: tokio::sync::Mutex<()>,
    /// The folder the user last chose in `import_discover`'s dialog. The
    /// other import commands read only this folder, so a page cannot make
    /// the shell open a path the user never picked.
    import_pick: Mutex<Option<PathBuf>>,
    #[cfg(test)]
    import_read_barrier: Option<std::sync::Arc<tokio::sync::Barrier>>,
    /// Set when the saved mode could not be opened, for the picker to show.
    last_error: Mutex<Option<String>>,
    windows: AtomicU64,
    /// The frontend the local backend serves, shared with the picker.
    web: WebSource,
}

impl Shell {
    fn save(&self, update: impl FnOnce(&mut DesktopConfig)) -> Result<(), String> {
        let mut config = self.config.lock().expect("config mutex poisoned");
        update(&mut config);
        config
            .save(&self.config_file)
            .map_err(|e| format!("无法保存设置：{e}"))
    }

    fn data_dir() -> Result<PathBuf, String> {
        neo_server::embed::default_data_dir().ok_or_else(|| "找不到系统的应用数据目录".to_owned())
    }
}

/// The pages a window may show. A window never leaves its own origin, so
/// a page cannot carry it to another origin whose capabilities are still
/// registered, such as the picker's or a local backend's old port.
#[derive(Clone)]
enum Stay {
    /// The bundled frontend: `tauri://localhost`, or `http://tauri.localhost`
    /// on Windows.
    Bundled,
    Origin(String),
}

impl Stay {
    fn allows(&self, url: &url::Url) -> bool {
        match self {
            Self::Bundled => url.scheme() == "tauri" || url.host_str() == Some("tauri.localhost"),
            Self::Origin(origin) => url.origin().ascii_serialization() == *origin,
        }
    }
}

/// A label no window has had before. Each window gets a new one, because a
/// webview's initialization scripts are fixed when it is created, and a
/// page's capability names only its own window's label.
fn next_label<R: tauri::Runtime>(app: &AppHandle<R>) -> String {
    let shell = app.state::<Shell>();
    format!("main-{}", shell.windows.fetch_add(1, Ordering::SeqCst))
}

/// Replaces the main window with a window called `label` showing `url`.
fn open_window<R: tauri::Runtime>(
    app: &AppHandle<R>,
    label: String,
    url: WebviewUrl,
    stay: Stay,
    init: Option<String>,
) -> Result<(), String> {
    let mut builder = WebviewWindowBuilder::new(app, &label, url)
        .title("kikoeru-neo")
        .inner_size(1200.0, 800.0)
        .on_navigation(move |url| {
            let allowed = stay.allows(url);
            if !allowed {
                tracing::warn!(%url, "blocked a navigation away from the window's origin");
            }
            allowed
        });
    if let Some(script) = init {
        builder = builder.initialization_script(&script);
    }
    // The page reaches up to the top edge, like the system's own apps,
    // instead of sitting under a separate title bar strip.
    #[cfg(target_os = "macos")]
    {
        builder = builder
            .title_bar_style(tauri::TitleBarStyle::Overlay)
            .hidden_title(true)
            .traffic_light_position(TRAFFIC_LIGHTS)
            .initialization_script(OVERLAY_TITLE_BAR);
    }
    // The WebDriver test fails on any Content Security Policy violation.
    // Tauri runs this before anything on the page loads, so a stylesheet
    // or worker blocked during startup is recorded too.
    #[cfg(feature = "e2e")]
    {
        builder = builder.initialization_script(CSP_RECORDER);
    }
    // msedgedriver starts the app with WebView2's user data folder and
    // browser arguments (among them the DevTools port) in these variables.
    // Tauri always passes its own folder and arguments, which take
    // precedence, so the test build hands the driver's over.
    #[cfg(feature = "e2e")]
    if let Some(dir) = std::env::var_os("WEBVIEW2_USER_DATA_FOLDER") {
        builder = builder.data_directory(PathBuf::from(dir));
    }
    #[cfg(all(feature = "e2e", windows))]
    if let Ok(args) = std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS") {
        builder = builder.additional_browser_args(&args);
    }
    let window = builder.build().map_err(|e| format!("无法打开窗口：{e}"))?;
    // Closing hides the window, so playback goes on; the tray quits.
    let hidden = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = hidden.hide();
        }
    });
    for (other, old) in app.webview_windows() {
        if other != label {
            let _ = old.destroy();
        }
    }
    Ok(())
}

fn open_picker<R: tauri::Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let label = next_label(app);
    open_window(
        app,
        label,
        WebviewUrl::App(PICKER.into()),
        Stay::Bundled,
        None,
    )
}

/// Starts the local backend if it is not running, and shows it signed in.
async fn open_local(app: &AppHandle) -> Result<(), String> {
    let shell = app.state::<Shell>();
    let mut server = shell.server.lock().await;
    if let Some(old) = server.take() {
        stop_server(old).await;
    }
    // On its own task: the backend's futures are Send but not for every
    // lifetime, which Tauri's command futures require.
    let data_dir = Shell::data_dir()?;
    let (started, token) = tauri::async_runtime::spawn(local::start(data_dir, shell.web.clone()))
        .await
        .map_err(|e| format!("无法启动本机后端：{e}"))?
        .map_err(|e| format!("无法启动本机后端：{e:#}"))?;
    let port = started.port();
    *server = Some(started);
    drop(server);
    renew_daily(app.clone(), port);
    let label = next_label(app);
    allow_page(
        app,
        &label,
        &format!("http://127.0.0.1:{port}"),
        &[
            "allow-desktop-state",
            "allow-switch-mode",
            "allow-pick-folder",
            "allow-import-discover",
            "allow-import-preview",
            "allow-import-run",
            "allow-media-state",
            "allow-open-log-dir",
            "core:event:allow-listen",
            "core:event:allow-unlisten",
            WINDOW_DRAG[0],
            WINDOW_DRAG[1],
        ],
    )?;
    let url = format!("http://127.0.0.1:{port}/")
        .parse()
        .expect("a loopback URL");
    open_window(
        app,
        label,
        WebviewUrl::External(url),
        Stay::Origin(format!("http://127.0.0.1:{port}")),
        Some(local::init_script(port, &token)),
    )
}

fn open_server(app: &AppHandle, url: url::Url) -> Result<(), String> {
    let origin = url.origin().ascii_serialization();
    // A remote page gets the media state, moving its own window and going
    // back to the picker, and nothing else. Its login page is where a user
    // connected to the wrong server looks for the way out.
    let label = next_label(app);
    allow_page(
        app,
        &label,
        &origin,
        &[
            "allow-switch-mode",
            "allow-media-state",
            "core:event:allow-listen",
            "core:event:allow-unlisten",
            WINDOW_DRAG[0],
            WINDOW_DRAG[1],
        ],
    )?;
    let stay = Stay::Origin(url.origin().ascii_serialization());
    open_window(app, label, WebviewUrl::External(url), stay, None)
}

/// Writes a fresh session token into the local page once a day, well
/// inside the token's 30-day life, for as long as this backend runs.
fn renew_daily(app: AppHandle, port: u16) {
    const EVERY: std::time::Duration = std::time::Duration::from_secs(24 * 3600);
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(EVERY).await;
            let shell = app.state::<Shell>();
            let server = shell.server.lock().await;
            let Some(server) = server.as_ref().filter(|s| s.port() == port) else {
                return;
            };
            let token = match server.session_token(local::ADMIN).await {
                Ok(token) => token,
                Err(e) => {
                    tracing::warn!(error = %e, "cannot renew the local session");
                    continue;
                }
            };
            for window in app.webview_windows().into_values() {
                let _ = window.eval(local::renew_script(&token));
            }
        }
    });
}

/// Grants `permissions` to pages served from `origin` in the window `label`.
fn allow_page(
    app: &AppHandle,
    label: &str,
    origin: &str,
    permissions: &[&str],
) -> Result<(), String> {
    app.add_capability(page_capability(label, origin, permissions))
        .map_err(|e| format!("无法设置页面权限：{e}"))
}

/// Tauri keeps every capability it was given and merges them, so a grant
/// for all `main-*` windows would let a later remote window on the same
/// origin (a server reached at an old local port) call the local commands.
/// Naming the one window it is for leaves nothing for later windows.
fn page_capability(label: &str, origin: &str, permissions: &[&str]) -> CapabilityBuilder {
    // Not local: the bundled picker keeps only its own capability.
    let mut capability = CapabilityBuilder::new(format!("page-{label}"))
        .local(false)
        .remote(format!("{origin}/*"))
        .window(label);
    for permission in permissions {
        capability = capability.permission(*permission);
    }
    capability
}

async fn stop_local(shell: &Shell) {
    if let Some(server) = shell.server.lock().await.take() {
        stop_server(server).await;
    }
}

async fn stop_server(server: Server) {
    let stopped = tauri::async_runtime::spawn(server.stop()).await;
    if let Ok(Err(e)) = stopped {
        tracing::warn!(error = %e, "the local backend did not stop cleanly");
    }
}

#[tauri::command]
fn desktop_state(shell: State<'_, Shell>) -> DesktopState {
    let config = shell.config.lock().expect("config mutex poisoned").clone();
    DesktopState {
        mode: config.mode,
        server_url: config.server_url,
        last_error: shell
            .last_error
            .lock()
            .expect("error mutex poisoned")
            .clone(),
    }
}

#[tauri::command]
async fn use_local(app: AppHandle) -> Result<(), String> {
    let shell = app.state::<Shell>();
    let _changing = shell.transition.lock().await;
    open_local(&app).await?;
    shell.save(|c| c.mode = Some(Mode::Local))
}

#[tauri::command]
async fn use_server(app: AppHandle, url: String) -> Result<(), String> {
    let url = check_server(&config::server_url(&url)?).await?;
    let shell = app.state::<Shell>();
    let _changing = shell.transition.lock().await;
    stop_local(&shell).await;
    shell.save(|c| {
        c.mode = Some(Mode::Server);
        c.server_url = Some(url.to_string());
    })?;
    open_server(&app, url)
}

/// The server address a redirected `…/api/health` request ended at.
fn server_base(landed: &url::Url) -> Option<url::Url> {
    let path = landed.path().strip_suffix("api/health")?;
    let mut base = landed.clone();
    base.set_path(path);
    base.set_query(None);
    Some(base)
}

/// A kikoeru-neo server answers `/api/health` with `{"status":"ok"}`.
/// Returns the server's address after redirects, such as http to https,
/// since the window is pinned to that origin.
async fn check_server(url: &url::Url) -> Result<url::Url, String> {
    let health = url.join("api/health").map_err(|e| e.to_string())?;
    let answer = reqwest::Client::new()
        .get(health)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("连不上这个服务器：{}", e.without_url()))?;
    let landed = answer.url().clone();
    // An upgrade to https or another port is fine; another host is not,
    // or anyone on the path of a plain http request could pin the app to
    // their own site.
    if landed.host_str() != url.host_str() {
        return Err(format!(
            "这个服务器把请求转到了另一个主机：{}",
            landed.host_str().unwrap_or_default()
        ));
    }
    let body: serde_json::Value = answer
        .json()
        .await
        .map_err(|_| "这个地址不是 kikoeru-neo 服务器".to_owned())?;
    if body["status"] != "ok" {
        return Err("这个地址不是 kikoeru-neo 服务器".to_owned());
    }
    server_base(&landed).ok_or_else(|| "这个服务器把请求转到了别处".to_owned())
}

#[tauri::command]
async fn switch_mode<R: tauri::Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let shell = app.state::<Shell>();
    let _changing = shell.transition.lock().await;
    stop_local(&shell).await;
    shell.save(|c| c.mode = None)?;
    open_picker(&app)
}

/// Shows the system's folder dialog. `None` when the user cancels.
async fn choose_folder(app: &AppHandle, title: &str) -> Result<Option<PathBuf>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title(title)
        .pick_folder(move |folder| {
            let _ = tx.send(folder);
        });
    let Some(folder) = rx.await.ok().flatten() else {
        return Ok(None);
    };
    folder
        .into_path()
        .map(Some)
        .map_err(|e| format!("无法读取所选文件夹：{e}"))
}

/// The page adds the folder itself through the roots API.
#[tauri::command]
async fn pick_folder(app: AppHandle) -> Result<Option<String>, String> {
    // The Windows WebDriver test cannot click a system dialog. Only builds
    // with the `e2e` feature read this, so release builds never do.
    #[cfg(feature = "e2e")]
    if let Some(path) = std::env::var_os("KIKOERU_NEO_E2E_PICK_FOLDER") {
        return path
            .into_string()
            .map(Some)
            .map_err(|_| "所选文件夹的路径不是有效的文本".to_owned());
    }
    let Some(path) = choose_folder(&app, "选择媒体库文件夹").await? else {
        return Ok(None);
    };
    path.into_os_string()
        .into_string()
        .map(Some)
        .map_err(|_| "所选文件夹的路径不是有效的文本".to_owned())
}

#[tauri::command]
async fn import_discover(
    app: AppHandle,
) -> Result<Option<dto::ImportDiscovered>, dto::ImportFailure> {
    let Some(pick) = choose_folder(&app, "选择旧 kikoeru 的数据目录")
        .await
        .map_err(import::other)?
    else {
        return Ok(None);
    };
    *app.state::<Shell>()
        .import_pick
        .lock()
        .expect("import mutex poisoned") = Some(pick.clone());
    tauri::async_runtime::spawn_blocking(move || import::discover(pick))
        .await
        .map_err(|e| import::other(format!("找旧数据失败：{e}")))?
        .map(Some)
        .map_err(|e| import::failure(&e))
}

/// Reads the old library from a fresh snapshot. On its own task for the
/// same reason as the backend's start.
async fn open_import(
    shell: &Shell,
    pick: import::Pick,
) -> Result<import::Opened, dto::ImportFailure> {
    let picked = shell
        .import_pick
        .lock()
        .expect("import mutex poisoned")
        .clone();
    if picked.as_ref() != Some(&pick.pick) {
        return Err(import::other("请先用“选择旧数据目录”选一次文件夹"));
    }
    #[cfg(test)]
    if let Some(barrier) = &shell.import_read_barrier {
        // Hold the read boundary so tests can request a mode switch here.
        barrier.wait().await;
        barrier.wait().await;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    tauri::async_runtime::spawn(import::open(pick, now))
        .await
        .map_err(|e| import::other(format!("无法读取旧库：{e}")))?
        .map_err(|e| import::failure(&e))
}

// The page passes arguments in snake_case, as the contract names them.
#[tauri::command(rename_all = "snake_case")]
async fn import_preview(
    shell: State<'_, Shell>,
    pick: PathBuf,
    accept_guessed: bool,
    recover_journal: bool,
) -> Result<dto::ImportPreview, dto::ImportFailure> {
    let pick = import::Pick {
        pick,
        accept_guessed,
        recover_journal,
    };
    Ok(import::preview(open_import(&shell, pick).await?))
}

#[tauri::command(rename_all = "snake_case")]
async fn import_run<R: tauri::Runtime>(
    app: AppHandle<R>,
    pick: PathBuf,
    accept_guessed: bool,
    recover_journal: bool,
    to_me: Vec<String>,
    add_roots: Vec<String>,
) -> Result<dto::ImportResult, dto::ImportFailure> {
    let shell = app.state::<Shell>();
    // Mode changes must also wait for the snapshot and preflight, so the
    // backend and the import's window stay alive for the whole operation.
    let _changing = shell.transition.lock().await;
    still_local(&shell).await.map_err(import::other)?;
    let opened = open_import(
        &shell,
        import::Pick {
            pick,
            accept_guessed,
            recover_journal,
        },
    )
    .await?;
    if let Some(refusal) = &opened.refusal {
        return Err(dto::ImportFailure {
            code: dto::ImportErrorCode::WouldRefuse,
            message: refusal.join("\n"),
        });
    }
    let failed = |e: anyhow::Error| import::failure(&e);
    let users = import::users(&to_me, &opened.plan).map_err(failed)?;
    let server = shell.server.lock().await;
    let server = server
        .as_ref()
        .ok_or_else(|| import::other("本机后端没有运行"))?;
    let token = server.session_token(local::ADMIN).await.map_err(failed)?;
    let api = roots::Api::new(server.port(), token).map_err(failed)?;
    // Checked before the merge, so a wrong name changes nothing.
    let existing = api
        .list()
        .await
        .map_err(|e| import::other(format!("无法读取媒体库：{e:#}")))?;
    let folders = import::roots_to_add(&opened.report, &add_roots, &roots::local_paths(&existing))
        .map_err(failed)?;
    let merged = server
        .merge_legacy(&opened.plan, &users)
        .await
        .map_err(|e| dto::ImportFailure {
            code: neo_server::import::error::code(&e),
            message: format!("合并失败，本机库没有改动：{e:#}"),
        })?;
    let mut imported = import::result(merged);
    for (name, path) in &folders {
        let added = api.add_local(name, path).await.map_err(|e| {
            import::other(format!("用户数据已经合并，但加媒体库 {name} 失败：{e:#}"))
        })?;
        imported.roots_added.push(added);
    }
    Ok(imported)
}

/// Keeps every CSP violation of the page in `window.__cspViolations`, for
/// desktop/e2e/run.mjs.
#[cfg(feature = "e2e")]
const CSP_RECORDER: &str = "window.__cspViolations = []; \
    document.addEventListener('securitypolicyviolation', function (e) { \
      window.__cspViolations.push(e.effectiveDirective + ' blocked ' + (e.blockedURI || 'inline')); \
    });";

/// Writes the WebView2 variables the WebDriver job's driver set, for its
/// log artifact.
#[cfg(feature = "e2e")]
fn log_webview2_environment() {
    let lines: String = std::env::vars()
        .filter(|(key, _)| key.starts_with("WEBVIEW2"))
        .map(|(key, value)| format!("{key}={value}\n"))
        .collect();
    let _ = std::fs::write(std::env::temp_dir().join("kikoeru-neo-e2e-env.txt"), lines);
}

/// An import applies only while local mode is on: the folder dialog can
/// stay open while the tray switches the mode away.
async fn still_local(shell: &Shell) -> Result<(), String> {
    let local = shell.config.lock().expect("config mutex poisoned").mode == Some(Mode::Local);
    if local && shell.server.lock().await.is_some() {
        Ok(())
    } else {
        Err("已经不在本机模式，没有导入".to_owned())
    }
}

#[tauri::command]
fn media_state(app: AppHandle, title: Option<String>, artist: Option<String>, playing: bool) {
    tray::now_playing(&app, title.as_deref(), artist.as_deref(), playing);
}

/// Shows the folder of the log files in the system's file manager. It
/// opens only this one folder, so a page cannot use it to open anything
/// else.
#[tauri::command]
fn open_log_dir(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;

    let dir = app
        .path()
        .app_log_dir()
        .map_err(|e| format!("这台电脑上找不到日志文件夹：{e}"))?;
    // The files are created on the first line, so the folder may not exist
    // yet if logging to it failed.
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建日志文件夹：{e}"))?;
    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|e| format!("无法打开日志文件夹：{e}"))
}

/// Opens what the saved settings say, or the picker.
async fn open_saved(app: &AppHandle) -> Result<(), String> {
    let config = app
        .state::<Shell>()
        .config
        .lock()
        .expect("config mutex poisoned")
        .clone();
    let opened = match (config.mode, config.server_url) {
        (Some(Mode::Local), _) => open_local(app).await,
        // Checked first: a webview that fails to load would not tell us,
        // and the picker would never get the chance to show why.
        (Some(Mode::Server), Some(url)) => match config::server_url(&url) {
            Ok(url) => match check_server(&url).await {
                Ok(url) => open_server(app, url),
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        },
        _ => return open_picker(app),
    };
    if let Err(e) = opened {
        *app.state::<Shell>()
            .last_error
            .lock()
            .expect("error mutex poisoned") = Some(e);
        open_picker(app)?;
    }
    Ok(())
}

pub fn run() {
    #[cfg(feature = "e2e")]
    log_webview2_environment();
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if let [_, flag, report, library @ ..] = args.as_slice()
        && flag == "--smoke"
        && library.len() <= 1
    {
        let library = library.first().map(std::path::Path::new);
        std::process::exit(smoke::main(std::path::Path::new(report), library));
    }
    #[cfg(feature = "e2e")]
    if let [_, flag, library, port_file] = args.as_slice()
        && flag == "--e2e-server"
    {
        std::process::exit(smoke::e2e_server(
            std::path::Path::new(library),
            std::path::Path::new(port_file),
        ));
    }
    let mut context = web::context();
    let web = web::share(&mut context);
    tauri::Builder::default()
        // One process per user: a second local backend would share the
        // data directory and clear the first one's temporary files.
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            tray::show(app)
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            desktop_state,
            use_local,
            use_server,
            switch_mode,
            pick_folder,
            import_discover,
            import_preview,
            import_run,
            media_state,
            open_log_dir,
        ])
        .setup(|app| {
            // The log folder is known only once the app exists, so the few
            // lines before this point go nowhere.
            logs::init(app.path().app_log_dir().ok().as_deref());
            let config_file = config::path(&app.path().app_config_dir()?);
            app.manage(Shell {
                config: Mutex::new(DesktopConfig::load(&config_file)),
                config_file,
                server: tokio::sync::Mutex::new(None),
                transition: tokio::sync::Mutex::new(()),
                import_pick: Mutex::new(None),
                #[cfg(test)]
                import_read_barrier: None,
                last_error: Mutex::new(None),
                windows: AtomicU64::new(0),
                web,
            });
            tray::create(app.handle())?;
            let handle = app.handle().clone();
            tauri::async_runtime::block_on(open_saved(&handle)).map_err(anyhow::Error::msg)?;
            Ok(())
        })
        .build(context)
        .expect("the desktop shell could not start")
        .run(|app, event| {
            // The backend stops before the process may exit, and so before
            // the single-instance lock is released: a new instance started
            // meanwhile must not open the same data directory while this
            // backend is still stopping its transcodes.
            // Closing the window only hides it, and on macOS clicking the
            // Dock icon of the running app is how people bring it back.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen {
                has_visible_windows: false,
                ..
            } = event
            {
                tray::show(app);
                return;
            }
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                // A held lock means a start or stop is under way, so the
                // backend may be running. Waiting for it here would block
                // the event loop.
                let running = match app.state::<Shell>().server.try_lock() {
                    Ok(server) => server.is_some(),
                    Err(_) => true,
                };
                if running {
                    api.prevent_exit();
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        stop_local(&app.state::<Shell>()).await;
                        app.exit(code.unwrap_or(0));
                    });
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_redirected_health_check_gives_the_server_address() {
        let url = |s: &str| url::Url::parse(s).unwrap();
        assert_eq!(
            server_base(&url("https://k.example/neo/api/health")),
            Some(url("https://k.example/neo/"))
        );
        assert_eq!(
            server_base(&url("https://k.example/api/health?x=1")),
            Some(url("https://k.example/"))
        );
        assert_eq!(server_base(&url("https://k.example/login")), None);
    }

    #[test]
    fn the_picker_may_drag_its_window_too() {
        let picker: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/picker.json")).unwrap();
        let granted = picker["permissions"].as_array().unwrap();
        for permission in WINDOW_DRAG {
            assert!(granted.iter().any(|p| p == permission), "{permission}");
        }
    }

    #[test]
    fn macos_lets_the_window_open_a_plain_http_server() {
        let info = plist::Value::from_reader_xml(&include_bytes!("../Info.plist")[..]).unwrap();
        let ats = info
            .as_dictionary()
            .and_then(|d| d.get("NSAppTransportSecurity"))
            .and_then(plist::Value::as_dictionary)
            .expect("Info.plist sets NSAppTransportSecurity");
        assert_eq!(
            ats.get("NSAllowsArbitraryLoads")
                .and_then(plist::Value::as_boolean),
            Some(true)
        );
        // When present, this key replaces NSAllowsArbitraryLoads for
        // WKWebView, so it must not quietly turn http back off.
        assert!(ats.get("NSAllowsArbitraryLoadsInWebContent").is_none());
    }

    #[test]
    fn a_page_capability_names_only_its_own_window() {
        use tauri::ipc::RuntimeCapability;
        use tauri::utils::acl::capability::CapabilityFile;
        let built =
            page_capability("main-3", "http://127.0.0.1:4321", &["allow-pick-folder"]).build();
        let CapabilityFile::Capability(capability) = built else {
            panic!("one capability");
        };
        assert_eq!(capability.windows, ["main-3"]);
        assert_eq!(capability.identifier, "page-main-3");
        assert!(!capability.local);
        assert_eq!(
            capability.remote.map(|r| r.urls),
            Some(vec!["http://127.0.0.1:4321/*".to_owned()])
        );
    }

    #[test]
    fn a_window_stays_on_its_origin() {
        let url = |s: &str| url::Url::parse(s).unwrap();
        let local = Stay::Origin("http://127.0.0.1:4321".into());
        assert!(local.allows(&url("http://127.0.0.1:4321/works/3")));
        assert!(!local.allows(&url("http://127.0.0.1:4322/")));
        assert!(!local.allows(&url("tauri://localhost/desktop")));
        assert!(!local.allows(&url("https://evil.example/")));
        assert!(Stay::Bundled.allows(&url("tauri://localhost/desktop")));
        assert!(Stay::Bundled.allows(&url("http://tauri.localhost/desktop")));
        assert!(!Stay::Bundled.allows(&url("http://127.0.0.1:4321/")));
    }
}
