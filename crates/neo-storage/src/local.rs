//! The local backend from book/src/storage.md: paths resolve under the root,
//! symlinks are never followed, and a root that is not there is
//! `Unavailable`, not an empty library.
//!
//! On Linux every path is opened with `openat2` and `RESOLVE_BENEATH |
//! RESOLVE_NO_SYMLINKS` relative to a handle on the root, and files are
//! handed out as open handles. Checking a path and then opening it by name
//! would let a directory swapped for a symlink in between lead outside the
//! root, so the kernel enforces both rules during the lookup itself. Other
//! Unix systems lack `openat2`, and a Linux sandbox may refuse it; there
//! the path is opened one component at a time with `O_NOFOLLOW`, each
//! relative to the directory opened before, which gives the same
//! guarantees because a `RawPath` has no `..`.

use std::{
    ffi::OsStr,
    fs::File,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::Arc,
};

#[cfg(target_os = "linux")]
use rustix::{fd::AsRawFd, fs::ResolveFlags};
use rustix::{
    fd::OwnedFd,
    fs::{self as rfs, AtFlags, Dir, FileType, Mode, OFlags, Stat},
    io::Errno,
};

use crate::{Access, Entry, EntryKind, LocalFile, RawPath, Source, Storage, StorageError};

/// No symlink anywhere in the path, and no way out of the root. Magic
/// links under /proc are symlinks too, so NO_SYMLINKS covers them.
#[cfg(target_os = "linux")]
const RESOLVE: ResolveFlags = ResolveFlags::BENEATH.union(ResolveFlags::NO_SYMLINKS);

/// A handle on a directory to look entries up in. Linux has `O_PATH`, which
/// needs no read permission and has no open-time side effects.
#[cfg(target_os = "linux")]
const DIR_HANDLE: OFlags = OFlags::DIRECTORY.union(OFlags::PATH);
#[cfg(not(target_os = "linux"))]
const DIR_HANDLE: OFlags = OFlags::DIRECTORY.union(OFlags::RDONLY);

