//! The openlist backend: a thin adapter over `openlist-client`, which does
//! the rate limiting, caching and link handling (book/src/storage.md).

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use futures_util::StreamExt;
use openlist_client::{Client, Entry as ListEntry, ErrorKind};

use crate::{
    Access, Entry, EntryKind, RawPath, RemoteFile, RemoteOpen, RemoteResponse, Source, Storage,
    StorageError, WalkEntry,
};

#[derive(Debug, Clone)]
pub struct OpenListStorage {
    client: Client,
    /// The root's directory on OpenList, normalized, such as `/cloud/云端作品`.
    base: String,
}

impl OpenListStorage {
    /// `base_path` is the root's directory on OpenList.
    ///
    /// # Errors
    /// The base path cannot be used as an OpenList path.
    pub fn new(client: Client, base_path: &str) -> Result<Self, StorageError> {
        let base = openlist_client::path::normalize(base_path)
            .map_err(|e| StorageError::Upstream(e.to_string()))?;
        Ok(Self { client, base })
    }

    /// The OpenList path for a path below the root.
    fn full_path(&self, rel: &RawPath) -> Result<String, StorageError> {
        if rel.is_root() {
            return Ok(self.base.clone());
        }
        // OpenList names are text. A raw path that is not UTF-8 cannot have
        // come from an OpenList listing.
        let rel = std::str::from_utf8(rel.as_bytes())
            .map_err(|_| StorageError::Upstream("not a UTF-8 path".into()))?;
        // OpenList turns `\` into `/` before cleaning a path, so a name with
        // one could reach outside the root. RawPath allows the byte because
        // it is a Shift_JIS trail byte on local disks.
        if rel.contains('\\') {
            return Err(StorageError::Upstream(
                "backslash in an OpenList path".into(),
            ));
        }
        Ok(format!("{}/{rel}", self.base.trim_end_matches('/')))
    }

    /// Maps a client error. A `NotFound` for the root itself means the base
    /// path is gone or not mounted, which must never read as an empty
    /// library.
    ///
    /// Any other `NotFound` counts only while the base itself is still
    /// there, as for local roots: an unmounted storage makes every path
    /// vanish at once, and that must not read as deleted works.
    async fn error(&self, rel: &RawPath, error: &openlist_client::Error) -> StorageError {
        match error.kind() {
            ErrorKind::NotFound if rel.is_root() => self.base_missing(),
            ErrorKind::NotFound => {
                // Listed fresh: a cached listing could predate the base
                // vanishing.
                self.client.invalidate_listing(&self.base);
                match self.client.list(&self.base).await {
                    Ok(_) => StorageError::NotFound,
                    Err(e) if e.kind() == ErrorKind::NotFound => self.base_missing(),
                    Err(e) => StorageError::Unavailable(e.to_string()),
                }
            }
            ErrorKind::Unavailable => StorageError::Unavailable(error.to_string()),
            ErrorKind::Upstream => StorageError::Upstream(error.to_string()),
        }
    }

    fn base_missing(&self) -> StorageError {
        StorageError::Unavailable(format!("{} is not on OpenList", self.base))
    }

    fn empty_root(&self) -> StorageError {
        StorageError::Unavailable(format!(
            "{} is empty; is the OpenList base path correct?",
            self.base
        ))
    }

    async fn remote(&self, rel: &RawPath) -> Result<RemoteFile, StorageError> {
        let entry = self.stat(rel).await?;
        if entry.kind != EntryKind::File {
            return Err(StorageError::NotFound);
        }
        Ok(RemoteFile {
            size: entry.size,
            mtime_ms: entry.mtime_ms,
            opener: Arc::new(OpenListFile {
                path: self.full_path(rel)?,
                storage: self.clone(),
                rel: rel.clone(),
            }),
        })
    }
}

fn convert(entry: &ListEntry) -> Entry {
    Entry {
        name: entry.name.as_bytes().to_vec(),
        kind: if entry.is_dir {
            EntryKind::Dir
        } else {
            EntryKind::File
        },
        size: (!entry.is_dir).then_some(entry.size),
        mtime_ms: entry.modified.and_then(unix_ms),
        etag: None,
    }
}

