//! Locating the data directory and reading `config.toml`, as described in
//! book/src/config.md. The service only reads the config file, never writes it.

use std::{
    collections::HashSet,
    ffi::OsString,
    fmt, fs, io,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};

use serde::Deserialize;
use url::Url;

pub const DATA_DIR_ENV: &str = "KIKOERU_DATA_DIR";
pub const LISTEN_ENV: &str = "KIKOERU_LISTEN";
pub const DATABASE_URL_ENV: &str = "KIKOERU_DATABASE_URL";
/// The config file for commands other than `serve`, which take no
/// `--config`, so that they open the database `serve` uses.
pub const CONFIG_ENV: &str = "KIKOERU_CONFIG";
const LISTEN_HINT: &str = "expected an address such as \"127.0.0.1:8888\"";
const CONFIG_FILE_NAME: &str = "config.toml";

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error(
        "no data directory: pass --data-dir or set {DATA_DIR_ENV} (the current directory is never used)"
    )]
    NoDataDir,
    #[error("cannot read config file {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("cannot parse config file {path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("invalid value for {param}: {value} ({reason})")]
    Invalid {
        param: String,
        value: String,
        reason: &'static str,
    },
}

/// An explicit tool path must be absolute: a relative one would depend on
/// the working directory the service happens to start in.
fn executable(param: &str, value: Option<PathBuf>) -> Result<Option<PathBuf>, ConfigError> {
    match value {
        Some(path) if !path.is_absolute() => Err(invalid(
            param,
            path.display(),
            "must be an absolute path to the executable",
        )),
        other => Ok(other),
    }
}

fn invalid(
    param: impl Into<String>,
    value: impl fmt::Display,
    reason: &'static str,
) -> ConfigError {
    ConfigError::Invalid {
        param: param.into(),
        value: value.to_string(),
        reason,
    }
}

/// Where the data directory and config file are, after applying precedence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub data_dir: PathBuf,
    pub config: PathBuf,
    /// The config path came from `--config`, so a missing file is an error
    /// instead of a reason to use defaults.
    pub config_explicit: bool,
}

