//! The webdav backend: a thin adapter over `webdav-client`
//! (book/src/storage.md, "webdav").

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use futures_util::StreamExt;
use webdav_client::{Client, Entry as DavEntry, ErrorKind};

use crate::{
    Access, Entry, EntryKind, RawPath, RemoteFile, RemoteOpen, RemoteResponse, Source, Storage,
    StorageError,
};

#[derive(Debug, Clone)]
pub struct WebDavStorage {
    client: Client,
    /// The root's directory on the server, such as `/音声`, without a
    /// trailing slash. Empty for the server's root.
    base: String,
}

impl WebDavStorage {
    /// `base_path` is the root's directory below the client's URL.
    ///
    /// # Errors
    /// The base path has a segment that cannot be sent safely.
    pub fn new(client: Client, base_path: &str) -> Result<Self, StorageError> {
        let mut base = String::new();
        for segment in base_path.split('/').filter(|s| !s.is_empty()) {
            if segment == "." || segment == ".." || segment.contains(['\\', '\0']) {
                return Err(StorageError::Upstream(format!(
                    "invalid WebDAV base path {base_path:?}"
                )));
            }
            base.push('/');
            base.push_str(segment);
        }
        Ok(Self { client, base })
    }

    /// The server path for a path below the root.
    fn full_path(&self, rel: &RawPath) -> Result<String, StorageError> {
        if rel.is_root() {
            return Ok(self.base.clone());
        }
        // WebDAV names are text. A raw path that is not UTF-8 cannot have
        // come from a listing.
        let rel = std::str::from_utf8(rel.as_bytes())
            .map_err(|_| StorageError::Upstream("not a UTF-8 path".into()))?;
        // RawPath allows the byte because it is a Shift_JIS trail byte on
        // local disks, but a server may read it as a separator.
        if rel.contains('\\') {
            return Err(StorageError::Upstream("backslash in a WebDAV path".into()));
        }
        Ok(format!("{}/{rel}", self.base))
    }

    /// Maps a client error. As for local and OpenList roots, a missing path
    /// counts only while the base itself is still there: an unmounted share
    /// makes every path vanish at once, which must not read as deleted
    /// works.
    async fn error(&self, rel: &RawPath, error: &webdav_client::Error) -> StorageError {
        match error.kind() {
            ErrorKind::NotFound if rel.is_root() => self.base_missing(),
            ErrorKind::NotFound => match self.client.stat(&self.base).await {
                Ok(entry) if entry.is_dir => StorageError::NotFound,
                Ok(_) => self.base_missing(),
                Err(e) if e.kind() == ErrorKind::NotFound => self.base_missing(),
                Err(e) => StorageError::Unavailable(e.to_string()),
            },
            ErrorKind::Unavailable => StorageError::Unavailable(error.to_string()),
            ErrorKind::Upstream => StorageError::Upstream(error.to_string()),
        }
    }

    fn base_missing(&self) -> StorageError {
        let base = if self.base.is_empty() {
            "/"
        } else {
            &self.base
        };
        StorageError::Unavailable(format!("{base} is not on the WebDAV server"))
    }

    async fn remote(&self, rel: &RawPath) -> Result<RemoteFile, StorageError> {
        let entry = self.stat(rel).await?;
        if entry.kind != EntryKind::File {
            return Err(StorageError::NotFound);
        }
        Ok(RemoteFile {
            size: entry.size,
            mtime_ms: entry.mtime_ms,
            opener: Arc::new(WebDavFile {
                path: self.full_path(rel)?,
                storage: self.clone(),
                rel: rel.clone(),
            }),
        })
    }
}

fn convert(entry: &DavEntry) -> Entry {
    Entry {
        name: entry.name.as_bytes().to_vec(),
        kind: if entry.is_dir {
            EntryKind::Dir
        } else {
            EntryKind::File
        },
        size: entry.size,
        mtime_ms: entry.modified.and_then(unix_ms),
        etag: entry.etag.clone(),
    }
}

fn unix_ms(time: SystemTime) -> Option<i64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
}

#[async_trait::async_trait]
impl Storage for WebDavStorage {
    fn list_concurrency(&self) -> usize {
        self.client.config().list_concurrency
    }

    async fn list(&self, dir: &RawPath) -> Result<Vec<Entry>, StorageError> {
        let path = self.full_path(dir)?;
        match self.client.list(&path).await {
            Ok(entries) => Ok(entries.iter().map(convert).collect()),
            Err(e) => Err(self.error(dir, &e).await),
        }
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
}

struct WebDavFile {
    storage: WebDavStorage,
    rel: RawPath,
    path: String,
}

#[async_trait::async_trait]
impl RemoteOpen for WebDavFile {
    async fn open(&self, headers: &http::HeaderMap) -> Result<RemoteResponse, StorageError> {
        let transfer = match self.storage.client.open(&self.path, headers).await {
            Ok(transfer) => transfer,
            Err(e) => return Err(self.storage.error(&self.rel, &e).await),
        };
        let body = transfer
            .body
            .map(|chunk| chunk.map_err(std::io::Error::other))
            .boxed();
        Ok(RemoteResponse {
            status: transfer.status,
            headers: transfer.headers,
            body,
        })
    }
}