fn unix_ms(time: SystemTime) -> Option<i64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
}

#[async_trait::async_trait]
impl Storage for OpenListStorage {
    fn list_concurrency(&self) -> usize {
        self.client.config().list_concurrency
    }

    async fn list(&self, dir: &RawPath) -> Result<Vec<Entry>, StorageError> {
        let path = self.full_path(dir)?;
        let listing = match self.client.list(&path).await {
            Ok(listing) => listing,
            Err(e) => return Err(self.error(dir, &e).await),
        };
        if dir.is_root() && listing.entries.is_empty() {
            return Err(self.empty_root());
        }
        Ok(listing.entries.iter().map(convert).collect())
    }

    async fn stat(&self, path: &RawPath) -> Result<Entry, StorageError> {
        let full = self.full_path(path)?;
        let entry = match self.client.stat(&full).await {
            Ok(entry) => entry,
            Err(e) => return Err(self.error(path, &e).await),
        };
        let mut entry = convert(&entry);
        if let Some(name) = path.file_name() {
            entry.name = name.to_vec();
        }
        Ok(entry)
    }

    async fn source(&self, path: &RawPath) -> Result<Source, StorageError> {
        self.remote(path).await.map(Source::Remote)
    }

    async fn access(&self, path: &RawPath) -> Result<Access, StorageError> {
        self.remote(path).await.map(Access::Remote)
    }

    /// The client's concurrent walk. As in the default walk, a directory
    /// that vanishes below the walked one makes the walk unavailable, not
    /// the walked directory missing.
    async fn walk(&self, dir: &RawPath) -> Result<Vec<WalkEntry>, StorageError> {
        let full = self.full_path(dir)?;
        let top = match self.client.list(&full).await {
            Ok(top) => top,
            Err(e) => return Err(self.error(dir, &e).await),
        };
        if dir.is_root() && top.entries.is_empty() {
            return Err(self.empty_root());
        }
        let found = match self.client.walk_from(top).await {
            Ok(found) => found,
            Err(e) if e.kind() == ErrorKind::NotFound => {
                return Err(StorageError::Unavailable(format!(
                    "{full} changed during the walk"
                )));
            }
            Err(e) => return Err(self.error(dir, &e).await),
        };
        let prefix = format!("{}/", full.trim_end_matches('/'));
        found
            .iter()
            .map(|item| {
                let rel = item.path.strip_prefix(&prefix).ok_or_else(|| {
                    StorageError::Upstream(format!("{} is outside {full}", item.path))
                })?;
                let path = RawPath::new(rel.as_bytes().to_vec())
                    .map_err(|e| StorageError::Upstream(format!("bad name in listing: {e}")))?;
                Ok(WalkEntry {
                    path,
                    entry: convert(&item.entry),
                })
            })
            .collect()
    }
}

struct OpenListFile {
    storage: OpenListStorage,
    rel: RawPath,
    path: String,
}

#[async_trait::async_trait]
impl RemoteOpen for OpenListFile {
    async fn open(&self, headers: &http::HeaderMap) -> Result<RemoteResponse, StorageError> {
        // The bytes are forwarded as the file, so the upstream must not
        // compress them.
        let mut headers = headers.clone();
        headers.insert(
            http::header::ACCEPT_ENCODING,
            http::HeaderValue::from_static("identity"),
        );
        let transfer = match self.storage.client.open(&self.path, &headers).await {
            Ok(transfer) => transfer,
            // The provider refused even a fresh link: an upstream answer,
            // reported as 502, not an outage.
            Err(
                e @ openlist_client::Error::Http {
                    status: 401 | 403 | 404,
                    ..
                },
            ) => return Err(StorageError::Upstream(e.to_string())),
            // Through the same check as listings, so a vanished base is an
            // outage, not a deleted file.
            Err(e) => return Err(self.storage.error(&self.rel, &e).await),
        };
        let status = transfer.status();
        let headers = transfer.headers().clone();
        let body = transfer
            .into_stream()
            .map(|chunk| chunk.map_err(std::io::Error::other))
            .boxed();
        Ok(RemoteResponse {
            status,
            headers,
            body,
        })
    }
}
