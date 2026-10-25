//! The ffmpeg and ffprobe bundled through Tauri's `externalBin`
//! (`tauri.bundle.conf.json`, used only by the release workflow). Tauri
//! installs them next to the app's executable without the target-triple
//! suffix they carry in the source tree. Builds without them leave the
//! backend to search `PATH`, where the Nix package's wrapper puts
//! nixpkgs' ffmpeg-headless.

use std::path::{Path, PathBuf};

pub fn bundled(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    next_to(exe.parent()?, name)
}

fn next_to(dir: &Path, name: &str) -> Option<PathBuf> {
    let path = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    path.is_file().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bundled_tool_is_found_only_when_it_exists() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(next_to(dir.path(), "ffmpeg"), None);
        let file = dir
            .path()
            .join(format!("ffmpeg{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&file, b"").unwrap();
        assert_eq!(next_to(dir.path(), "ffmpeg"), Some(file));
    }
}
