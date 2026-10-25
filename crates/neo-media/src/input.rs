//! How ffmpeg and ffprobe are told where to read from.

use std::{ffi::OsString, fs::File, io, path::PathBuf, process::Stdio, sync::Arc};

/// Demuxers for self-contained media files. Manifest and playlist formats
/// (dash, hls, concat, image sequences) are left out because they open
/// further files by absolute path, which would escape the library root.
/// The list is inherited by any nested context ffmpeg opens.
const FORMATS: &str = "mp3,wav,w64,flac,mov,ogg,aac,ape,wv,tta,aiff,asf,matroska,avi,mpegts,dsf,caf,amr,ac3,eac3,dts,mpeg,flv";

/// A local media source as ffmpeg sees it.
#[derive(Debug, Clone)]
pub enum Input {
    File(PathBuf),
    /// A file the storage layer already opened. It is handed to the child as
    /// stdin and read as `/dev/fd/0`, which Linux reopens as the same inode
    /// with its own offset. That keeps it seekable without ever resolving
    /// the library path again, so a symlink swapped in meanwhile is never
    /// followed.
    Handle(Arc<File>),
}

impl Input {
    /// Arguments that go before `-i`, then the `-i` value.
    ///
    /// A file is passed as `file:<path>` so a name starting with `-` or
    /// containing `:` cannot be read as an option or another protocol. The
    /// protocol whitelist stops a crafted playlist or concat file inside the
    /// library from making ffmpeg open other files or URLs.
    pub(crate) fn args(&self) -> Vec<OsString> {
        match self {
            Self::Handle(_) => vec![
                "-protocol_whitelist".into(),
                "file".into(),
                "-format_whitelist".into(),
                FORMATS.into(),
                "-i".into(),
                "file:/dev/fd/0".into(),
            ],
            Self::File(path) => {
                // Absolute, because the HLS job runs ffmpeg in its own
                // output directory and a relative path would point there.
                let path = std::path::absolute(path).unwrap_or_else(|_| path.clone());
                let mut target = OsString::from("file:");
                target.push(path.as_os_str());
                vec![
                    "-protocol_whitelist".into(),
                    "file".into(),
                    "-format_whitelist".into(),
                    FORMATS.into(),
                    "-i".into(),
                    target,
                ]
            }
        }
    }
}

impl Input {
    /// The child's stdin: the open file for [`Input::Handle`], else nothing.
    ///
    /// # Errors
    /// The handle cannot be duplicated.
    pub(crate) fn stdin(&self) -> io::Result<Stdio> {
        match self {
            Self::Handle(file) => Ok(Stdio::from(file.try_clone()?)),
            Self::File(_) => Ok(Stdio::null()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_get_the_file_protocol_and_whitelist() {
        let args = Input::File("-rf:odd.mp3".into()).args();
        assert_eq!(args[..2], ["-protocol_whitelist", "file"]);
        assert_eq!(args[2], "-format_whitelist");
        assert_eq!(args[4], "-i");
        let absolute = std::path::absolute("-rf:odd.mp3").unwrap();
        assert_eq!(args[5], format!("file:{}", absolute.display()).as_str());
    }
}
