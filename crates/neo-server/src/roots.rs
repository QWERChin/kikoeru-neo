//! Media library roots from both sources, config.toml and the database
//! (book/src/contract.md, "媒体库管理接口"), and the storage backend each
//! one gets.

use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, anyhow};
use neo_core::dto::{RootKind, RootSettings, RootSource};
use neo_db::roots::StoredRoot;
use neo_storage::{LocalStorage, OpenListStorage, Storage, WebDavStorage};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    config::{RootBackend, RootConfig},
    http::RootEntry,
    private::PrivateFile,
};

/// The defaults config.toml applies to an OpenList root.
pub const DEFAULT_LIST_CONCURRENCY: u32 = 4;
pub const DEFAULT_LINK_RATE: f64 = 1.0;

/// `root_settings.settings`: what a stored root needs besides its password.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum StoredSettings {
    Local {
        path: PathBuf,
        // Defaults keep rows saved before these fields existed readable.
        #[serde(default)]
        watch: Option<bool>,
        #[serde(default)]
        auto_scan_interval: Option<u32>,
    },
    OpenList {
        url: String,
        base_path: String,
        username: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password_version: Option<String>,
        list_concurrency: Option<u32>,
        link_rate: Option<f64>,
        #[serde(default)]
        auto_scan_interval: Option<u32>,
    },
    WebDav {
        url: String,
        base_path: String,
        username: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password_version: Option<String>,
        list_concurrency: Option<u32>,
        #[serde(default)]
        auto_scan_interval: Option<u32>,
    },
}

/// When a root is scanned without anyone asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScanPolicy {
    /// `None` takes the running mode's default.
    pub watch: Option<bool>,
    /// Seconds between automatic scans.
    pub auto_scan_interval: Option<u32>,
}

impl StoredSettings {
    pub fn password_version(&self) -> Option<&str> {
        match self {
            Self::Local { .. } => None,
            Self::OpenList {
                password_version, ..
            }
            | Self::WebDav {
                password_version, ..
            } => password_version.as_deref(),
        }
    }

    pub fn set_password_version(&mut self, version: Option<String>) {
        match self {
            Self::Local { .. } => {}
            Self::OpenList {
                password_version, ..
            }
            | Self::WebDav {
                password_version, ..
            } => {
                *password_version = version;
            }
        }
    }

    /// Rows written before versioned secrets use the original filename.
    /// A version is a nonce, never a path supplied by the stored JSON.
    pub fn password_path(&self, data_dir: &Path, id: &str) -> Result<PathBuf> {
        match self.password_version() {
            None => Ok(password_path(data_dir, id)),
            Some(version) => {
                anyhow::ensure!(
                    valid_password_version(version),
                    "invalid password version for root {id}"
                );
                Ok(data_dir
                    .join("secrets/roots")
                    .join(format!("{id}.{version}.password")))
            }
        }
    }

    pub fn kind(&self) -> RootKind {
        match self {
            Self::Local { .. } => RootKind::Local,
            Self::OpenList { .. } => RootKind::OpenList,
            Self::WebDav { .. } => RootKind::WebDav,
        }
    }

    pub fn policy(&self) -> ScanPolicy {
        match self {
            Self::Local {
                watch,
                auto_scan_interval,
                ..
            } => ScanPolicy {
                watch: *watch,
                auto_scan_interval: *auto_scan_interval,
            },
            Self::OpenList {
                auto_scan_interval, ..
            }
            | Self::WebDav {
                auto_scan_interval, ..
            } => ScanPolicy {
                watch: None,
                auto_scan_interval: *auto_scan_interval,
            },
        }
    }

    fn kind_name(&self) -> &'static str {
        match self {
            Self::Local { .. } => "local",
            Self::OpenList { .. } => "openlist",
            Self::WebDav { .. } => "webdav",
        }
    }

    /// The backend the storage is built from, with the password read from
    /// `password_file` under the data directory.
    pub fn backend(&self, password_file: PathBuf) -> Result<RootBackend> {
        Ok(match self.clone() {
            Self::Local { path, .. } => RootBackend::Local { path },
            Self::OpenList {
                url,
                base_path,
                username,
                list_concurrency,
                link_rate,
                ..
            } => RootBackend::OpenList {
                url: parse_http_url(&url)
                    .ok_or_else(|| anyhow!("{url:?} is not an http or https URL"))?,
                base_path,
                username,
                password_file,
                list_concurrency: list_concurrency.unwrap_or(DEFAULT_LIST_CONCURRENCY),
                link_rate: link_rate.unwrap_or(DEFAULT_LINK_RATE),
            },
            Self::WebDav {
                url,
                base_path,
                username,
                list_concurrency,
                ..
            } => RootBackend::WebDav {
                url: parse_http_url(&url)
                    .ok_or_else(|| anyhow!("{url:?} is not an http or https URL"))?,
                base_path,
                username,
                password_file,
                list_concurrency: list_concurrency.unwrap_or(DEFAULT_LIST_CONCURRENCY),
            },
        })
    }

    pub fn to_stored(&self, id: &str, display_name: &str) -> StoredRoot {
        StoredRoot {
            id: id.to_owned(),
            display_name: display_name.to_owned(),
            kind: self.kind_name().to_owned(),
            settings: serde_json::to_string(self).expect("settings serialize"),
        }
    }
}

