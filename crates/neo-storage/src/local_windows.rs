//! The local backend on Windows, for the desktop app. Windows has no
//! `openat`, so the rules of book/src/storage.md are kept in two steps.
//! Each component is first looked at with `symlink_metadata`, so a symlink
//! or junction anywhere in the path is refused. Then the file is opened
//! without following a reparse point in its last component, and the path
//! Windows reports for the open handle must lie under the root. That last
//! check is made on the handle, so a directory swapped for a junction
//! between the two steps is caught too, and directories are listed from
//! their handle rather than by path.

use std::{
    fs::{self, File, Metadata},
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::UNIX_EPOCH,
};

use crate::{Access, Entry, EntryKind, LocalFile, RawPath, Source, Storage, StorageError};

#[derive(Debug, Clone)]
pub struct LocalStorage {
    root: PathBuf,
}

impl LocalStorage {
    /// `root` must be absolute. A root that is missing now may appear later.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn canonical_root(&self) -> Result<PathBuf, StorageError> {
        fs::canonicalize(&self.root)
            .map_err(|e| StorageError::Unavailable(format!("{}: {e}", self.root.display())))
    }

    /// The path of `rel` under the root, refusing names Windows would read
    /// as separators or drive prefixes, and any symlink or junction on the
    /// way.
    fn resolve(&self, rel: &RawPath) -> Result<PathBuf, StorageError> {
        let mut path = self.canonical_root()?;
        for name in rel.components() {
            if name.iter().any(|b| matches!(b, b'\\' | b':')) {
                return Err(StorageError::NotFound);
            }
            // Windows names are UTF-16, listed here as UTF-8. Bytes that are
            // not UTF-8 never named a file on this system.
            let Ok(name) = std::str::from_utf8(name) else {
                return Err(StorageError::NotFound);
            };
            path.push(name);
            let meta = fs::symlink_metadata(&path).map_err(|e| self.io_error(rel, e))?;
            if meta.file_type().is_symlink() || is_reparse_point(&meta) {
                return Err(StorageError::NotFound);
            }
        }
        Ok(path)
    }

    /// As on Unix, only "does not exist" with the root still populated
    /// counts as missing.
    fn io_error(&self, rel: &RawPath, e: io::Error) -> StorageError {
        match e.kind() {
            io::ErrorKind::NotFound | io::ErrorKind::NotADirectory => {
                match self.check_populated() {
                    Ok(()) => StorageError::NotFound,
                    Err(unavailable) => unavailable,
                }
            }
            _ => StorageError::Unavailable(format!("{}: {e}", rel.to_string_lossy())),
        }
    }

    fn check_populated(&self) -> Result<(), StorageError> {
        let mut entries = fs::read_dir(&self.root)
            .map_err(|e| StorageError::Unavailable(format!("{}: {e}", self.root.display())))?;
        match entries.next() {
            Some(Ok(_)) => Ok(()),
            Some(Err(e)) => Err(StorageError::Unavailable(format!(
                "{}: {e}",
                self.root.display()
            ))),
            None => Err(self.empty_root()),
        }
    }

    fn empty_root(&self) -> StorageError {
        StorageError::Unavailable(format!(
            "{} is empty; is the disk connected?",
            self.root.display()
        ))
    }

    /// Opens `rel` without following a reparse point in its last
    /// component, as a directory too, and checks that the path Windows
    /// reports for the handle lies under the root. Everything after this
    /// works on the handle, so a directory swapped for a junction after the
    /// path checks in `resolve` is caught here.
    fn open_checked(&self, rel: &RawPath) -> Result<(File, Metadata), StorageError> {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        };
        let path = self.resolve(rel)?;
        let handle = fs::OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .open(&path)
            .map_err(|e| self.io_error(rel, e))?;
        let meta = handle.metadata().map_err(|e| self.io_error(rel, e))?;
        if is_reparse_point(&meta) {
            return Err(StorageError::NotFound);
        }
        let opened = final_path(&handle).map_err(|e| self.io_error(rel, e))?;
        if !opened.starts_with(self.canonical_root()?) {
            return Err(StorageError::NotFound);
        }
        Ok((handle, meta))
    }

    fn list_blocking(&self, dir: &RawPath) -> Result<Vec<Entry>, StorageError> {
        let (handle, meta) = self.open_checked(dir)?;
        if !meta.is_dir() {
            return Err(StorageError::NotFound);
        }
        let mut entries = read_dir_handle(&handle)
            .map_err(|e| StorageError::Unavailable(format!("{}: {e}", dir.to_string_lossy())))?;
        if dir.is_root() && entries.is_empty() {
            return Err(self.empty_root());
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    fn stat_blocking(&self, rel: &RawPath) -> Result<Entry, StorageError> {
        let (_, meta) = self.open_checked(rel)?;
        Ok(entry(rel.file_name().unwrap_or_default().to_vec(), &meta))
    }

    fn file_blocking(&self, rel: &RawPath) -> Result<LocalFile, StorageError> {
        let (file, meta) = self.open_checked(rel)?;
        if !meta.is_file() {
            return Err(StorageError::NotFound);
        }
        let found = entry(Vec::new(), &meta);
        Ok(LocalFile {
            file: Arc::new(file),
            path: self.resolve(rel)?,
            size: found.size.unwrap_or(0),
            mtime_ms: found.mtime_ms,
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

/// The entries of an open directory, read from the handle itself, so no
/// name is looked up again and no junction is followed.
fn read_dir_handle(dir: &File) -> io::Result<Vec<Entry>> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{
        Foundation::ERROR_NO_MORE_FILES,
        Storage::FileSystem::{
            FILE_ID_BOTH_DIR_INFO, FileIdBothDirectoryInfo, FileIdBothDirectoryRestartInfo,
            GetFileInformationByHandleEx,
        },
    };
    const DIRECTORY: u32 = 0x10;
    const REPARSE_POINT: u32 = 0x400;
    // 100 ns ticks between 1601-01-01 and 1970-01-01.
    const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;
    // u64 words keep the records 8-byte aligned, as the struct needs.
    let mut buf = vec![0u64; 8 * 1024];
    let mut class = FileIdBothDirectoryRestartInfo;
    let mut entries = Vec::new();
    loop {
        // SAFETY: the handle is open, and the buffer and its byte length
        // match.
        let ok = unsafe {
            GetFileInformationByHandleEx(
                dir.as_raw_handle() as _,
                class,
                buf.as_mut_ptr().cast(),
                (buf.len() * 8) as u32,
            )
        };
        if ok == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                return Ok(entries);
            }
            return Err(error);
        }
        class = FileIdBothDirectoryInfo;
        let base = buf.as_ptr().cast::<u8>();
        let mut offset = 0usize;
        loop {
            // SAFETY: Windows wrote whole records into the buffer, each at
            // an 8-byte aligned offset given by the one before.
            let info = unsafe { &*base.add(offset).cast::<FILE_ID_BOTH_DIR_INFO>() };
            let name_ptr = std::ptr::addr_of!(info.FileName).cast::<u16>();
            // SAFETY: FileNameLength is the name's length in bytes, inside
            // the record.
            let name =
                unsafe { std::slice::from_raw_parts(name_ptr, info.FileNameLength as usize / 2) };
            // A name with an unpaired surrogate has no UTF-8 form, so it
            // could never be opened again by name. It is listed as a special
            // entry, which the scanner skips, rather than under a lossy name
            // that might open a different file.
            let (name, representable) = match String::from_utf16(name) {
                Ok(name) => (name, true),
                Err(_) => (String::from_utf16_lossy(name), false),
            };
            if name != "." && name != ".." {
                let attrs = info.FileAttributes;
                let kind = if !representable {
                    EntryKind::Other
                } else if attrs & REPARSE_POINT != 0 {
                    EntryKind::Symlink
                } else if attrs & DIRECTORY != 0 {
                    EntryKind::Dir
                } else {
                    EntryKind::File
                };
                let mtime_ms = (info.LastWriteTime >= UNIX_EPOCH_TICKS)
                    .then(|| (info.LastWriteTime - UNIX_EPOCH_TICKS) / 10_000);
                entries.push(Entry {
                    name: name.into_bytes(),
                    kind,
                    size: (kind == EntryKind::File).then(|| info.EndOfFile.max(0) as u64),
                    mtime_ms,
                    etag: None,
                });
            }
            if info.NextEntryOffset == 0 {
                break;
            }
            offset += info.NextEntryOffset as usize;
        }
    }
}

/// The path of an open file as Windows resolves it, in the same `\\?\`
/// form `fs::canonicalize` returns for the root.
fn final_path(file: &File) -> io::Result<PathBuf> {
    use std::os::windows::{ffi::OsStringExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_NAME_NORMALIZED, GetFinalPathNameByHandleW, VOLUME_NAME_DOS,
    };
    let mut buf = vec![0u16; 512];
    loop {
        // SAFETY: the handle is open for the duration of the call, and the
        // length passed is the buffer's.
        let len = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle() as _,
                buf.as_mut_ptr(),
                buf.len() as u32,
                FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
            )
        } as usize;
        if len == 0 {
            return Err(io::Error::last_os_error());
        }
        if len < buf.len() {
            return Ok(PathBuf::from(std::ffi::OsString::from_wide(&buf[..len])));
        }
        // Too small: `len` is the size needed, terminator included.
        buf.resize(len, 0);
    }
}

/// Junctions and other reparse points act like symlinks but are not all
/// reported as one.
fn is_reparse_point(meta: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

fn entry(name: Vec<u8>, meta: &Metadata) -> Entry {
    let kind = if meta.file_type().is_symlink() || is_reparse_point(meta) {
        EntryKind::Symlink
    } else if meta.is_file() {
        EntryKind::File
    } else if meta.is_dir() {
        EntryKind::Dir
    } else {
        EntryKind::Other
    };
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_millis()).ok());
    Entry {
        name,
        kind,
        size: (kind == EntryKind::File).then(|| meta.len()),
        mtime_ms,
        etag: None,
    }
}
