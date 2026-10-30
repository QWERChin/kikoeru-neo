//! Storage backends: what is in a root, how the server reads a file, and
//! how a browser gets it. The contract is book/src/storage.md.

#[cfg(unix)]
mod local;
#[cfg(windows)]
#[path = "local_windows.rs"]
mod local;
mod openlist;
mod path;
mod webdav;

use std::{collections::HashSet, fs::File, path::PathBuf, sync::Arc};

use futures_util::{StreamExt, stream::FuturesUnordered};

pub use local::LocalStorage;
pub use openlist::OpenListStorage;
pub use path::{PathError, RawPath};
pub use webdav::WebDavStorage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryKind {
    File,
    Dir,
    /// Symlinks are listed so the scanner can log them, but they are never
    /// followed or opened.
    Symlink,
    /// Sockets, FIFOs and devices.
    Other,
}

/// One child of a listed directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The raw file name, a single path component.
    pub name: Vec<u8>,
    pub kind: EntryKind,
    pub size: Option<u64>,
    /// Unix milliseconds. `None` when the backend has no reliable time.
    pub mtime_ms: Option<i64>,
    /// The server's version marker, from WebDAV. Many servers change a
    /// directory's ETag when anything below it changes, so it joins the
    /// listing digest. `None` for local and OpenList entries.
    pub etag: Option<String>,
}

/// A file or directory found by [`Storage::walk`] or [`Storage::walk_partial`],
/// with its path relative to the walked directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkEntry {
    pub path: RawPath,
    pub entry: Entry,
}

/// A tree with holes at directories that could not be listed. Both entry
/// and failure paths are relative to the walked directory.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PartialWalk {
    pub entries: Vec<WalkEntry>,
    /// Neither these paths nor their descendants may be treated as seen
    /// or missing. A directory entry from its parent is not a full listing.
    pub failed: Vec<(RawPath, StorageError)>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StorageError {
    /// The backend answered that the path does not exist. Only this counts
    /// as evidence that a work or track is missing.
    #[error("not found")]
    NotFound,
    /// The backend could not answer: the root is not mounted, a request
    /// timed out, a login failed, a listing broke off, the provider throttled.
    #[error("storage unavailable: {0}")]
    Unavailable(String),
    /// The backend answered with an error of its own.
    #[error("storage error: {0}")]
    Upstream(String),
}

/// A local file opened by the backend. Read it through `file`: opening
/// `path` again by name would give up the checks the backend made, because
/// the path may have changed since. A child process such as ffmpeg can get
/// the handle as an inherited descriptor or on stdin and open
/// `/dev/fd/N`, which stays seekable.
#[derive(Debug, Clone)]
pub struct LocalFile {
    pub file: Arc<File>,
    /// For logs and error messages only.
    pub path: PathBuf,
    pub size: u64,
    pub mtime_ms: Option<i64>,
}

/// How the server itself reads a file, for ffprobe, transcoding, subtitles
/// and zip downloads.
#[derive(Debug, Clone)]
pub enum Source {
    LocalFile(LocalFile),
    Remote(RemoteFile),
}

/// How a browser gets a file. Browsers only ever talk to this server.
#[derive(Debug, Clone)]
pub enum Access {
    /// The server answers Range requests from this open local file.
    ServeLocal(LocalFile),
    /// The server opens the file upstream and forwards the response.
    Remote(RemoteFile),
}

/// A file the server reads over the network, such as an OpenList file.
/// Callers never see a provider URL: they ask the opener, which handles
/// links, their expiry and the provider's rate limits.
#[derive(Clone)]
pub struct RemoteFile {
    pub size: Option<u64>,
    /// Unix milliseconds, `None` when the provider gives no reliable time.
    pub mtime_ms: Option<i64>,
    pub opener: Arc<dyn RemoteOpen>,
}

impl std::fmt::Debug for RemoteFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteFile")
            .field("size", &self.size)
            .field("mtime_ms", &self.mtime_ms)
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
pub trait RemoteOpen: Send + Sync {
    /// Starts reading the file, forwarding `headers` such as `Range` and
    /// `If-Range`. The status and headers are the upstream's.
    async fn open(&self, headers: &http::HeaderMap) -> Result<RemoteResponse, StorageError>;
}

/// An upstream response. Dropping `body` cancels the upstream request.
pub struct RemoteResponse {
    pub status: http::StatusCode,
    pub headers: http::HeaderMap,
    pub body: futures_util::stream::BoxStream<'static, Result<bytes::Bytes, std::io::Error>>,
}

/// The `list_concurrency` default of book/src/storage.md.
pub const DEFAULT_LIST_CONCURRENCY: usize = 4;

#[async_trait::async_trait]
pub trait Storage: Send + Sync {
    /// All direct children of a directory. Paging stays inside the backend,
    /// and a failed page fails the whole call rather than returning part of
    /// the listing.
    async fn list(&self, dir: &RawPath) -> Result<Vec<Entry>, StorageError>;

    async fn stat(&self, path: &RawPath) -> Result<Entry, StorageError>;

    async fn source(&self, path: &RawPath) -> Result<Source, StorageError>;

    async fn access(&self, path: &RawPath) -> Result<Access, StorageError>;

    /// How many listings a scan may have in flight at once. The OpenList
    /// backend reports its root's `list_concurrency`, which its client
    /// also enforces; local disks take the same default.
    fn list_concurrency(&self) -> usize {
        DEFAULT_LIST_CONCURRENCY
    }