/// The legacy, unversioned password path. New edits never overwrite it.
pub fn password_path(data_dir: &Path, id: &str) -> PathBuf {
    data_dir
        .join("secrets/roots")
        .join(format!("{id}.password"))
}

/// Writes an immutable password file before its version is committed with
/// the address. An interrupted write leaves only an unreferenced file.
pub fn write_password(data_dir: &Path, id: &str, password: &str) -> Result<String> {
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce).map_err(|e| anyhow!("no system randomness: {e}"))?;
    let version = format!("{:032x}", u128::from_le_bytes(nonce));
    let path = data_dir
        .join("secrets/roots")
        .join(format!("{id}.{version}.password"));
    let dir = path.parent().expect("the password path has a parent");
    crate::private::dir()
        .recursive(true)
        .create(dir)
        .with_context(|| format!("cannot create {}", dir.display()))?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .private()
        .open(&path)
        .with_context(|| format!("cannot write {}", path.display()))?;
    #[cfg(test)]
    crash_tests::crash_at("password_created");
    file.write_all(password.as_bytes())?;
    #[cfg(test)]
    crash_tests::crash_at("password_written");
    file.sync_all()?;
    #[cfg(test)]
    crash_tests::crash_at("password_synced");
    sync_directory(dir)?;
    sync_directory(&data_dir.join("secrets"))?;
    #[cfg(test)]
    crash_tests::crash_at("password_published");
    Ok(version)
}

fn valid_password_version(version: &str) -> bool {
    version.len() == 32 && version.bytes().all(|b| b.is_ascii_hexdigit())
}

fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    fs::File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// An exclusive lock on `secrets/.lock`, held while a root's settings and
/// password change and while `backup` copies the database and `secrets/`.
pub fn lock_secrets(data_dir: &Path) -> Result<fs::File> {
    let dir = data_dir.join("secrets");
    crate::private::dir()
        .recursive(true)
        .create(&dir)
        .with_context(|| format!("cannot create {}", dir.display()))?;
    let path = dir.join(".lock");
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .private()
        .open(&path)
        .with_context(|| format!("cannot open {}", path.display()))?;
    file.lock()
        .with_context(|| format!("cannot lock {}", path.display()))?;
    Ok(file)
}

/// Configured password paths stay protected even before their clients load.
pub fn configured_passwords(roots: &[RootConfig]) -> Vec<PathBuf> {
    roots
        .iter()
        .filter_map(|root| match &root.backend {
            RootBackend::Local { .. } => None,
            RootBackend::OpenList { password_file, .. }
            | RootBackend::WebDav { password_file, .. } => Some(password_file.clone()),
        })
        .collect()
}

