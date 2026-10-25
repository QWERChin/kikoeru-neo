//! Files and directories that only the service user may read. On Unix that
//! means modes 0700 and 0600. Windows has no permission bits: files under
//! the user's profile, where the desktop app keeps its data, inherit an
//! ACL that already limits them to that user, so these helpers leave
//! Windows defaults alone.

use std::{
    fs::{DirBuilder, OpenOptions},
    io,
    path::Path,
};

/// A recursive-or-not builder for directories only the owner can enter.
pub fn dir() -> DirBuilder {
    #[allow(unused_mut)]
    let mut builder = DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder
}

pub trait PrivateFile {
    /// New files are created readable by the owner only.
    fn private(&mut self) -> &mut Self;
}

impl PrivateFile for OpenOptions {
    fn private(&mut self) -> &mut Self {
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(self, 0o600);
        self
    }
}

/// Makes an existing file readable by the owner only.
pub fn restrict_file(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

/// The permission bits of `path` without following a symlink, or `None`
/// where the platform has none.
pub fn mode(path: &Path) -> io::Result<Option<u32>> {
    let meta = std::fs::symlink_metadata(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Ok(Some(meta.permissions().mode() & 0o777))
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        Ok(None)
    }
}
