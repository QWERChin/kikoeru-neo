//! Client settings. Defaults follow the WebDAV section of the design book.

use std::time::Duration;

use url::Url;

#[derive(Clone)]
pub struct Config {
    /// The server's WebDAV root, such as `https://nas.lan/dav`. Paths given
    /// to the client are below it.
    pub url: Url,
    pub username: String,
    pub password: String,
    /// `PROPFIND` requests in flight at once.
    pub list_concurrency: usize,
    /// `GET` transfers open at once. A player can hold one open for
    /// minutes, so transfers do not share the listing limit.
    pub max_transfers: usize,
    /// Timeout for a `PROPFIND`, and for a `GET` until its headers arrive.
    pub request_timeout: Duration,
    /// How long one call may wait out 429 and 503 pauses.
    pub cooldown_budget: Duration,
    /// Attempts for connection errors and retryable statuses.
    pub attempts: u32,
    /// First retry delay. Later retries double it, with full jitter.
    pub retry_delay: Duration,
    /// Largest `PROPFIND` response accepted. An entry is roughly 600 bytes,
    /// so this fits a directory of about 100 000 entries.
    pub max_response_bytes: usize,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("url", &self.url.as_str())
            .field("username", &self.username)
            .field("list_concurrency", &self.list_concurrency)
            .field("max_transfers", &self.max_transfers)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum ConfigError {
    #[error("the WebDAV url must be an http or https URL with a host")]
    Url,
    #[error("the WebDAV url must not carry a user name, password, query or fragment")]
    UrlParts,
    #[error("{0} must be greater than 0")]
    Zero(&'static str),
}

impl Config {
    pub fn new(url: Url, username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            url,
            username: username.into(),
            password: password.into(),
            list_concurrency: 4,
            max_transfers: 16,
            request_timeout: Duration::from_secs(30),
            cooldown_budget: Duration::from_secs(120),
            attempts: 3,
            retry_delay: Duration::from_millis(500),
            max_response_bytes: 64 << 20,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), ConfigError> {
        if !matches!(self.url.scheme(), "http" | "https") || self.url.host().is_none() {
            return Err(ConfigError::Url);
        }
        // Credentials go in the Authorization header only, and a query
        // would end up in every request path.
        if !self.url.username().is_empty()
            || self.url.password().is_some()
            || self.url.query().is_some()
            || self.url.fragment().is_some()
        {
            return Err(ConfigError::UrlParts);
        }
        for (name, value) in [
            ("list_concurrency", self.list_concurrency),
            ("max_transfers", self.max_transfers),
            ("attempts", self.attempts as usize),
        ] {
            if value == 0 {
                return Err(ConfigError::Zero(name));
            }
        }
        Ok(())
    }
}