/// `--data-dir` wins over `KIKOERU_DATA_DIR`. An empty variable counts as
/// unset. `--config` wins over `$DATA_DIR/config.toml`.
pub fn resolve_paths(
    cli_data_dir: Option<PathBuf>,
    env_data_dir: Option<OsString>,
    cli_config: Option<PathBuf>,
) -> Result<Paths, ConfigError> {
    let data_dir = cli_data_dir
        .or_else(|| env_data_dir.filter(|v| !v.is_empty()).map(PathBuf::from))
        .ok_or(ConfigError::NoDataDir)?;
    let config_explicit = cli_config.is_some();
    let config = cli_config.unwrap_or_else(|| data_dir.join(CONFIG_FILE_NAME));
    Ok(Paths {
        data_dir,
        config,
        config_explicit,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub listen: SocketAddr,
    /// The file set `listen`, so an overriding `KIKOERU_LISTEN` is worth
    /// a warning when it differs.
    listen_in_file: bool,
    pub public_url: Option<Url>,
    /// Reverse proxies whose `X-Forwarded-For` is believed for login rate
    /// limiting. Empty by default, so the socket address is used.
    pub trusted_proxies: Vec<IpAddr>,
    pub roots: Vec<RootConfig>,
    pub media: MediaConfig,
    pub cache: CacheConfig,
    pub log: LogConfig,
    pub metadata: MetadataConfig,
    /// `database.url`: a PostgreSQL database instead of the SQLite file in
    /// the data directory.
    pub database_url: Option<DatabaseUrl>,
    /// Whether local roots that leave `watch` unset are watched. Not read
    /// from the file: `serve` leaves it off and `embed::start` turns it on.
    pub watch_by_default: bool,
}

/// A `postgres://` URL. It may hold a password, so it is never printed as
/// it is, and `Debug` shows it without one.
#[derive(Clone, PartialEq, Eq)]
pub struct DatabaseUrl(String);

impl DatabaseUrl {
    fn parse(param: &str, value: &str) -> Result<Self, ConfigError> {
        if !(value.starts_with("postgres://") || value.starts_with("postgresql://")) {
            // The value is not echoed: it may be a URL with a password.
            return Err(invalid(
                param,
                "(not shown)",
                "expected a postgres:// URL; SQLite needs no setting",
            ));
        }
        if !cfg!(feature = "postgres") {
            return Err(invalid(
                param,
                "(not shown)",
                "this build of kikoeru-neo only supports SQLite",
            ));
        }
        Ok(Self(value.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for DatabaseUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DatabaseUrl(..)")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RootConfig {
    pub id: String,
    pub display_name: String,
    pub backend: RootBackend,
    /// `None` takes [`Config::watch_by_default`]. Always `None` or false
    /// for OpenList roots.
    pub watch: Option<bool>,
    /// Seconds between automatic scans, within the bounds of
    /// [`crate::autoscan::check_interval`].
    pub auto_scan_interval: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RootBackend {
    Local {
        path: PathBuf,
    },
    OpenList {
        url: Url,
        base_path: String,
        username: String,
        password_file: PathBuf,
        list_concurrency: u32,
        link_rate: f64,
    },
    /// A read-only WebDAV share (book/src/storage.md, "webdav").
    WebDav {
        url: Url,
        base_path: String,
        username: String,
        password_file: PathBuf,
        list_concurrency: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaConfig {
    pub heavy_jobs: u32,
    pub light_jobs: u32,
    pub job_timeout: Duration,
    pub queue_limit: u32,
    /// `media.ffmpeg`: an absolute path, or `None` to search `PATH`.
    pub ffmpeg: Option<PathBuf>,
    /// `media.ffprobe`: an absolute path, or `None` to search `PATH`.
    pub ffprobe: Option<PathBuf>,
    /// `media.video_transcode`: whether a video the browser cannot play,
    /// even after a remux, is re-encoded. Off, it gets an error instead,
    /// for a host whose CPU should not spend minutes on one video.
    pub video_transcode: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheConfig {
    pub max_bytes: u64,
    pub min_free_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogConfig {
    pub level: tracing::Level,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataConfig {
    pub source: MetadataSource,
    /// Where DLsite is. Only tests point it elsewhere, at a local fake.
    pub dlsite_url: Option<Url>,
    /// Asked when DLsite fails (book/src/scanning.md, "备用来源").
    pub fallback: MetadataFallback,
    /// asmr.one mirrors in probe order, or `None` for the defaults. Only
    /// tests set it, to a local fake.
    pub asmr_one_urls: Option<Vec<Url>>,
    /// Whether hvdb fills in voice actors the answer lacks
    /// (book/src/scanning.md, "声优补充").
    pub hvdb: bool,
    /// Where hvdb is. Only tests point it elsewhere, at a local fake.
    pub hvdb_url: Option<Url>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataFallback {
    None,
    AsmrOne,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataSource {
    Dlsite,
    /// No scraping: new works keep their directory names. For tests and
    /// for libraries without network access.
    None,
}

/// Whether `load` found a file or fell back to defaults, so the caller can
/// log it once logging is set up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    File,
    Defaults,
}

impl Config {
    /// The config at `paths`, with `KIKOERU_DATABASE_URL` applied.
    ///
    /// # Errors
    /// The file cannot be read or is invalid.
    pub fn load_with_env(paths: &Paths) -> Result<(Self, Source), ConfigError> {
        let (config, source) = Self::load(paths)?;
        let config = config.with_database_env(std::env::var_os(DATABASE_URL_ENV))?;
        Ok((config, source))
    }

    pub fn load(paths: &Paths) -> Result<(Self, Source), ConfigError> {
        let (raw, source) = RawConfig::load(paths)?;
        Ok((raw.validate()?, source))
    }

    /// The desktop always opens its SQLite file, even when a restored
    /// server config still contains a PostgreSQL address.
    pub(crate) fn load_sqlite(paths: &Paths) -> Result<(Self, Source), ConfigError> {
        let (mut raw, source) = RawConfig::load(paths)?;
        raw.database.url = None;
        Ok((raw.validate()?, source))
    }

    pub fn parse(text: &str, path: &Path) -> Result<Self, ConfigError> {
        RawConfig::parse(text, path)?.validate()
    }

    /// Applies `KIKOERU_DATABASE_URL`, which wins over `database.url` from
    /// the file, so a password can come from the service's environment
    /// instead of the config file. An empty variable counts as unset.
    pub fn with_database_env(self, value: Option<OsString>) -> Result<Self, ConfigError> {
        let Some(value) = value.filter(|v| !v.is_empty()) else {
            return Ok(self);
        };
        let url = DatabaseUrl::parse(DATABASE_URL_ENV, &value.to_string_lossy())?;
        Ok(Self {
            database_url: Some(url),
            ..self
        })
    }

    /// Where the library database is: `database.url`, or the SQLite file
    /// in `data_dir`.
    #[must_use]
    pub fn database(&self, data_dir: &Path) -> neo_db::Location {
        match &self.database_url {
            #[cfg(feature = "postgres")]
            Some(url) => neo_db::Location::Postgres(url.as_str().to_owned()),
            _ => neo_db::Location::Sqlite(data_dir.join(crate::datadir::DATABASE_FILE)),
        }
    }

    /// Applies `KIKOERU_LISTEN`, which wins over `listen` from the file. An
    /// empty variable counts as unset, as for `KIKOERU_DATA_DIR`. Returns
    /// the file's address when the variable replaced a different one, so
    /// the caller can warn once logging is set up.
    pub fn with_listen_env(
        self,
        value: Option<OsString>,
    ) -> Result<(Self, Option<SocketAddr>), ConfigError> {
        let Some(value) = value.filter(|v| !v.is_empty()) else {
            return Ok((self, None));
        };
        let listen = parse_listen(LISTEN_ENV, &value.to_string_lossy())?;
        let replaced = (self.listen_in_file && self.listen != listen).then_some(self.listen);
        Ok((Self { listen, ..self }, replaced))
    }
}

fn parse_listen(param: &str, value: &str) -> Result<SocketAddr, ConfigError> {
    value
        .parse()
        .map_err(|_| invalid(param, format!("{value:?}"), LISTEN_HINT))
}

// The raw structs accept whatever TOML type serde can read, so validation
// below can name the parameter and the value in its error.

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    listen: Option<String>,
    public_url: Option<String>,
    #[serde(default)]
    trusted_proxies: Vec<String>,
    #[serde(default)]
    roots: Vec<RawRoot>,
    #[serde(default)]
    media: RawMedia,
    #[serde(default)]
    cache: RawCache,
    #[serde(default)]
    log: RawLog,
    #[serde(default)]
    metadata: RawMetadata,
    #[serde(default)]
    database: RawDatabase,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawDatabase {
    url: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
enum RawRoot {
    Local {
        id: String,
        display_name: String,
        path: PathBuf,
        watch: Option<bool>,
        auto_scan_interval: Option<String>,
    },
    OpenList {
        id: String,
        display_name: String,
        url: String,
        base_path: String,
        username: String,
        password_file: PathBuf,
        list_concurrency: Option<i64>,
        link_rate: Option<f64>,
        watch: Option<bool>,
        auto_scan_interval: Option<String>,
    },
    WebDav {
        id: String,
        display_name: String,
        url: String,
        base_path: String,
        username: String,
        password_file: PathBuf,
        list_concurrency: Option<i64>,
        watch: Option<bool>,
        auto_scan_interval: Option<String>,
    },
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawMedia {
    heavy_jobs: Option<i64>,
    light_jobs: Option<i64>,
    job_timeout: Option<String>,
    queue_limit: Option<i64>,
    ffmpeg: Option<PathBuf>,
    ffprobe: Option<PathBuf>,
    video_transcode: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawCache {
    max_bytes: Option<RawSize>,
    min_free_bytes: Option<RawSize>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawSize {
    Bytes(i64),
    Text(String),
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawLog {
    level: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawMetadata {
    source: Option<String>,
    dlsite_url: Option<String>,
    fallback: Option<String>,
    asmr_one_urls: Option<Vec<String>>,
    hvdb: Option<bool>,
    hvdb_url: Option<String>,
}

fn positive(param: &str, value: Option<i64>, default: u32) -> Result<u32, ConfigError> {
    match value {
        None => Ok(default),
        Some(v) => u32::try_from(v)
            .ok()
            .filter(|v| *v > 0)
            .ok_or_else(|| invalid(param, v, "must be a positive integer")),
    }
}

fn size(param: &str, value: Option<RawSize>, default: &str) -> Result<u64, ConfigError> {
    match value {
        None => Ok(parse_size(default).expect("default size parses")),
        Some(RawSize::Bytes(v)) => {
            u64::try_from(v).map_err(|_| invalid(param, v, "must not be negative"))
        }
        Some(RawSize::Text(text)) => parse_size(&text).ok_or_else(|| {
            invalid(
                param,
                format!("{text:?}"),
                "expected a size such as \"20GB\" or \"512MiB\"",
            )
        }),
    }
}

/// Sizes use decimal units (`KB` = 1000) and binary units (`KiB` = 1024).
fn parse_size(text: &str) -> Option<u64> {
    let text = text.trim();
    if text.starts_with('-') {
        return None;
    }
    bytesize::ByteSize::from_str(text).ok().map(|s| s.as_u64())
}

impl RawConfig {
    fn load(paths: &Paths) -> Result<(Self, Source), ConfigError> {
        match fs::read_to_string(&paths.config) {
            Ok(text) => Ok((Self::parse(&text, &paths.config)?, Source::File)),
            Err(e) if e.kind() == io::ErrorKind::NotFound && !paths.config_explicit => {
                Ok((Self::parse("", &paths.config)?, Source::Defaults))
            }
            Err(source) => Err(ConfigError::Read {
                path: paths.config.clone(),
                source,
            }),
        }
    }

    fn parse(text: &str, path: &Path) -> Result<Self, ConfigError> {
        toml::from_str(text).map_err(|e| ConfigError::Parse {
            path: path.to_owned(),
            message: e.to_string(),
        })
    }

    fn validate(self) -> Result<Config, ConfigError> {
        let listen_in_file = self.listen.is_some();
        let listen = match self.listen {
            None => SocketAddr::from(([127, 0, 0, 1], 8888)),
            Some(v) => parse_listen("listen", &v)?,
        };
        let public_url = self
            .public_url
            .map(|v| parse_http_url("public_url", &v))
            .transpose()?;
        let trusted_proxies = self
            .trusted_proxies
            .iter()
            .enumerate()
            .map(|(i, v)| {
                v.parse().map_err(|_| {
                    invalid(
                        format!("trusted_proxies[{i}]"),
                        format!("{v:?}"),
                        "expected an IP address such as \"127.0.0.1\"",
                    )
                })
            })
            .collect::<Result<_, _>>()?;
        let mut seen = HashSet::new();
        let roots = self
            .roots
            .into_iter()
            .enumerate()
            .map(|(i, root)| {
                let root = root.validate(i)?;
                if !seen.insert(root.id.clone()) {
                    return Err(invalid(
                        format!("roots[{i}].id"),
                        format!("{:?}", root.id),
                        "duplicate root id",
                    ));
                }
                Ok(root)
            })
            .collect::<Result<_, _>>()?;
        let media = MediaConfig {
            heavy_jobs: positive("media.heavy_jobs", self.media.heavy_jobs, 1)?,
            light_jobs: positive("media.light_jobs", self.media.light_jobs, 4)?,
            job_timeout: match self.media.job_timeout {
                None => Duration::from_secs(2 * 60 * 60),
                Some(v) => humantime::parse_duration(&v)
                    .ok()
                    .filter(|d| !d.is_zero())
                    .ok_or_else(|| {
                        invalid(
                            "media.job_timeout",
                            format!("{v:?}"),
                            "expected a duration such as \"2h\" or \"90m\"",
                        )
                    })?,
            },
            queue_limit: positive("media.queue_limit", self.media.queue_limit, 32)?,
            ffmpeg: executable("media.ffmpeg", self.media.ffmpeg)?,
            ffprobe: executable("media.ffprobe", self.media.ffprobe)?,
            video_transcode: self.media.video_transcode.unwrap_or(true),
        };
        let cache = CacheConfig {
            max_bytes: size("cache.max_bytes", self.cache.max_bytes, "20GB")?,
            min_free_bytes: size("cache.min_free_bytes", self.cache.min_free_bytes, "5GB")?,
        };
        let log = LogConfig {
            level: match self.log.level {
                None => tracing::Level::INFO,
                Some(v) => match v.as_str() {
                    "trace" | "debug" | "info" | "warn" | "error" => {
                        v.parse().expect("listed levels parse")
                    }
                    _ => {
                        return Err(invalid(
                            "log.level",
                            format!("{v:?}"),
                            "expected one of trace, debug, info, warn, error",
                        ));
                    }
                },
            },
        };
        let metadata = MetadataConfig {
            source: match self.metadata.source.as_deref() {
                None | Some("dlsite") => MetadataSource::Dlsite,
                Some("none") => MetadataSource::None,
                Some(v) => {
                    return Err(invalid(
                        "metadata.source",
                        format!("{v:?}"),
                        "expected dlsite or none",
                    ));
                }
            },
            dlsite_url: match &self.metadata.dlsite_url {
                None => None,
                Some(v) => Some(base_url("metadata.dlsite_url", v)?),
            },
            fallback: match self.metadata.fallback.as_deref() {
                None | Some("none") => MetadataFallback::None,
                Some("asmr_one") => MetadataFallback::AsmrOne,
                Some(v) => {
                    return Err(invalid(
                        "metadata.fallback",
                        format!("{v:?}"),
                        "expected none or asmr_one",
                    ));
                }
            },
            asmr_one_urls: match &self.metadata.asmr_one_urls {
                None => None,
                Some(list) if list.is_empty() => {
                    return Err(invalid(
                        "metadata.asmr_one_urls",
                        "[]",
                        "list at least one mirror",
                    ));
                }
                Some(list) => Some(
                    list.iter()
                        .map(|v| base_url("metadata.asmr_one_urls", v))
                        .collect::<Result<_, _>>()?,
                ),
            },
            hvdb: self.metadata.hvdb.unwrap_or(false),
            hvdb_url: match &self.metadata.hvdb_url {
                None => None,
                Some(v) => Some(base_url("metadata.hvdb_url", v)?),
            },
        };
        Ok(Config {
            listen,
            listen_in_file,
            public_url,
            trusted_proxies,
            roots,
            media,
            cache,
            log,
            metadata,
            database_url: self
                .database
                .url
                .map(|v| DatabaseUrl::parse("database.url", &v))
                .transpose()?,
            watch_by_default: false,
        })
    }
}

/// An http or https URL that endpoints are joined onto. A last segment
/// without the slash would be replaced by the join, so one is added.
fn base_url(param: &str, value: &str) -> Result<Url, ConfigError> {
    let mut url = parse_http_url(param, value)?;
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()));
    }
    Ok(url)
}

/// Root IDs end up in URLs and in the database, and `import-legacy --root`
/// checks its IDs with the same rule.
pub fn is_valid_root_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn parse_http_url(param: &str, value: &str) -> Result<Url, ConfigError> {
    Url::parse(value)
        .ok()
        .filter(|u| matches!(u.scheme(), "http" | "https") && u.host().is_some())
        .ok_or_else(|| invalid(param, format!("{value:?}"), "expected an http or https URL"))
}

impl RawRoot {
    fn validate(self, i: usize) -> Result<RootConfig, ConfigError> {
        let param = |name: &str| format!("roots[{i}].{name}");
        let (id, display_name, backend, watch, interval) = match self {
            Self::Local {
                id,
                display_name,
                path,
                watch,
                auto_scan_interval,
            } => {
                if !path.is_absolute() {
                    return Err(invalid(
                        param("path"),
                        path.display(),
                        "must be an absolute path",
                    ));
                }
                (
                    id,
                    display_name,
                    RootBackend::Local { path },
                    watch,
                    auto_scan_interval,
                )
            }
            Self::OpenList {
                id,
                display_name,
                url,
                base_path,
                username,
                password_file,
                list_concurrency,
                link_rate,
                watch,
                auto_scan_interval,
            } => {
                if watch == Some(true) {
                    return Err(invalid(
                        param("watch"),
                        true,
                        "only local roots can be watched",
                    ));
                }
                let link_rate = link_rate.unwrap_or(1.0);
                if !(link_rate.is_finite() && link_rate > 0.0) {
                    return Err(invalid(
                        param("link_rate"),
                        link_rate,
                        "must be a positive number",
                    ));
                }
                let backend = RootBackend::OpenList {
                    url: parse_http_url(&param("url"), &url)?,
                    base_path,
                    username,
                    password_file,
                    list_concurrency: positive(&param("list_concurrency"), list_concurrency, 4)?,
                    link_rate,
                };
                (id, display_name, backend, watch, auto_scan_interval)
            }
            Self::WebDav {
                id,
                display_name,
                url,
                base_path,
                username,
                password_file,
                list_concurrency,
                watch,
                auto_scan_interval,
            } => {
                if watch == Some(true) {
                    return Err(invalid(
                        param("watch"),
                        true,
                        "only local roots can be watched",
                    ));
                }
                let backend = RootBackend::WebDav {
                    url: parse_http_url(&param("url"), &url)?,
                    base_path,
                    username,
                    password_file,
                    list_concurrency: positive(&param("list_concurrency"), list_concurrency, 4)?,
                };
                (id, display_name, backend, watch, auto_scan_interval)
            }
        };
        let kind = match backend {
            RootBackend::Local { .. } => neo_core::dto::RootKind::Local,
            RootBackend::OpenList { .. } => neo_core::dto::RootKind::OpenList,
            RootBackend::WebDav { .. } => neo_core::dto::RootKind::WebDav,
        };
        let auto_scan_interval = match interval {
            None => None,
            Some(text) => {
                let secs = humantime::parse_duration(&text)
                    .ok()
                    .and_then(|d| u32::try_from(d.as_secs()).ok())
                    .ok_or_else(|| {
                        invalid(
                            param("auto_scan_interval"),
                            format!("{text:?}"),
                            "expected a duration such as \"6h\" or \"30m\"",
                        )
                    })?;
                crate::autoscan::check_interval(kind, secs).map_err(|reason| {
                    invalid(param("auto_scan_interval"), format!("{text:?}"), reason)
                })?;
                Some(secs)
            }
        };
        if !is_valid_root_id(&id) {
            return Err(invalid(
                param("id"),
                format!("{id:?}"),
                "must be non-empty and use only ASCII letters, digits, '-' and '_'",
            ));
        }
        Ok(RootConfig {
            id,
            display_name,
            backend,
            watch,
            auto_scan_interval,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Config, ConfigError> {
        Config::parse(text, Path::new("config.toml"))
    }

    fn invalid_param(text: &str) -> (String, String) {
        match parse(text) {
            Err(ConfigError::Invalid { param, value, .. }) => (param, value),
            other => panic!("expected an invalid value error, got {other:?}"),
        }
    }

    #[test]
    fn cli_data_dir_wins_over_env() {
        let paths = resolve_paths(Some("/cli".into()), Some("/env".into()), None).unwrap();
        assert_eq!(paths.data_dir, PathBuf::from("/cli"));
        assert_eq!(paths.config, PathBuf::from("/cli/config.toml"));
        assert!(!paths.config_explicit);
    }

    #[test]
    fn env_data_dir_is_the_fallback() {
        let paths = resolve_paths(None, Some("/env".into()), None).unwrap();
        assert_eq!(paths.data_dir, PathBuf::from("/env"));
        assert_eq!(paths.config, PathBuf::from("/env/config.toml"));
    }

    #[test]
    fn no_data_dir_is_an_error() {
        assert!(matches!(
            resolve_paths(None, None, None),
            Err(ConfigError::NoDataDir)
        ));
        assert!(matches!(
            resolve_paths(None, Some("".into()), Some("/etc/c.toml".into())),
            Err(ConfigError::NoDataDir)
        ));
    }

    #[test]
    fn cli_config_wins_over_data_dir_file() {
        let paths = resolve_paths(None, Some("/env".into()), Some("/etc/c.toml".into())).unwrap();
        assert_eq!(paths.config, PathBuf::from("/etc/c.toml"));
        assert!(paths.config_explicit);
    }

    #[test]
    fn missing_default_config_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let paths = resolve_paths(Some(dir.path().into()), None, None).unwrap();
        let (config, source) = Config::load(&paths).unwrap();
        assert_eq!(source, Source::Defaults);
        assert_eq!(config, parse("").unwrap());
    }

    #[test]
    fn missing_explicit_config_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let paths = resolve_paths(
            Some(dir.path().into()),
            None,
            Some(dir.path().join("absent.toml")),
        )
        .unwrap();
        assert!(matches!(
            Config::load(&paths),
            Err(ConfigError::Read { .. })
        ));
    }

    #[test]
    fn defaults_match_the_book() {
        let c = parse("").unwrap();
        assert_eq!(c.listen, "127.0.0.1:8888".parse().unwrap());
        assert_eq!(c.public_url, None);
        assert!(c.trusted_proxies.is_empty());
        assert!(c.roots.is_empty());
        assert_eq!(
            c.media,
            MediaConfig {
                heavy_jobs: 1,
                light_jobs: 4,
                job_timeout: Duration::from_secs(7200),
                queue_limit: 32,
                ffmpeg: None,
                ffprobe: None,
                video_transcode: true,
            }
        );
        assert_eq!(c.cache.max_bytes, 20_000_000_000);
        assert_eq!(c.cache.min_free_bytes, 5_000_000_000);
        assert_eq!(c.log.level, tracing::Level::INFO);
        assert_eq!(c.metadata.source, MetadataSource::Dlsite);
    }

    #[test]
    fn full_config_parses() {
        let c = parse(
            r#"
            listen = "0.0.0.0:9000"
            public_url = "https://kikoeru.example.lan"
            trusted_proxies = ["127.0.0.1", "::1"]

            [[roots]]
            id = "local1"
            display_name = "Local"
            type = "local"
            path = "/srv/media"

            [[roots]]
            id = "cloud"
            display_name = "Cloud"
            type = "openlist"
            url = "https://openlist.example.lan"
            base_path = "/cloud/works"
            username = "kikoeru"
            password_file = "/run/secrets/openlist"
            link_rate = 0.5

            [media]
            heavy_jobs = 2
            job_timeout = "90m"
            video_transcode = false

            [cache]
            max_bytes = "1GiB"
            min_free_bytes = 1000

            [log]
            level = "debug"

            [metadata]
            source = "none"
            dlsite_url = "http://127.0.0.1:9000/dlsite"
            "#,
        )
        .unwrap();
        assert_eq!(c.listen, "0.0.0.0:9000".parse().unwrap());
        assert_eq!(
            c.trusted_proxies,
            [
                "127.0.0.1".parse::<IpAddr>().unwrap(),
                "::1".parse().unwrap()
            ]
        );
        assert_eq!(c.roots.len(), 2);
        assert_eq!(
            c.roots[1].backend,
            RootBackend::OpenList {
                url: Url::parse("https://openlist.example.lan").unwrap(),
                base_path: "/cloud/works".into(),
                username: "kikoeru".into(),
                password_file: "/run/secrets/openlist".into(),
                list_concurrency: 4,
                link_rate: 0.5,
            }
        );
        assert_eq!(c.media.heavy_jobs, 2);
        assert_eq!(c.media.job_timeout, Duration::from_secs(5400));
        assert!(!c.media.video_transcode);
        assert_eq!(c.cache.max_bytes, 1 << 30);
        assert_eq!(c.cache.min_free_bytes, 1000);
        assert_eq!(c.log.level, tracing::Level::DEBUG);
        assert_eq!(c.metadata.source, MetadataSource::None);
        assert_eq!(
            c.metadata.dlsite_url,
            Some(Url::parse("http://127.0.0.1:9000/dlsite/").unwrap())
        );
    }

    #[test]
    fn roots_take_a_scan_interval_and_local_ones_a_watch_flag() {
        let c = parse(
            r#"
            [[roots]]
            id = "a"
            display_name = "A"
            type = "local"
            path = "/srv/a"
            watch = true
            auto_scan_interval = "1d"

            [[roots]]
            id = "b"
            display_name = "B"
            type = "openlist"
            url = "https://o.test"
            base_path = "/"
            username = "u"
            password_file = "/run/p"
            auto_scan_interval = "6h"
            "#,
        )
        .unwrap();
        assert!(!c.watch_by_default);
        assert_eq!(c.roots[0].watch, Some(true));
        assert_eq!(c.roots[0].auto_scan_interval, Some(86_400));
        assert_eq!(c.roots[1].watch, None);
        assert_eq!(c.roots[1].auto_scan_interval, Some(21_600));
    }

    #[test]
    fn scan_settings_outside_their_bounds_fail() {
        let local = |extra: &str| {
            format!(
                "[[roots]]\nid = \"a\"\ndisplay_name = \"A\"\ntype = \"local\"\npath = \"/srv/a\"\n{extra}"
            )
        };
        let openlist = |extra: &str| {
            format!(
                "[[roots]]\nid = \"b\"\ndisplay_name = \"B\"\ntype = \"openlist\"\n\
                 url = \"https://o.test\"\nbase_path = \"/\"\nusername = \"u\"\n\
                 password_file = \"/run/p\"\n{extra}"
            )
        };
        let cases = [
            (local("auto_scan_interval = \"4m\""), "\"4m\""),
            (local("auto_scan_interval = \"31d\""), "\"31d\""),
            (local("auto_scan_interval = \"often\""), "\"often\""),
            (openlist("auto_scan_interval = \"29m\""), "\"29m\""),
        ];
        for (text, value) in cases {
            assert_eq!(
                invalid_param(&text),
                ("roots[0].auto_scan_interval".to_owned(), value.to_owned()),
                "{text}"
            );
        }
        assert_eq!(
            invalid_param(&openlist("watch = true")),
            ("roots[0].watch".to_owned(), "true".to_owned())
        );
    }

    #[test]
    fn invalid_values_name_param_and_value() {
        let cases = [
            ("[media]\nheavy_jobs = -1", "media.heavy_jobs", "-1"),
            ("[media]\nqueue_limit = 0", "media.queue_limit", "0"),
            (
                "[media]\nffmpeg = \"bin/ffmpeg\"",
                "media.ffmpeg",
                "bin/ffmpeg",
            ),
            ("[media]\nffprobe = \"ffprobe\"", "media.ffprobe", "ffprobe"),
            (
                "[media]\njob_timeout = \"soon\"",
                "media.job_timeout",
                "\"soon\"",
            ),
            (
                "[cache]\nmax_bytes = \"20 parsecs\"",
                "cache.max_bytes",
                "\"20 parsecs\"",
            ),
            (
                "[cache]\nmin_free_bytes = \"-5GB\"",
                "cache.min_free_bytes",
                "\"-5GB\"",
            ),
            ("[cache]\nmax_bytes = -3", "cache.max_bytes", "-3"),
            ("listen = \"localhost\"", "listen", "\"localhost\""),
            ("public_url = \"ftp://x\"", "public_url", "\"ftp://x\""),
            ("[log]\nlevel = \"loud\"", "log.level", "\"loud\""),
            (
                "[metadata]\nsource = \"vgmdb\"",
                "metadata.source",
                "\"vgmdb\"",
            ),
            (
                "[metadata]\ndlsite_url = \"ftp://x\"",
                "metadata.dlsite_url",
                "\"ftp://x\"",
            ),
            (
                "trusted_proxies = [\"proxy.lan\"]",
                "trusted_proxies[0]",
                "\"proxy.lan\"",
            ),
            (
                "[[roots]]\nid = \"a b\"\ndisplay_name = \"x\"\ntype = \"local\"\npath = \"/m\"",
                "roots[0].id",
                "\"a b\"",
            ),
            (
                "[[roots]]\nid = \"a\"\ndisplay_name = \"x\"\ntype = \"local\"\npath = \"rel\"",
                "roots[0].path",
                "rel",
            ),
        ];
        for (text, param, value) in cases {
            assert_eq!(
                invalid_param(text),
                (param.to_owned(), value.to_owned()),
                "{text}"
            );
        }
    }

    #[test]
    fn the_metadata_fallback_is_off_until_asked_for() {
        let c = parse("").unwrap();
        assert_eq!(c.metadata.fallback, MetadataFallback::None);
        assert_eq!(c.metadata.asmr_one_urls, None);
        let c = parse(
            "[metadata]\nfallback = \"asmr_one\"\nasmr_one_urls = [\"http://127.0.0.1:9000/asmr\"]\n",
        )
        .unwrap();
        assert_eq!(c.metadata.fallback, MetadataFallback::AsmrOne);
        assert_eq!(
            c.metadata.asmr_one_urls,
            Some(vec![Url::parse("http://127.0.0.1:9000/asmr/").unwrap()])
        );
        for (text, param) in [
            ("[metadata]\nfallback = \"hvdb\"", "metadata.fallback"),
            ("[metadata]\nasmr_one_urls = []", "metadata.asmr_one_urls"),
            (
                "[metadata]\nasmr_one_urls = [\"ftp://x\"]",
                "metadata.asmr_one_urls",
            ),
        ] {
            assert_eq!(invalid_param(text).0, param, "{text}");
        }
    }

    #[test]
    fn hvdb_is_off_until_asked_for() {
        let c = parse("").unwrap();
        assert!(!c.metadata.hvdb);
        assert_eq!(c.metadata.hvdb_url, None);
        let c =
            parse("[metadata]\nhvdb = true\nhvdb_url = \"http://127.0.0.1:9000/hvdb\"\n").unwrap();
        assert!(c.metadata.hvdb);
        assert_eq!(
            c.metadata.hvdb_url,
            Some(Url::parse("http://127.0.0.1:9000/hvdb/").unwrap())
        );
        assert_eq!(
            invalid_param("[metadata]\nhvdb_url = \"ftp://x\"").0,
            "metadata.hvdb_url"
        );
    }

    #[test]
    fn webdav_roots_take_their_own_fields() {
        let c = parse(
            "[[roots]]\nid = \"nas\"\ndisplay_name = \"NAS\"\ntype = \"webdav\"\n\
             url = \"https://nas.example.lan/dav\"\nbase_path = \"/音声\"\n\
             username = \"kikoeru\"\npassword_file = \"/run/secrets/webdav\"\n",
        )
        .unwrap();
        assert_eq!(
            c.roots[0].backend,
            RootBackend::WebDav {
                url: Url::parse("https://nas.example.lan/dav").unwrap(),
                base_path: "/音声".into(),
                username: "kikoeru".into(),
                password_file: "/run/secrets/webdav".into(),
                list_concurrency: 4,
            }
        );
        // link_rate belongs to OpenList only.
        let err = parse(
            "[[roots]]\nid = \"nas\"\ndisplay_name = \"NAS\"\ntype = \"webdav\"\n\
             url = \"https://nas.example.lan\"\nbase_path = \"/\"\nusername = \"u\"\n\
             password_file = \"/p\"\nlink_rate = 1.0\n",
        )
        .unwrap_err();
        assert!(matches!(err, ConfigError::Parse { .. }), "{err}");
    }

    #[test]
    fn duplicate_root_ids_are_rejected() {
        let root = "[[roots]]\nid = \"a\"\ndisplay_name = \"x\"\ntype = \"local\"\npath = \"/m\"\n";
        assert_eq!(
            invalid_param(&format!("{root}{root}")),
            ("roots[1].id".to_owned(), "\"a\"".to_owned())
        );
    }

    #[test]
    fn unknown_keys_and_wrong_types_are_parse_errors() {
        for text in [
            "lisen = \"127.0.0.1:1\"",
            "[media]\nheavy_jobs = \"two\"",
            "[[roots]]\nid = \"a\"\ndisplay_name = \"x\"\ntype = \"local\"\npath = \"/m\"\nextra = 1",
            "[[roots]]\nid = \"a\"\ndisplay_name = \"x\"\ntype = \"ftp\"",
        ] {
            let err = parse(text).unwrap_err();
            assert!(matches!(err, ConfigError::Parse { .. }), "{text}: {err}");
        }
    }

    fn with_listen_env(file: &str, env: Option<&str>) -> (SocketAddr, Option<SocketAddr>) {
        let (config, replaced) = parse(file)
            .unwrap()
            .with_listen_env(env.map(OsString::from))
            .unwrap();
        (config.listen, replaced)
    }

    #[test]
    fn unset_listen_env_keeps_the_file_or_default() {
        let file = "listen = \"0.0.0.0:9000\"";
        assert_eq!(
            with_listen_env(file, None),
            ("0.0.0.0:9000".parse().unwrap(), None)
        );
        assert_eq!(
            with_listen_env("", None),
            ("127.0.0.1:8888".parse().unwrap(), None)
        );
    }

    #[test]
    fn empty_listen_env_counts_as_unset() {
        assert_eq!(
            with_listen_env("listen = \"0.0.0.0:9000\"", Some("")),
            ("0.0.0.0:9000".parse().unwrap(), None)
        );
    }

    #[test]
    fn listen_env_wins_and_reports_a_different_file_value() {
        let file = "listen = \"127.0.0.1:9000\"";
        assert_eq!(
            with_listen_env(file, Some("0.0.0.0:8888")),
            (
                "0.0.0.0:8888".parse().unwrap(),
                Some("127.0.0.1:9000".parse().unwrap())
            )
        );
        // Nothing to warn about when the file agrees or says nothing.
        assert_eq!(
            with_listen_env(file, Some("127.0.0.1:9000")),
            ("127.0.0.1:9000".parse().unwrap(), None)
        );
        assert_eq!(
            with_listen_env("", Some("[::]:8888")),
            ("[::]:8888".parse().unwrap(), None)
        );
    }

    #[test]
    fn invalid_listen_env_names_the_variable_like_listen() {
        let err = parse("")
            .unwrap()
            .with_listen_env(Some("localhost:8888".into()))
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid value for KIKOERU_LISTEN: \"localhost:8888\" \
             (expected an address such as \"127.0.0.1:8888\")"
        );
        let file_err = invalid_param("listen = \"localhost:8888\"");
        assert_eq!(file_err.1, "\"localhost:8888\"");
    }

    #[test]
    fn database_defaults_to_the_sqlite_file_in_the_data_dir() {
        assert_eq!(
            parse("").unwrap().database(Path::new("/data")),
            neo_db::Location::Sqlite("/data/neo.sqlite3".into())
        );
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn database_url_selects_postgres_and_the_env_wins() {
        let file = parse("[database]\nurl = \"postgres:///kikoeru?host=/run/postgresql\"").unwrap();
        assert_eq!(
            file.clone().database(Path::new("/data")),
            neo_db::Location::Postgres("postgres:///kikoeru?host=/run/postgresql".into())
        );
        // An empty variable counts as unset, as for KIKOERU_LISTEN.
        let kept = file.clone().with_database_env(Some("".into())).unwrap();
        assert_eq!(kept.database_url, file.database_url);
        let env = file
            .with_database_env(Some("postgresql://k:pw@db/kikoeru".into()))
            .unwrap();
        assert_eq!(
            env.database(Path::new("/data")),
            neo_db::Location::Postgres("postgresql://k:pw@db/kikoeru".into())
        );
        assert_eq!(format!("{:?}", env.database_url), "Some(DatabaseUrl(..))");
    }

    #[test]
    fn a_database_url_that_is_not_postgres_is_rejected_without_echoing_it() {
        let (param, value) = invalid_param("[database]\nurl = \"mysql://k:secret@db/k\"");
        assert_eq!(
            (param.as_str(), value.as_str()),
            ("database.url", "(not shown)")
        );
        let err = parse("")
            .unwrap()
            .with_database_env(Some("sqlite:/data/neo.sqlite3".into()))
            .unwrap_err();
        assert!(err.to_string().contains("KIKOERU_DATABASE_URL"), "{err}");
        assert!(!err.to_string().contains("/data/neo.sqlite3"), "{err}");
    }

    #[test]
    fn tool_paths_default_to_path_and_accept_absolute_paths() {
        let defaults = parse("").unwrap();
        assert_eq!(
            (defaults.media.ffmpeg, defaults.media.ffprobe),
            (None, None)
        );
        let set =
            parse("[media]\nffmpeg = \"/opt/ff/ffmpeg\"\nffprobe = \"/opt/ff/ffprobe\"").unwrap();
        assert_eq!(
            set.media.ffmpeg.as_deref(),
            Some(Path::new("/opt/ff/ffmpeg"))
        );
        assert_eq!(
            set.media.ffprobe.as_deref(),
            Some(Path::new("/opt/ff/ffprobe"))
        );
    }
}
