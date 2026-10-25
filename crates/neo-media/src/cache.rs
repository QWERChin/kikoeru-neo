//! The transcode cache under `cache/transcode/`, with keys that change
//! whenever the source does.
//!
//! A cached file's modification time is when it was published, and its
//! access time is set explicitly on every use, because many filesystems are
//! mounted with `noatime` or `relatime`. Eviction goes by access time.

use std::{
    collections::HashMap,
    fs::{self, FileTimes},
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

use sha2::{Digest, Sha256};

use crate::MediaError;

/// Bump when the transcode parameters or output format change, so old
/// files stop matching.
const FORMAT_VERSION: u8 = 1;
/// Remote files without a reliable modification time could change without
/// the key changing, so their cache entries expire.
const UNKNOWN_MTIME_TTL: Duration = Duration::from_secs(7 * 24 * 3600);

/// Everything a transcode result depends on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKey {
    pub root_id: String,
    /// Distinguishes sources with matching paths, sizes and timestamps.
    pub source_version: i64,
    /// The track's raw path relative to the media library root, not to
    /// its work directory: two works can each hold an `01.wav` of the same
    /// size and time.
    pub path: Vec<u8>,
    pub size: u64,
    /// Unix milliseconds, `None` when the storage cannot tell.
    pub mtime: Option<i64>,
    /// The processing, such as `aac-192k`.
    pub params: &'static str,
}

