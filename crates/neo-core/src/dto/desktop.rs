//! What the desktop shell's commands and events carry ("外壳给页面的命令"
//! in book/src/desktop.md). They are not HTTP types, but the bundled
//! frontend reads them the same way, so they are exported with ts-rs too.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum DesktopMode {
    Local,
    Server,
}

/// The answer of `desktop_state`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct DesktopState {
    /// `null` until a mode is chosen, and after switching mode.
    pub mode: Option<DesktopMode>,
    pub server_url: Option<String>,
    /// Why the saved mode could not be opened at startup.
    pub last_error: Option<String>,
}

/// The payload of the `media-control` event, from the tray menu and the
/// system media keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
pub enum MediaControl {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
}