/// Opens `rel` below `root` one component at a time, each with
/// `O_NOFOLLOW` and relative to the directory opened before. A symlink at
/// any step fails the open, and a `RawPath` has no `..` to climb with, so
/// the result is always inside the root. Swapping a directory for a
/// symlink meanwhile does not help, because each step uses the directory
/// handle already open rather than a path.
fn walk_beneath(root: &OwnedFd, rel: &RawPath, flags: OFlags) -> Result<OwnedFd, Errno> {
    use rustix::fd::AsFd;
    let last = OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut parts = rel.components().peekable();
    let Some(mut name) = parts.next() else {
        return rfs::openat(root, ".", flags | last, Mode::empty());
    };
    let mut dir: Option<OwnedFd> = None;
    loop {
        let at = dir.as_ref().map_or(root.as_fd(), |d| d.as_fd());
        let name_os = OsStr::from_bytes(name);
        match parts.next() {
            None => return rfs::openat(at, name_os, flags | last, Mode::empty()),
            Some(next) => {
                let opened = rfs::openat(at, name_os, DIR_HANDLE | last, Mode::empty())?;
                dir = Some(opened);
                name = next;
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct LocalStorage {
    root: PathBuf,
}

impl LocalStorage {
    /// `root` must be absolute. It is not checked here, because a root that
    /// is missing at startup (an unmounted disk) may appear later. The root
    /// path itself is trusted config, so a symlink there is followed.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn open_root(&self) -> Result<OwnedFd, StorageError> {
        rfs::open(
            &self.root,
            OFlags::DIRECTORY | OFlags::RDONLY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|e| StorageError::Unavailable(format!("{}: {}", self.root.display(), e)))
    }

    /// Opens `rel` below the root. The empty path is the root itself.
    #[cfg(target_os = "linux")]
    fn open_beneath(
        &self,
        root: &OwnedFd,
        rel: &RawPath,
        flags: OFlags,
    ) -> Result<OwnedFd, StorageError> {
        let path: &OsStr = if rel.is_root() {
            OsStr::new(".")
        } else {
            OsStr::from_bytes(rel.as_bytes())
        };
        match rfs::openat2(root, path, flags | OFlags::CLOEXEC, Mode::empty(), RESOLVE) {
            // systemd's RestrictSUIDSGID= cannot inspect the flags openat2
            // takes, so it fails every call with ENOSYS, as kernels before
            // 5.6 do.
            Err(Errno::NOSYS) => walk_beneath(root, rel, flags),
            opened => opened,
        }
        .map_err(|e| self.errno(rel, e))
    }

    #[cfg(not(target_os = "linux"))]
    fn open_beneath(
        &self,
        root: &OwnedFd,
        rel: &RawPath,
        flags: OFlags,
    ) -> Result<OwnedFd, StorageError> {
        walk_beneath(root, rel, flags).map_err(|e| self.errno(rel, e))
    }

    /// Only "does not exist" is evidence that something is missing, and
    /// only while the root itself is still there: an unmounted disk makes
    /// every path vanish at once. A symlink on the way (ELOOP) or a path
    /// that would leave the root (EXDEV) also does not exist as far as the
    /// library is concerned. Anything else, permissions included, means the
    /// backend could not answer.
    fn errno(&self, rel: &RawPath, e: Errno) -> StorageError {
        match e {
            Errno::NOENT | Errno::NOTDIR | Errno::LOOP | Errno::XDEV => {
                match self
                    .open_root()
                    .and_then(|root| self.check_populated(&root))
                {
                    Ok(()) => StorageError::NotFound,
                    Err(unavailable) => unavailable,
                }
            }
            _ => StorageError::Unavailable(format!("{}: {e}", rel.to_string_lossy())),
        }
    }

    fn list_blocking(&self, dir: &RawPath) -> Result<Vec<Entry>, StorageError> {
        let root = self.open_root()?;
        let fd = self.open_beneath(&root, dir, OFlags::DIRECTORY | OFlags::RDONLY)?;
        let mut reader = Dir::read_from(&fd).map_err(|e| self.errno(dir, e))?;
        let mut entries = Vec::new();
        while let Some(item) = reader.read() {
            let item = item.map_err(|e| self.errno(dir, e))?;
            let name = item.file_name().to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            // Relative to the open directory and without following a
            // symlink, so the entry is the one that was listed. A child that
            // vanished meanwhile makes the listing incomplete, not the
            // directory missing.
            let stat =
                rfs::statat(&fd, item.file_name(), AtFlags::SYMLINK_NOFOLLOW).map_err(|e| {
                    StorageError::Unavailable(format!(
                        "{}/{}: {e}",
                        dir.to_string_lossy(),
                        String::from_utf8_lossy(name)
                    ))
                })?;
            entries.push(entry(name.to_vec(), &stat));
        }
        if dir.is_root() && entries.is_empty() {
            return Err(self.empty_root());
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    /// When the root is a mount point, unmounting the disk leaves an empty
    /// directory behind, which would otherwise read as a library whose
    /// works were all deleted. A library in use is never empty, so an empty
    /// root counts as unavailable, and nothing below it counts as missing.
    fn check_populated(&self, root: &OwnedFd) -> Result<(), StorageError> {
        let mut reader = Dir::read_from(root)
            .map_err(|e| StorageError::Unavailable(format!("{}: {e}", self.root.display())))?;
        while let Some(item) = reader.read() {
            let item = item
                .map_err(|e| StorageError::Unavailable(format!("{}: {e}", self.root.display())))?;
            let name = item.file_name().to_bytes();
            if name != b"." && name != b".." {
                return Ok(());
            }
        }
        Err(self.empty_root())
    }

    fn empty_root(&self) -> StorageError {
        StorageError::Unavailable(format!(
            "{} is empty; is the disk mounted?",
            self.root.display()
        ))
    }

    fn stat_blocking(&self, rel: &RawPath) -> Result<Entry, StorageError> {
        let root = self.open_root()?;
        let (Some(parent), Some(name)) = (rel.parent(), rel.file_name()) else {
            let stat = rfs::fstat(&root).map_err(|e| self.errno(rel, e))?;
            return Ok(entry(Vec::new(), &stat));
        };
        let dir = self.open_beneath(&root, &parent, DIR_HANDLE)?;
        let stat = rfs::statat(&dir, OsStr::from_bytes(name), AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|e| self.errno(rel, e))?;
        Ok(entry(name.to_vec(), &stat))
    }

    #[cfg(target_os = "linux")]
    fn file_blocking(&self, rel: &RawPath) -> Result<LocalFile, StorageError> {
        let root = self.open_root()?;
        // An O_PATH handle has no open-time side effects, which a FIFO or a
        // device would have, so the type is checked before reading.
        let handle = self.open_beneath(&root, rel, OFlags::PATH)?;
        let stat = rfs::fstat(&handle).map_err(|e| self.errno(rel, e))?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
            return Err(StorageError::NotFound);
        }
        // Reopening through the handle's /proc entry gives a readable
        // descriptor for the same inode, with no path lookup to race.
        let fd = rfs::open(
            format!("/proc/self/fd/{}", handle.as_raw_fd()),
            OFlags::RDONLY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|e| {
            StorageError::Unavailable(format!("{}: reopen: {e}", rel.to_string_lossy()))
        })?;
        let meta = entry(Vec::new(), &stat);
        Ok(LocalFile {
            file: Arc::new(File::from(fd)),
            path: self.root.join(OsStr::from_bytes(rel.as_bytes())),
            size: meta.size.unwrap_or(0),
            mtime_ms: meta.mtime_ms,
        })
    }

    /// Without `O_PATH`, the type is read with `statat` first, and the file
    /// is opened non-blocking so that a FIFO swapped in meanwhile cannot
    /// hang the open. The open file must be the inode that was checked.
    #[cfg(not(target_os = "linux"))]
    fn file_blocking(&self, rel: &RawPath) -> Result<LocalFile, StorageError> {
        self.file_by_walking(rel)
    }

    #[cfg(any(not(target_os = "linux"), test))]
    fn file_by_walking(&self, rel: &RawPath) -> Result<LocalFile, StorageError> {
        let root = self.open_root()?;
        let (Some(parent), Some(name)) = (rel.parent(), rel.file_name()) else {
            return Err(StorageError::NotFound);
        };
        let dir = walk_beneath(&root, &parent, OFlags::DIRECTORY | OFlags::RDONLY)
            .map_err(|e| self.errno(rel, e))?;
        let name = OsStr::from_bytes(name);
        let checked =
            rfs::statat(&dir, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|e| self.errno(rel, e))?;
        if FileType::from_raw_mode(checked.st_mode) != FileType::RegularFile {
            return Err(StorageError::NotFound);
        }
        let fd = rfs::openat(
            &dir,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|e| self.errno(rel, e))?;
        let stat = rfs::fstat(&fd).map_err(|e| self.errno(rel, e))?;
        if (stat.st_dev, stat.st_ino) != (checked.st_dev, checked.st_ino) {
            return Err(StorageError::Unavailable(format!(
                "{} changed while it was opened",
                rel.to_string_lossy()
            )));
        }
        let meta = entry(Vec::new(), &stat);
        Ok(LocalFile {
            file: Arc::new(File::from(fd)),
            path: self.root.join(OsStr::from_bytes(rel.as_bytes())),
            size: meta.size.unwrap_or(0),
            mtime_ms: meta.mtime_ms,
        })
    }

    async fn blocking<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Self) -> Result<T, StorageError> + Send + 'static,
    ) -> Result<T, StorageError> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || f(&this))
            .await
            .map_err(|e| StorageError::Unavailable(format!("blocking task failed: {e}")))?
    }
}

