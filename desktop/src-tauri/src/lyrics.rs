//! The desktop lyrics window: two subtitle lines in a transparent window
//! that stays above other applications (book/src/desktop.md, "桌面歌词").
//! The page that plays sends the lines, and the window, a bundled page with
//! its own capability (capabilities/lyrics.json), shows them.

use std::sync::Mutex;

use neo_core::dto::{LyricsLines, MediaControl};
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

/// The window's label, which capabilities/lyrics.json names.
pub const LABEL: &str = "lyrics";
/// The window's path in the bundled frontend.
const PAGE: &str = "desktop-lyrics";
const WIDTH: f64 = 960.0;
const HEIGHT: f64 = 160.0;
/// How far above the bottom of the screen the window opens, clear of the
/// Windows taskbar and the macOS Dock at their usual sizes.
const BOTTOM_GAP: f64 = 120.0;

/// The latest lines, so a window opened between two changes starts with
/// them. A new window always starts unlocked.
#[derive(Default)]
pub struct Lyrics {
    lines: Mutex<Option<LyricsLines>>,
}

fn open<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let mut builder = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App(PAGE.into()))
        .title("kikoeru-neo 桌面歌词")
        .inner_size(WIDTH, HEIGHT)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .focused(false)
        // The page is bundled and has nothing to link to.
        .on_navigation(|url| url.scheme() == "tauri" || url.host_str() == Some("tauri.localhost"));
    if let Ok(Some(monitor)) = app.primary_monitor() {
        let size = monitor.size().to_logical::<f64>(monitor.scale_factor());
        let origin = monitor.position().to_logical::<f64>(monitor.scale_factor());
        builder = builder.position(
            origin.x + (size.width - WIDTH) / 2.0,
            origin.y + size.height - HEIGHT - BOTTOM_GAP,
        );
    }
    builder
        .build()
        .map_err(|e| format!("无法打开桌面歌词：{e}"))?;
    Ok(())
}

/// Opens the window, or closes it when it is open.
pub fn toggle<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    match app.get_webview_window(LABEL) {
        Some(window) => window
            .destroy()
            .map_err(|e| format!("无法关闭桌面歌词：{e}")),
        None => open(app),
    }
}

/// Lets clicks through the window, or takes them again. A locked window
/// cannot be clicked, so only the tray unlocks it.
pub fn lock<R: Runtime>(app: &AppHandle<R>, locked: bool) -> Result<(), String> {
    let Some(window) = app.get_webview_window(LABEL) else {
        return Ok(());
    };
    window
        .set_ignore_cursor_events(locked)
        .map_err(|e| format!("无法锁定桌面歌词：{e}"))?;
    let _ = app.emit_to(LABEL, "lyrics-locked", locked);
    Ok(())
}

#[tauri::command]
pub fn lyrics_toggle(app: AppHandle) -> Result<(), String> {
    toggle(&app)
}

#[tauri::command]
pub fn lyrics_lines(app: AppHandle, lines: LyricsLines) {
    *app.state::<Lyrics>()
        .lines
        .lock()
        .expect("lyrics mutex poisoned") = Some(lines.clone());
    let _ = app.emit_to(LABEL, "lyrics-lines", lines);
}

/// What the window shows when it opens.
#[tauri::command]
pub fn lyrics_now(app: AppHandle) -> Option<LyricsLines> {
    app.state::<Lyrics>()
        .lines
        .lock()
        .expect("lyrics mutex poisoned")
        .clone()
}

#[tauri::command]
pub fn lyrics_lock(app: AppHandle, locked: bool) -> Result<(), String> {
    lock(&app, locked)
}

/// The window's playback buttons, which reach the page like the tray's.
#[tauri::command]
pub fn lyrics_control(app: AppHandle, action: MediaControl) {
    crate::tray::control(&app, action);
}