/// Called under `secrets/.lock`, after a commit or at startup. Read every
/// row, including roots hidden by config.toml, before deleting anything.
/// Failed or uncertain commits leave both versions for this same sweep.
pub async fn cleanup_passwords(
    data_dir: &Path,
    db: &neo_db::Db,
    configured_passwords: &[PathBuf],
) -> Result<()> {
    let rows = neo_db::roots::list_stored_for_cleanup(db).await?;
    let mut referenced = HashSet::new();
    let mut superseded = HashSet::new();
    for path in configured_passwords {
        referenced.insert(fs::canonicalize(path)?);
    }
    for row in rows {
        let settings: StoredSettings = serde_json::from_str(&row.settings)
            .with_context(|| format!("cannot collect password references for root {}", row.id))?;
        if !matches!(settings, StoredSettings::Local { .. }) {
            referenced.insert(fs::canonicalize(
                settings.password_path(data_dir, &row.id)?,
            )?);
            if settings.password_version().is_some() {
                superseded.insert(row.id);
            }
        }
    }
    // SQLite uses WAL NORMAL. Flush the committed reference changes before
    // deleting files that a power loss could otherwise make live again.
    if let Some(sqlite) = db.sqlite() {
        anyhow::ensure!(
            sqlite.checkpoint().await?,
            "password cleanup deferred: database checkpoint is busy"
        );
    }
    #[cfg(test)]
    crash_tests::crash_at("references_synced");
    let dir = data_dir.join("secrets/roots");
    let files = match fs::read_dir(&dir) {
        Ok(files) => files,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    for file in files {
        let file = file?;
        let name = file.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(stem) = name.strip_suffix(".password") else {
            continue;
        };
        let managed = match stem.split_once('.') {
            Some((id, version)) => is_valid_stored_id(id) && valid_password_version(version),
            None => is_valid_stored_id(stem) && superseded.contains(stem),
        };
        if managed && !referenced.contains(&fs::canonicalize(file.path())?) {
            fs::remove_file(file.path())?;
            #[cfg(test)]
            crash_tests::crash_at("password_removed");
        }
    }
    sync_directory(&dir)?;
    Ok(())
}

/// A root from config.toml as the API shows it. `watch_by_default` is
/// what a local root that leaves `watch` unset gets.
pub fn config_entry(root: &RootConfig, watch_by_default: bool) -> RootEntry {
    let policy = ScanPolicy {
        watch: root.watch,
        auto_scan_interval: root.auto_scan_interval,
    };
    entry(
        &root.id,
        RootSource::Config,
        &root.backend,
        policy,
        watch_by_default,
    )
}

/// A root from the database as the API shows it.
pub fn stored_entry(
    data_dir: &Path,
    root: &StoredRoot,
    watch_by_default: bool,
) -> Result<(RootEntry, RootBackend)> {
    let settings: StoredSettings = serde_json::from_str(&root.settings)
        .with_context(|| format!("stored settings of root {} are unreadable", root.id))?;
    let backend = settings
        .backend(settings.password_path(data_dir, &root.id)?)
        .with_context(|| format!("stored settings of root {} are invalid", root.id))?;
    let entry = entry(
        &root.id,
        RootSource::Db,
        &backend,
        settings.policy(),
        watch_by_default,
    );
    Ok((entry, backend))
}

fn entry(
    id: &str,
    source: RootSource,
    backend: &RootBackend,
    policy: ScanPolicy,
    watch_by_default: bool,
) -> RootEntry {
    let auto_scan_interval = policy.auto_scan_interval;
    let (kind, settings) = match backend {
        RootBackend::Local { path } => (
            RootKind::Local,
            RootSettings::Local {
                path: path.display().to_string(),
                watch: policy.watch.unwrap_or(watch_by_default),
                auto_scan_interval,
            },
        ),
        RootBackend::OpenList {
            url,
            base_path,
            username,
            password_file,
            list_concurrency,
            link_rate,
        } => (
            RootKind::OpenList,
            RootSettings::OpenList {
                url: url.to_string(),
                base_path: base_path.clone(),
                username: username.clone(),
                // Whether a password is there, never the password.
                password_set: password_file.is_file(),
                list_concurrency: Some(*list_concurrency),
                link_rate: Some(*link_rate),
                auto_scan_interval,
            },
        ),
        RootBackend::WebDav {
            url,
            base_path,
            username,
            password_file,
            list_concurrency,
        } => (
            RootKind::WebDav,
            RootSettings::WebDav {
                url: url.to_string(),
                base_path: base_path.clone(),
                username: username.clone(),
                password_set: password_file.is_file(),
                list_concurrency: Some(*list_concurrency),
                auto_scan_interval,
            },
        ),
    };
    RootEntry {
        id: id.to_owned(),
        kind,
        source,
        settings,
    }
}

/// The storage backend for one root. An OpenList root reads its password
/// file here, so a missing file fails this root only.
pub fn build_storage(id: &str, backend: &RootBackend) -> Result<Arc<dyn Storage>> {
    build_storage_with(id, backend, None)
}

/// [`build_storage`], with `password` used instead of the password file
/// when given, so new settings can be tried before anything is saved.
pub fn build_storage_with(
    id: &str,
    backend: &RootBackend,
    password: Option<&str>,
) -> Result<Arc<dyn Storage>> {
    Ok(match backend {
        RootBackend::Local { path } => Arc::new(LocalStorage::new(path.clone())),
        RootBackend::OpenList {
            url,
            base_path,
            username,
            password_file,
            list_concurrency,
            link_rate,
        } => {
            let from_file;
            let password = match password {
                Some(password) => password,
                None => {
                    from_file = fs::read_to_string(password_file)
                        .with_context(|| format!("cannot read the password file of root {id}"))?;
                    // A trailing newline is how most secret files end.
                    from_file.trim_end_matches(['\r', '\n'])
                }
            };
            let mut client_config =
                openlist_client::Config::new(url.clone(), username.clone(), password);
            client_config.list_concurrency = *list_concurrency as usize;
            client_config.link_rate = *link_rate;
            client_config.user_agent =
                concat!("kikoeru-neo/", env!("CARGO_PKG_VERSION")).to_owned();
            let client = openlist_client::Client::new(client_config)
                .with_context(|| format!("invalid OpenList settings for root {id}"))?;
            let storage = OpenListStorage::new(client, base_path)
                .map_err(|e| anyhow!("invalid base_path for root {id}: {e}"))?;
            Arc::new(storage)
        }
        RootBackend::WebDav {
            url,
            base_path,
            username,
            password_file,
            list_concurrency,
        } => {
            let password = match password {
                Some(password) => password.to_owned(),
                None => read_password(id, password_file)?,
            };
            let mut client_config =
                webdav_client::Config::new(url.clone(), username.clone(), password);
            client_config.list_concurrency = *list_concurrency as usize;
            let client = webdav_client::Client::new(client_config)
                .with_context(|| format!("invalid WebDAV settings for root {id}"))?;
            let storage = WebDavStorage::new(client, base_path)
                .map_err(|e| anyhow!("invalid base_path for root {id}: {e}"))?;
            Arc::new(storage)
        }
    })
}

fn read_password(id: &str, file: &Path) -> Result<String> {
    let text = fs::read_to_string(file)
        .with_context(|| format!("cannot read the password file of root {id}"))?;
    // A trailing newline is how most secret files end.
    Ok(text.trim_end_matches(['\r', '\n']).to_owned())
}

/// The URL rule config.toml applies to `url`: http or https with a host.
pub fn parse_http_url(value: &str) -> Option<Url> {
    Url::parse(value)
        .ok()
        .filter(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some())
}

/// IDs of roots added through the API: 1 to 32 lowercase ASCII letters,
/// digits, `-` and `_`, starting with a letter or digit. Stricter than
/// config.toml, which also allows uppercase.
pub fn is_valid_stored_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    (1..=32).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-' || *b == b'_')
}

