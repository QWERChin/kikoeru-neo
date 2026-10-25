//! The tray icon: the window, playback controls and switching mode. The
//! controls reach the page as `media-control` events; system media keys
//! go through the page's Media Session (book/src/desktop.md).

use neo_core::dto::MediaControl;
use tauri::{
    AppHandle, Emitter, Manager,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
};

const TRAY: &str = "main";

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let item = |id: &str, text: &str| MenuItem::with_id(app, id, text, true, None::<&str>);
    let menu = Menu::with_items(
        app,
        &[
            &item("show", "显示窗口")?,
            &PredefinedMenuItem::separator(app)?,
            &item("toggle", "播放/暂停")?,
            &item("previous", "上一首")?,
            &item("next", "下一首")?,
            &PredefinedMenuItem::separator(app)?,
            &item("switch", "切换模式")?,
            &item("quit", "退出")?,
        ],
    )?;
    let mut tray = TrayIconBuilder::with_id(TRAY)
        .tooltip("kikoeru-neo")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show(app),
            "toggle" => control(app, MediaControl::Toggle),
            "previous" => control(app, MediaControl::Previous),
            "next" => control(app, MediaControl::Next),
            "switch" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = crate::switch_mode(app).await {
                        tracing::warn!(error = %e, "cannot switch mode");
                    }
                });
            }
            "quit" => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

fn control(app: &AppHandle, action: MediaControl) {
    let _ = app.emit("media-control", action);
}

pub fn show(app: &AppHandle) {
    for window in app.webview_windows().into_values() {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Puts what is playing into the tray's tooltip.
pub fn now_playing(app: &AppHandle, title: Option<&str>, artist: Option<&str>, playing: bool) {
    let Some(tray) = app.tray_by_id(TRAY) else {
        return;
    };
    let text = match (title, artist) {
        (Some(title), Some(artist)) => format!("{title} - {artist}"),
        (Some(title), None) => title.to_owned(),
        _ => "kikoeru-neo".to_owned(),
    };
    let text = if playing || title.is_none() {
        text
    } else {
        format!("{text}（已暂停）")
    };
    let _ = tray.set_tooltip(Some(text));
}
