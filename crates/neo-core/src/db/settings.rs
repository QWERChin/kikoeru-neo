//! Per-user player settings (`user_settings`, book/src/data-model.md).

use crate::dto::{Accent, UserSettings};

/// Stored settings. A user without a row gets [`Settings::default`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub normalize_loudness: bool,
    pub target_lufs: f64,
    pub accent: Option<Accent>,
    pub seek_back_seconds: u32,
    pub seek_forward_seconds: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            normalize_loudness: false,
            target_lufs: -16.0,
            accent: None,
            seek_back_seconds: UserSettings::DEFAULT_SEEK_BACK,
            seek_forward_seconds: UserSettings::DEFAULT_SEEK_FORWARD,
        }
    }
}