impl CacheKey {
    /// Also the task `dedupe_key`, so one transcode per key runs at a time.
    pub fn digest(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update([FORMAT_VERSION]);
        for field in [self.root_id.as_bytes(), &self.path, self.params.as_bytes()] {
            hasher.update((field.len() as u64).to_le_bytes());
            hasher.update(field);
        }
        hasher.update(self.size.to_le_bytes());
        hasher.update(self.source_version.to_le_bytes());
        match self.mtime {
            Some(mtime) => {
                hasher.update([1]);
                hasher.update(mtime.to_le_bytes());
            }
            None => hasher.update([0]),
        }
        hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// An HLS result is a directory of a playlist and its segments, the
    /// AAC fallback a single file.
    fn file_name(&self) -> String {
        let ext = if self.params.starts_with("hls") {
            "hls"
        } else if self.params.starts_with("waveform") {
            "wave"
        } else {
            "m4a"
        };
        format!("{}.{ext}", self.digest())
    }
}

/// A cached file, or an HLS directory, that eviction leaves alone until
/// this is dropped.
#[derive(Debug)]
pub struct CachedFile {
    pub path: PathBuf,
    /// For a directory, the total length of the files in it.
    pub len: u64,
    _pin: Pin,
}

/// An open segment of a published HLS set. The set stays out of eviction
/// until this is dropped.
#[derive(Debug)]
pub struct HlsSegment {
    pub file: fs::File,
    pub len: u64,
    pub modified: Option<SystemTime>,
    _pin: Pin,
}

#[derive(Debug)]
pub(crate) struct Pin {
    pins: Arc<Mutex<HashMap<String, usize>>>,
    name: String,
}

impl Drop for Pin {
    fn drop(&mut self) {
        let mut pins = self.pins.lock().expect("pin mutex poisoned");
        if let Some(count) = pins.get_mut(&self.name) {
            *count -= 1;
            if *count == 0 {
                pins.remove(&self.name);
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct TranscodeCache {
    transcode_dir: PathBuf,
    tmp_dir: PathBuf,
    max_bytes: u64,
    min_free_bytes: u64,
    pins: Arc<Mutex<HashMap<String, usize>>>,
    tools: crate::Tools,
}

impl TranscodeCache {
    /// Opens the cache below `cache_dir` and empties `tmp/`, which only
    /// holds output of processes that are gone. Call once at startup.
    ///
    /// # Errors
    /// The directories cannot be created or cleared.
    pub fn open(cache_dir: &Path, max_bytes: u64, min_free_bytes: u64) -> io::Result<Self> {
        let transcode_dir = cache_dir.join("transcode");
        let tmp_dir = cache_dir.join("tmp");
        create_private_dir(&transcode_dir)?;
        create_private_dir(&tmp_dir)?;
        for entry in fs::read_dir(&tmp_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                fs::remove_dir_all(entry.path())?;
            } else {
                fs::remove_file(entry.path())?;
            }
        }
        Ok(Self {
            transcode_dir,
            tmp_dir,
            max_bytes,
            min_free_bytes,
            pins: Arc::new(Mutex::new(HashMap::new())),
            tools: crate::Tools::default(),
        })
    }

    /// The ffmpeg and ffprobe that generating cache entries runs. The
    /// default finds both on `PATH`.
    #[must_use]
    pub fn with_tools(mut self, tools: crate::Tools) -> Self {
        self.tools = tools;
        self
    }

    pub fn tools(&self) -> &crate::Tools {
        &self.tools
    }

    pub(crate) fn tmp_dir(&self) -> &Path {
        &self.tmp_dir
    }

    /// A fresh path in `cache/tmp/`, which startup empties, for a file the
    /// caller writes and removes itself, such as a remote source downloaded
    /// for ffmpeg.
    pub fn temp_path(&self, label: &str) -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.tmp_dir
            .join(format!("{label}.{}.{n}.part", std::process::id()))
    }

    pub(crate) fn final_path(&self, key: &CacheKey) -> PathBuf {
        self.transcode_dir.join(key.file_name())
    }

    pub(crate) fn pin(&self, key: &CacheKey) -> Pin {
        let name = key.file_name();
        *self
            .pins
            .lock()
            .expect("pin mutex poisoned")
            .entry(name.clone())
            .or_default() += 1;
        Pin {
            pins: Arc::clone(&self.pins),
            name,
        }
    }

    /// Returns the cached result for `key`, recording the access.
    ///
    /// # Errors
    /// The file exists but cannot be read or touched.
    pub fn lookup(&self, key: &CacheKey) -> io::Result<Option<CachedFile>> {
        let pin = self.pin(key);
        let path = self.final_path(key);
        let meta = match fs::metadata(&path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        if key.mtime.is_none() {
            let published = meta.modified()?;
            let age = SystemTime::now()
                .duration_since(published)
                .unwrap_or_default();
            if age >= UNKNOWN_MTIME_TTL {
                // Deleted only when nobody else holds it. Otherwise the new
                // transcode replaces it by rename, which leaves open
                // handles intact.
                let pins = self.pins.lock().expect("pin mutex poisoned");
                if pins.get(&pin.name).copied() == Some(1) {
                    let _ = remove_entry(&path);
                }
                return Ok(None);
            }
        }
        touch(&access_marker(&path, &meta))?;
        Ok(Some(CachedFile {
            len: entry_len(&path, &meta)?,
            path,
            _pin: pin,
        }))
    }

    /// Opens segment `name` of the HLS set with this ID (see
    /// [`Self::hls_set_id`]). The source is not looked at again: the ID
    /// already fixes which file, version, processing and generation the set
    /// came from. `None` when the ID or the segment name is malformed, the
    /// set is gone, or it was regenerated since. The name is checked here
    /// too, so no caller can open the playlist or a path outside the set. A hit counts as use of the set for eviction.
    ///
    /// The set's directory is opened once, and the generation check and the
    /// segment open both go through that handle. A regeneration that swaps
    /// the directory in between therefore cannot hand out a newer
    /// generation's bytes under an old ID.
    ///
    /// # Errors
    /// [`io::ErrorKind::NotFound`] when the set exists but has no such
    /// segment, or any other read failure.
    pub fn open_hls_segment(&self, id: &str, name: &str) -> io::Result<Option<HlsSegment>> {
        let Some((digest, generation)) = id.split_once('-') else {
            return Ok(None);
        };
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            || generation.is_empty()
            || !generation.bytes().all(|b| b.is_ascii_digit())
            || !crate::hls::is_segment_name(name)
        {
            return Ok(None);
        }
        let set = format!("{digest}.hls");
        *self
            .pins
            .lock()
            .expect("pin mutex poisoned")
            .entry(set.clone())
            .or_default() += 1;
        let pin = Pin {
            pins: Arc::clone(&self.pins),
            name: set.clone(),
        };
        let Some((playlist, file)) = open_in_set(&self.transcode_dir.join(&set), generation, name)?
        else {
            return Ok(None);
        };
        let meta = file.metadata()?;
        if !meta.is_file() {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        playlist.set_times(FileTimes::new().set_accessed(SystemTime::now()))?;
        Ok(Some(HlsSegment {
            len: meta.len(),
            modified: meta.modified().ok(),
            file,
            _pin: pin,
        }))
    }

    /// The ID segment URIs name a published HLS set by: the key digest and
    /// the set's generation. A set regenerated under the same key, as one
    /// for a remote file without a modification time is after it expires,
    /// gets a new ID, so playlists given out before never reach it.
    ///
    /// # Errors
    /// The playlist's metadata cannot be read.
    pub fn hls_set_id(&self, key: &CacheKey, cached: &CachedFile) -> io::Result<String> {
        let generation =
            generation_of(&cached.path)?.ok_or_else(|| io::Error::other("no playlist"))?;
        Ok(format!("{}-{generation}", key.digest()))
    }

    pub(crate) fn cached_file(&self, key: &CacheKey, pin: Pin) -> io::Result<CachedFile> {
        let path = self.final_path(key);
        let len = entry_len(&path, &fs::metadata(&path)?)?;
        Ok(CachedFile {
            path,
            len,
            _pin: pin,
        })
    }

    /// Fails when free space on the cache's filesystem is below the minimum.
    ///
    /// # Errors
    /// [`MediaError::InsufficientStorage`], or the filesystem cannot be
    /// queried.
    pub fn check_space(&self) -> Result<(), MediaError> {
        self.check_space_for(0)
    }

    /// Like [`Self::check_space`], with `bytes` about to be written on top.
    ///
    /// # Errors
    /// [`MediaError::InsufficientStorage`] when writing `bytes` would leave
    /// less than `min_free_bytes`.
    pub fn check_space_for(&self, bytes: u64) -> Result<(), MediaError> {
        let Some(free) = free_bytes(&self.transcode_dir)? else {
            return Ok(());
        };
        if free < self.min_free_bytes.saturating_add(bytes) {
            return Err(MediaError::InsufficientStorage);
        }
        Ok(())
    }

    /// Deletes the least recently used files until the cache fits
    /// `max_bytes`. Files in use (pinned) are skipped.
    ///
    /// # Errors
    /// The directory cannot be listed.
    pub fn evict(&self) -> io::Result<()> {
        let mut files = Vec::new();
        let mut total = 0u64;
        for entry in fs::read_dir(&self.transcode_dir)? {
            let entry = entry?;
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() && !meta.is_dir() {
                continue;
            }
            let path = entry.path();
            // Before the size: listing a directory can move its own atime.
            let accessed = fs::metadata(access_marker(&path, &meta))
                .and_then(|m| m.accessed().or_else(|_| m.modified()))
                .unwrap_or(SystemTime::UNIX_EPOCH);
            let Ok(len) = entry_len(&path, &meta) else {
                continue;
            };
            total += len;
            files.push((accessed, entry.file_name(), len));
        }
        if total <= self.max_bytes {
            return Ok(());
        }
        files.sort();
        let pins = self.pins.lock().expect("pin mutex poisoned");
        for (_, name, len) in files {
            if total <= self.max_bytes {
                break;
            }
            if name.to_str().is_some_and(|name| pins.contains_key(name)) {
                continue;
            }
            match remove_entry(&self.transcode_dir.join(&name)) {
                Ok(()) => total -= len,
                Err(error) if error.kind() == io::ErrorKind::NotFound => total -= len,
                Err(error) => tracing::warn!(?name, %error, "cannot evict a cached file"),
            }
        }
        Ok(())
    }
}

/// The file whose access time records use of a cache entry. For an HLS
/// directory it is the playlist inside: the directory's own atime moves
/// whenever it is listed, which eviction itself does to measure it.
fn access_marker(path: &Path, meta: &fs::Metadata) -> PathBuf {
    if meta.is_dir() {
        path.join("index.m3u8")
    } else {
        path.to_owned()
    }
}

/// Opens `index.m3u8` and segment `name` of the HLS set in `dir`, if the
/// playlist was written at `generation`. On Unix both opens go through one
/// handle to the directory, so a regeneration that swaps the directory in
/// between cannot mix generations, and symlinks are never followed.
#[cfg(unix)]
fn open_in_set(
    dir: &Path,
    generation: &str,
    name: &str,
) -> io::Result<Option<(fs::File, fs::File)>> {
    use rustix::fs::{CWD, Mode, OFlags};
    let nofollow = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let dir = match rustix::fs::openat(CWD, dir, nofollow | OFlags::DIRECTORY, Mode::empty()) {
        Ok(dir) => dir,
        Err(rustix::io::Errno::NOENT | rustix::io::Errno::NOTDIR) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let playlist = match rustix::fs::openat(&dir, "index.m3u8", nofollow, Mode::empty()) {
        Ok(fd) => fs::File::from(fd),
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if generation_of_file(&playlist)?.as_deref() != Some(generation) {
        return Ok(None);
    }
    let file = fs::File::from(rustix::fs::openat(&dir, name, nofollow, Mode::empty())?);
    Ok(Some((playlist, file)))
}

/// Records a use of a cache entry. Windows needs write-attributes access to
/// change a time, which `File::open` does not ask for.
fn touch(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    let file = {
        use std::os::windows::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .access_mode(GENERIC_READ | FILE_WRITE_ATTRIBUTES)
            .open(path)?
    };
    #[cfg(not(windows))]
    let file = fs::File::open(path)?;
    file.set_times(FileTimes::new().set_accessed(SystemTime::now()))
}

#[cfg(windows)]
const GENERIC_READ: u32 = 0x8000_0000;
#[cfg(windows)]
const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;

/// Without `openat`, the playlist is opened first and kept open. Windows
/// refuses to rename a directory while a file in it is open, so the set
/// cannot be swapped before the segment is opened either.
#[cfg(not(unix))]
fn open_in_set(
    dir: &Path,
    generation: &str,
    name: &str,
) -> io::Result<Option<(fs::File, fs::File)>> {
    // The no-follow flag below covers only the last path component, so the
    // set's directory itself must not be a symlink or junction either.
    match fs::symlink_metadata(dir) {
        Ok(meta) if meta.is_dir() && !is_reparse_point(&meta) => {}
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    }
    let playlist = match open_no_follow(&dir.join("index.m3u8"), true) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if generation_of_file(&playlist)?.as_deref() != Some(generation) {
        return Ok(None);
    }
    let file = open_no_follow(&dir.join(name), false)?;
    Ok(Some((playlist, file)))
}

/// Opens the reparse point itself rather than its target, the Windows
/// counterpart of `O_NOFOLLOW`. The caller then requires a regular file, so
/// a symlink or junction placed in the cache is never served.
/// `touchable` adds the write-attributes access [`touch`] needs.
#[cfg(windows)]
fn open_no_follow(path: &Path, touchable: bool) -> io::Result<fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    let access = if touchable {
        GENERIC_READ | FILE_WRITE_ATTRIBUTES
    } else {
        GENERIC_READ
    };
    fs::OpenOptions::new()
        .access_mode(access)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(not(any(unix, windows)))]
fn open_no_follow(path: &Path, _touchable: bool) -> io::Result<fs::File> {
    fs::File::open(path)
}

#[cfg(windows)]
fn is_reparse_point(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(any(unix, windows)))]
fn is_reparse_point(meta: &fs::Metadata) -> bool {
    meta.file_type().is_symlink()
}

fn generation_of_file(playlist: &fs::File) -> io::Result<Option<String>> {
    Ok(playlist
        .metadata()?
        .modified()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos().to_string())
        .ok())
}

/// Free bytes on the filesystem holding `path`, for the caller.
#[cfg(unix)]
fn free_bytes(path: &Path) -> io::Result<Option<u64>> {
    let stat = rustix::fs::statvfs(path).map_err(io::Error::from)?;
    Ok(Some(stat.f_bavail.saturating_mul(stat.f_frsize)))
}

/// Not measured yet off Unix: the space check is skipped there, and the
/// desktop build relies on `cache.max_bytes` alone (book/src/desktop.md).
#[cfg(not(unix))]
fn free_bytes(_path: &Path) -> io::Result<Option<u64>> {
    Ok(None)
}

/// When the playlist of an HLS set was written, in nanoseconds. Nothing
/// rewrites it after publishing, and access times are set without touching
/// the modification time, so it identifies one generation of the set.
fn generation_of(dir: &Path) -> io::Result<Option<String>> {
    match fs::metadata(dir.join("index.m3u8")) {
        Ok(meta) => Ok(meta
            .modified()?
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_nanos().to_string())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// A file's length, or for an HLS directory the sum of its files.
fn entry_len(path: &Path, meta: &fs::Metadata) -> io::Result<u64> {
    if !meta.is_dir() {
        return Ok(meta.len());
    }
    let mut total = 0;
    for entry in fs::read_dir(path)? {
        let meta = entry?.metadata()?;
        if meta.is_file() {
            total += meta.len();
        }
    }
    Ok(total)
}

fn remove_entry(path: &Path) -> io::Result<()> {
    if fs::symlink_metadata(path)?.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

pub(crate) fn create_private_dir(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    // Windows directories inherit the ACL of the per-user data directory.
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    match builder.create(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(size: u64, mtime: Option<i64>) -> CacheKey {
        CacheKey {
            root_id: "local".into(),
            source_version: 0,
            path: b"RJ01/mp3/01.ape".to_vec(),
            size,
            mtime,
            params: "aac-192k",
        }
    }

    fn put(cache: &TranscodeCache, key: &CacheKey, len: usize, accessed_secs_ago: u64) {
        let path = cache.final_path(key);
        fs::write(&path, vec![0u8; len]).unwrap();
        let when = SystemTime::now() - Duration::from_secs(accessed_secs_ago);
        fs::File::open(&path)
            .unwrap()
            .set_times(FileTimes::new().set_accessed(when).set_modified(when))
            .unwrap();
    }

    #[test]
    fn keys_change_with_any_source_attribute() {
        let base = key(10, Some(1));
        let others = [
            key(11, Some(1)),
            key(10, Some(2)),
            key(10, None),
            CacheKey {
                root_id: "other".into(),
                ..base.clone()
            },
            CacheKey {
                source_version: 1,
                ..base.clone()
            },
            CacheKey {
                path: b"RJ02/mp3/01.ape".to_vec(),
                ..base.clone()
            },
            CacheKey {
                params: "aac-128k",
                ..base.clone()
            },
        ];
        for other in others {
            assert_ne!(base.digest(), other.digest(), "{other:?}");
        }
        assert_eq!(base.digest(), key(10, Some(1)).digest());
    }

    #[test]
    fn open_clears_leftover_temporary_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("tmp/sub")).unwrap();
        fs::write(dir.path().join("tmp/half.m4a"), b"x").unwrap();
        TranscodeCache::open(dir.path(), u64::MAX, 0).unwrap();
        assert_eq!(fs::read_dir(dir.path().join("tmp")).unwrap().count(), 0);
    }

    #[test]
    fn lookup_finds_published_files_and_expires_unknown_mtimes_after_a_week() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TranscodeCache::open(dir.path(), u64::MAX, 0).unwrap();
        assert!(cache.lookup(&key(10, Some(1))).unwrap().is_none());
        put(&cache, &key(10, Some(1)), 5, 30 * 24 * 3600);
        assert_eq!(cache.lookup(&key(10, Some(1))).unwrap().unwrap().len, 5);

        put(&cache, &key(10, None), 5, 8 * 24 * 3600);
        assert!(cache.lookup(&key(10, None)).unwrap().is_none());
        assert!(!cache.final_path(&key(10, None)).exists());
        put(&cache, &key(10, None), 5, 3600);
        assert!(cache.lookup(&key(10, None)).unwrap().is_some());
    }

    #[test]
    fn an_expired_file_in_use_is_not_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TranscodeCache::open(dir.path(), u64::MAX, 0).unwrap();
        put(&cache, &key(10, None), 5, 8 * 24 * 3600);
        let held = cache.pin(&key(10, None));
        assert!(cache.lookup(&key(10, None)).unwrap().is_none());
        assert!(cache.final_path(&key(10, None)).exists());
        drop(held);
    }

    #[test]
    fn eviction_drops_least_recently_used_files_but_never_pinned_ones() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TranscodeCache::open(dir.path(), 250, 0).unwrap();
        let (old, pinned, middle, new) = (
            key(1, Some(1)),
            key(2, Some(1)),
            key(3, Some(1)),
            key(4, Some(1)),
        );
        put(&cache, &old, 100, 400);
        put(&cache, &pinned, 100, 300);
        put(&cache, &middle, 100, 200);
        put(&cache, &new, 100, 100);
        let pin = cache.pin(&pinned);

        cache.evict().unwrap();
        assert!(!cache.final_path(&old).exists());
        assert!(cache.final_path(&pinned).exists(), "in use");
        assert!(!cache.final_path(&middle).exists());
        assert!(cache.final_path(&new).exists());

        drop(pin);
        cache.evict().unwrap();
        assert!(cache.final_path(&new).exists(), "already within the limit");
    }

    #[test]
    fn hls_directories_are_looked_up_counted_and_evicted_whole() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TranscodeCache::open(dir.path(), 150, 0).unwrap();
        let hls = |size| CacheKey {
            params: "hls-copy",
            ..key(size, Some(1))
        };
        let (old, new) = (hls(1), hls(2));
        for (k, ago) in [(&old, 400), (&new, 100)] {
            let path = cache.final_path(k);
            assert!(path.to_string_lossy().ends_with(".hls"));
            fs::create_dir(&path).unwrap();
            fs::write(path.join("index.m3u8"), vec![b'#'; 20]).unwrap();
            fs::write(path.join("seg-00000.ts"), vec![0u8; 80]).unwrap();
            let when = SystemTime::now() - Duration::from_secs(ago);
            fs::File::open(path.join("index.m3u8"))
                .unwrap()
                .set_times(FileTimes::new().set_accessed(when).set_modified(when))
                .unwrap();
        }
        // Listing a directory must not count as using it.
        cache.evict().unwrap();
        assert_eq!(cache.lookup(&new).unwrap().unwrap().len, 100);
        cache.evict().unwrap();
        assert!(!cache.final_path(&old).exists(), "200 bytes is over 150");
        assert!(cache.final_path(&new).join("seg-00000.ts").exists());
    }

    #[test]
    fn hls_set_ids_name_one_generation_and_count_as_access() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TranscodeCache::open(dir.path(), u64::MAX, 0).unwrap();
        let key = CacheKey {
            params: "hls-copy",
            ..key(1, None)
        };
        let path = cache.final_path(&key);
        fs::create_dir(&path).unwrap();
        let playlist = path.join("index.m3u8");
        fs::write(&playlist, b"#EXTM3U").unwrap();
        fs::write(path.join("seg-00000.ts"), b"segment").unwrap();
        let long_ago = SystemTime::now() - Duration::from_secs(3600);
        fs::File::open(&playlist)
            .unwrap()
            .set_times(
                FileTimes::new()
                    .set_accessed(long_ago)
                    .set_modified(long_ago),
            )
            .unwrap();
        let cached = cache.lookup(&key).unwrap().unwrap();
        let id = cache.hls_set_id(&key, &cached).unwrap();
        drop(cached);
        fs::File::open(&playlist)
            .unwrap()
            .set_times(FileTimes::new().set_accessed(long_ago))
            .unwrap();

        let segment = cache
            .open_hls_segment(&id, "seg-00000.ts")
            .unwrap()
            .unwrap();
        assert_eq!(segment.len, 7);
        let accessed = fs::metadata(&playlist).unwrap().accessed().unwrap();
        assert!(accessed > long_ago, "a segment hit is a use of the set");
        let modified = fs::metadata(&playlist).unwrap().modified().unwrap();
        assert_eq!(modified, long_ago, "the generation is untouched");
        drop(segment);
        assert_eq!(
            cache
                .open_hls_segment(&id, "seg-00001.ts")
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
        assert!(
            cache
                .open_hls_segment(&key.digest(), "seg-00000.ts")
                .unwrap()
                .is_none()
        );
        assert!(
            cache
                .open_hls_segment("zz-1", "seg-00000.ts")
                .unwrap()
                .is_none()
        );
        // Only segment names open, whatever the caller checked.
        for name in ["index.m3u8", "../seg-00000.ts", "seg-0.ts", ""] {
            assert!(
                cache.open_hls_segment(&id, name).unwrap().is_none(),
                "{name}"
            );
        }

        // Regenerated under the same key: the old ID no longer resolves.
        fs::File::open(&playlist)
            .unwrap()
            .set_times(FileTimes::new().set_modified(SystemTime::now()))
            .unwrap();
        assert!(
            cache
                .open_hls_segment(&id, "seg-00000.ts")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn lookup_counts_as_an_access_for_eviction() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TranscodeCache::open(dir.path(), 150, 0).unwrap();
        let (a, b) = (key(1, Some(1)), key(2, Some(1)));
        put(&cache, &a, 100, 400);
        put(&cache, &b, 100, 100);
        drop(cache.lookup(&a).unwrap());
        cache.evict().unwrap();
        assert!(cache.final_path(&a).exists());
        assert!(!cache.final_path(&b).exists());
    }

    #[test]
    fn space_check_refuses_below_the_minimum() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            TranscodeCache::open(dir.path(), u64::MAX, 0)
                .unwrap()
                .check_space()
                .is_ok()
        );
        let full = TranscodeCache::open(dir.path(), u64::MAX, u64::MAX).unwrap();
        assert!(matches!(
            full.check_space(),
            Err(MediaError::InsufficientStorage)
        ));
    }

    #[test]
    fn space_check_counts_the_bytes_about_to_be_written() {
        let dir = tempfile::tempdir().unwrap();
        let cache = TranscodeCache::open(dir.path(), u64::MAX, 0).unwrap();
        assert!(cache.check_space_for(1).is_ok());
        assert!(matches!(
            cache.check_space_for(u64::MAX),
            Err(MediaError::InsufficientStorage)
        ));
    }
}