#[cfg(test)]
pub(crate) mod crash_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_ids_are_short_lowercase_names() {
        for good in ["cloud", "local-2", "a_1", "0"] {
            assert!(is_valid_stored_id(good), "{good}");
        }
        for bad in ["", "Cloud", "-x", "_x", "a b", "a/b", &"a".repeat(33)] {
            assert!(!is_valid_stored_id(bad), "{bad}");
        }
    }

    #[test]
    fn settings_round_trip_without_a_password() {
        let settings = StoredSettings::OpenList {
            url: "https://openlist.test/".into(),
            base_path: "/voice".into(),
            username: "u".into(),
            password_version: None,
            list_concurrency: None,
            link_rate: Some(2.0),
            auto_scan_interval: None,
        };
        let stored = settings.to_stored("cloud", "Cloud");
        assert!(!stored.settings.contains("password"));
        let dir = tempfile::tempdir().unwrap();
        let (entry, backend) = stored_entry(dir.path(), &stored, false).unwrap();
        assert_eq!(entry.source, RootSource::Db);
        let RootBackend::OpenList {
            list_concurrency,
            link_rate,
            password_file,
            ..
        } = backend
        else {
            panic!("an OpenList backend");
        };
        assert_eq!(
            (list_concurrency, link_rate),
            (DEFAULT_LIST_CONCURRENCY, 2.0)
        );
        assert_eq!(password_file, password_path(dir.path(), "cloud"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn passwords_are_private_files() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let version = write_password(dir.path(), "cloud", "secret").unwrap();
        let path = dir
            .path()
            .join(format!("secrets/roots/cloud.{version}.password"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "secret");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let db = neo_db::Db::open_sqlite(&dir.path().join("db.sqlite3"))
            .await
            .unwrap();
        cleanup_passwords(dir.path(), &db, &[]).await.unwrap();
        assert!(!path.exists());
        cleanup_passwords(dir.path(), &db, &[]).await.unwrap();
        db.close().await;
    }
}
