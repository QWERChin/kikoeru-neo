//! The shell's own settings: which mode, and which server. They live in
//! the app config directory, apart from the data directory, which the
//! connect mode does not have.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use neo_core::dto::DesktopMode as Mode;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopConfig {
    pub mode: Option<Mode>,
    pub server_url: Option<String>,
}

pub fn path(config_dir: &Path) -> PathBuf {
    config_dir.join("desktop.json")
}

impl DesktopConfig {
    /// A missing or unreadable file means nothing was chosen yet, so the
    /// picker shows again rather than the app failing to start.
    pub fn load(file: &Path) -> Self {
        std::fs::read(file)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Written to a temporary file and renamed, so a crash never leaves a
    /// half-written file.
    pub fn save(&self, file: &Path) -> std::io::Result<()> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = file.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, file)
    }
}

/// A server address the user typed, reduced to its origin and path. Only
/// http and https are accepted.
pub fn server_url(input: &str) -> Result<url::Url, String> {
    let url = url::Url::parse(input.trim()).map_err(|_| format!("不是有效的地址：{input}"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return Err("只支持 http 和 https 地址".to_owned());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("地址里不能带用户名或密码".to_owned());
    }
    let mut url = url;
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_survive_a_bad_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = path(dir.path());
        assert_eq!(DesktopConfig::load(&file), DesktopConfig::default());
        let chosen = DesktopConfig {
            mode: Some(Mode::Server),
            server_url: Some("https://kikoeru.example.lan/".into()),
        };
        chosen.save(&file).unwrap();
        assert_eq!(DesktopConfig::load(&file), chosen);
        std::fs::write(&file, b"{not json").unwrap();
        assert_eq!(DesktopConfig::load(&file), DesktopConfig::default());
    }

    #[test]
    fn only_plain_http_server_addresses_are_accepted() {
        assert_eq!(
            server_url(" https://kikoeru.example.lan/neo?x=1#y ")
                .unwrap()
                .as_str(),
            "https://kikoeru.example.lan/neo"
        );
        for bad in [
            "ftp://x",
            "file:///etc",
            "javascript:alert(1)",
            "https://u:p@x",
            "nonsense",
        ] {
            assert!(server_url(bad).is_err(), "{bad}");
        }
    }
}