#[async_trait::async_trait]
impl Storage for LocalStorage {
    async fn list(&self, dir: &RawPath) -> Result<Vec<Entry>, StorageError> {
        let dir = dir.clone();
        self.blocking(move |s| s.list_blocking(&dir)).await
    }

    async fn stat(&self, path: &RawPath) -> Result<Entry, StorageError> {
        let path = path.clone();
        self.blocking(move |s| s.stat_blocking(&path)).await
    }

    async fn source(&self, path: &RawPath) -> Result<Source, StorageError> {
        let path = path.clone();
        self.blocking(move |s| s.file_blocking(&path))
            .await
            .map(Source::LocalFile)
    }

    async fn access(&self, path: &RawPath) -> Result<Access, StorageError> {
        let path = path.clone();
        self.blocking(move |s| s.file_blocking(&path))
            .await
            .map(Access::ServeLocal)
    }
}

fn entry(name: Vec<u8>, stat: &Stat) -> Entry {
    let kind = match FileType::from_raw_mode(stat.st_mode) {
        FileType::RegularFile => EntryKind::File,
        FileType::Directory => EntryKind::Dir,
        FileType::Symlink => EntryKind::Symlink,
        _ => EntryKind::Other,
    };
    let mtime_ms = (stat.st_mtime >= 0).then(|| {
        stat.st_mtime.saturating_mul(1000) + i64::from(stat.st_mtime_nsec as u32 / 1_000_000)
    });
    Entry {
        name,
        kind,
        size: (kind == EntryKind::File).then(|| u64::try_from(stat.st_size).unwrap_or(0)),
        mtime_ms,
        etag: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The component walk other Unix systems use, run here on Linux.
    #[test]
    fn walking_beneath_refuses_symlinks_at_every_step() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        std::fs::write(root.join("a/b/f.mp3"), b"x").unwrap();
        std::fs::write(tmp.path().join("outside"), b"secret").unwrap();
        std::os::unix::fs::symlink(root.join("a/b"), root.join("a/link")).unwrap();
        std::os::unix::fs::symlink(tmp.path().join("outside"), root.join("a/b/out")).unwrap();
        std::os::unix::fs::symlink(tmp.path(), root.join("up")).unwrap();
        let fd = rfs::open(&root, OFlags::DIRECTORY | OFlags::RDONLY, Mode::empty()).unwrap();
        let open = |p: &str| walk_beneath(&fd, &RawPath::new(p).unwrap(), OFlags::RDONLY);

        assert!(open("a/b/f.mp3").is_ok());
        assert!(open("").is_ok());
        for refused in ["a/link/f.mp3", "a/b/out", "up/outside", "a/link"] {
            assert!(
                matches!(open(refused), Err(Errno::LOOP | Errno::NOTDIR)),
                "{refused}"
            );
        }
        assert_eq!(open("a/missing").err(), Some(Errno::NOENT));
    }

    /// The file opening other Unix systems use, run here on Linux.
    #[test]
    fn files_opened_by_walking_are_regular_files_inside_the_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        std::fs::create_dir_all(root.join("w")).unwrap();
        std::fs::write(root.join("w/a.mp3"), b"audio").unwrap();
        std::fs::write(tmp.path().join("outside"), b"secret").unwrap();
        std::os::unix::fs::symlink(tmp.path().join("outside"), root.join("w/out.mp3")).unwrap();
        rustix::fs::mkfifoat(
            rustix::fs::CWD,
            root.join("w/fifo"),
            Mode::RUSR | Mode::WUSR,
        )
        .unwrap();
        let storage = LocalStorage::new(&root);
        let open = |p: &str| storage.file_by_walking(&RawPath::new(p).unwrap());

        let file = open("w/a.mp3").unwrap();
        assert_eq!(file.size, 5);
        let mut read = String::new();
        std::io::Read::read_to_string(&mut &*file.file, &mut read).unwrap();
        assert_eq!(read, "audio");
        for refused in ["w/out.mp3", "w/fifo", "w", "w/none.mp3"] {
            assert!(
                matches!(open(refused), Err(StorageError::NotFound)),
                "{refused}"
            );
        }
    }

    /// Makes `openat2` fail with ENOSYS on the calling thread, which is
    /// what systemd's RestrictSUIDSGID= does to a whole service.
    #[cfg(target_os = "linux")]
    fn refuse_openat2_on_this_thread() {
        use libc::{BPF_ABS, BPF_JEQ, BPF_JMP, BPF_K, BPF_LD, BPF_RET, BPF_W, c_ulong};
        let op = |code: u32, k: u32, jt: u8, jf: u8| libc::sock_filter {
            code: code as u16,
            jt,
            jf,
            k,
        };
        let enosys = libc::SECCOMP_RET_ERRNO | libc::ENOSYS as u32;
        let mut filter = [
            // The syscall number is the first field of seccomp_data.
            op(BPF_LD | BPF_W | BPF_ABS, 0, 0, 0),
            op(BPF_JMP | BPF_JEQ | BPF_K, libc::SYS_openat2 as u32, 0, 1),
            op(BPF_RET | BPF_K, enosys, 0, 0),
            op(BPF_RET | BPF_K, libc::SECCOMP_RET_ALLOW, 0, 0),
        ];
        let prog = libc::sock_fprog {
            len: filter.len() as u16,
            filter: filter.as_mut_ptr(),
        };
        let (one, zero): (c_ulong, c_ulong) = (1, 0);
        let mode = libc::SECCOMP_MODE_FILTER as c_ulong;
        // SAFETY: prctl reads `prog` and `filter` only during the call. The
        // arguments are c_ulong because that is the width the kernel reads.
        unsafe {
            let no_new_privs = libc::prctl(libc::PR_SET_NO_NEW_PRIVS, one, zero, zero, zero);
            assert_eq!(no_new_privs, 0);
            let seccomp = libc::prctl(libc::PR_SET_SECCOMP, mode, &raw const prog);
            assert_eq!(seccomp, 0);
        }
    }

    /// Under a sandbox that refuses `openat2`, paths are opened by walking
    /// and the backend still answers, without following symlinks.
    #[cfg(target_os = "linux")]
    #[test]
    fn opens_by_walking_when_openat2_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        std::fs::create_dir_all(root.join("w/sub")).unwrap();
        std::fs::write(root.join("w/a.mp3"), b"audio").unwrap();
        std::fs::write(tmp.path().join("outside"), b"secret").unwrap();
        std::os::unix::fs::symlink(tmp.path().join("outside"), root.join("w/out.mp3")).unwrap();
        std::os::unix::fs::symlink(root.join("w"), root.join("link")).unwrap();
        let storage = LocalStorage::new(&root);
        let p = |s: &str| RawPath::new(s).unwrap();

        // The filter stays on the thread that installed it, so the checks
        // get a thread of their own and call the blocking methods directly.
        std::thread::spawn(move || {
            refuse_openat2_on_this_thread();
            let fd = rfs::open(&root, OFlags::DIRECTORY | OFlags::RDONLY, Mode::empty()).unwrap();
            let refused = rfs::openat2(&fd, ".", OFlags::PATH, Mode::empty(), RESOLVE);
            assert_eq!(refused.err(), Some(Errno::NOSYS));

            let names: Vec<_> = storage
                .list_blocking(&p("w"))
                .unwrap()
                .into_iter()
                .map(|e| e.name)
                .collect();
            assert_eq!(names, [&b"a.mp3"[..], b"out.mp3", b"sub"]);
            assert_eq!(storage.stat_blocking(&p("w/a.mp3")).unwrap().size, Some(5));
            let file = storage.file_blocking(&p("w/a.mp3")).unwrap();
            let mut read = String::new();
            std::io::Read::read_to_string(&mut &*file.file, &mut read).unwrap();
            assert_eq!(read, "audio");

            for refused in ["w/out.mp3", "link/a.mp3", "link", "w/sub"] {
                assert!(
                    matches!(
                        storage.file_blocking(&p(refused)),
                        Err(StorageError::NotFound)
                    ),
                    "{refused}"
                );
            }
            assert!(matches!(
                storage.list_blocking(&p("link")),
                Err(StorageError::NotFound)
            ));
            assert!(matches!(
                storage.stat_blocking(&p("link/a.mp3")),
                Err(StorageError::NotFound)
            ));
        })
        .join()
        .unwrap();
    }
}