    /// Lists readable subtrees and records failed directories separately.
    /// A failure at `dir` itself still fails the entire call.
    async fn walk_partial(&self, dir: &RawPath) -> Result<PartialWalk, StorageError> {
        walk_partial(self, dir, 1).await
    }

    /// Every file and directory below `dir`. A directory comes before its
    /// contents, and siblings keep the order of their listing. Symlinks and
    /// other special entries are returned but not descended into. Any failed
    /// listing fails the whole walk, so a caller never mistakes a partial
    /// tree for a complete one.
    async fn walk(&self, dir: &RawPath) -> Result<Vec<WalkEntry>, StorageError> {
        let mut found = Vec::new();
        let mut pending = vec![RawPath::root()];
        while let Some(rel) = pending.pop() {
            let entries = match self.list(&dir.join_path(&rel)).await {
                // A subdirectory listed a moment ago that is gone now means
                // the tree changed during the walk, not that `dir` is missing.
                Err(StorageError::NotFound) if !rel.is_root() => {
                    return Err(StorageError::Unavailable(format!(
                        "{} changed during the walk",
                        dir.join_path(&rel).to_string_lossy()
                    )));
                }
                other => other?,
            };
            let mut subdirs = Vec::new();
            for entry in entries {
                let path = rel
                    .join(&entry.name)
                    .map_err(|e| StorageError::Upstream(format!("bad name in listing: {e}")))?;
                if entry.kind == EntryKind::Dir {
                    subdirs.push(path.clone());
                }
                found.push(WalkEntry { path, entry });
            }
            // Reversed so the stack pops directories in listing order.
            pending.extend(subdirs.into_iter().rev());
        }
        Ok(found)
    }
}

/// Uses the same complete-listing boundary for every backend. OpenList's
/// client partial walk drops repeated names, while scans keep the first.
async fn walk_partial(
    storage: &(impl Storage + ?Sized),
    dir: &RawPath,
    concurrency: usize,
) -> Result<PartialWalk, StorageError> {
    let mut walk = PartialWalk::default();
    let mut pending = vec![RawPath::root()];
    let mut visited = HashSet::from([RawPath::root()]);
    let mut listings = FuturesUnordered::new();
    loop {
        while listings.len() < concurrency.max(1) {
            let Some(rel) = pending.pop() else { break };
            listings.push(async move {
                let result = match storage.list(&dir.join_path(&rel)).await {
                    Ok(entries) => entries
                        .into_iter()
                        .map(|entry| {
                            let path = rel.join(&entry.name).map_err(|e| {
                                StorageError::Upstream(format!("bad name in listing: {e}"))
                            })?;
                            Ok(WalkEntry { path, entry })
                        })
                        .collect::<Result<Vec<_>, StorageError>>(),
                    Err(error) => Err(error),
                };
                (rel, result)
            });
        }
        let Some((rel, result)) = listings.next().await else {
            break;
        };
        match result {
            Ok(entries) => {
                let subdirs: Vec<_> = entries
                    .iter()
                    .filter(|item| item.entry.kind == EntryKind::Dir)
                    .filter(|item| visited.insert(item.path.clone()))
                    .map(|item| item.path.clone())
                    .collect();
                pending.extend(subdirs.into_iter().rev());
                walk.entries.extend(entries);
            }
            Err(error) if rel.is_root() => return Err(error),
            Err(error) => {
                let error = match error {
                    StorageError::NotFound => StorageError::Unavailable(format!(
                        "{} changed during the walk",
                        dir.join_path(&rel).to_string_lossy()
                    )),
                    other => other,
                };
                walk.failed.push((rel, error));
            }
        }
    }
    Ok(walk)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lists `a` with one subdirectory `b` that is gone by the time the
    /// walk lists it.
    struct Vanishing;

    #[async_trait::async_trait]
    impl Storage for Vanishing {
        async fn list(&self, dir: &RawPath) -> Result<Vec<Entry>, StorageError> {
            match dir.as_bytes() {
                b"a" => Ok(vec![Entry {
                    name: b"b".to_vec(),
                    kind: EntryKind::Dir,
                    size: None,
                    mtime_ms: None,
                    etag: None,
                }]),
                _ => Err(StorageError::NotFound),
            }
        }

        async fn stat(&self, _: &RawPath) -> Result<Entry, StorageError> {
            unimplemented!()
        }

        async fn source(&self, _: &RawPath) -> Result<Source, StorageError> {
            unimplemented!()
        }

        async fn access(&self, _: &RawPath) -> Result<Access, StorageError> {
            unimplemented!()
        }
    }

    #[tokio::test]
    async fn a_vanished_subdirectory_makes_the_walk_unavailable() {
        let a = RawPath::new("a").unwrap();
        assert!(matches!(
            Vanishing.walk(&a).await,
            Err(StorageError::Unavailable(_))
        ));
        let gone = RawPath::new("gone").unwrap();
        assert_eq!(Vanishing.walk(&gone).await, Err(StorageError::NotFound));
    }

    #[tokio::test]
    async fn a_partial_walk_records_vanished_children_but_still_fails_at_its_root() {
        let walk = Vanishing
            .walk_partial(&RawPath::new("a").unwrap())
            .await
            .unwrap();
        assert_eq!(walk.entries.len(), 1);
        assert_eq!(walk.failed.len(), 1);
        assert_eq!(walk.failed[0].0, RawPath::new("b").unwrap());
        assert!(matches!(walk.failed[0].1, StorageError::Unavailable(_)));
        assert_eq!(
            Vanishing.walk_partial(&RawPath::new("gone").unwrap()).await,
            Err(StorageError::NotFound)
        );
    }
}
