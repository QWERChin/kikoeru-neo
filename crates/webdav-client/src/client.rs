//! Requests: `PROPFIND` for listings and `stat`, `GET` for file bytes.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use http::{
    HeaderMap, HeaderValue, Method, StatusCode,
    header::{ACCEPT_ENCODING, CONTENT_TYPE, IF_RANGE, LOCATION, RANGE, RETRY_AFTER},
};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore},
    time::Instant,
};
use url::Url;

use crate::{
    config::{Config, ConfigError},
    error::{Error, is_retryable},
    xml::{self, Member},
};

const MAX_REDIRECTS: usize = 5;
const MAX_RETRY_DELAY: Duration = Duration::from_secs(10);
const MAX_COOLDOWN: Duration = Duration::from_secs(24 * 60 * 60);
const PATH_ESCAPES: &AsciiSet = &CONTROLS.add(b'%');

/// One child of a listed collection, or the target of a `stat`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The decoded name, a single path segment.
    pub name: String,
    pub is_dir: bool,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    /// Many servers change a collection's ETag when anything below it
    /// changes, which makes it a better change marker than the time.
    pub etag: Option<String>,
}

/// An open `GET`. Dropping the body cancels the request and frees the
/// transfer slot.
pub struct Transfer {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: std::pin::Pin<Box<dyn Stream<Item = Result<Bytes, Error>> + Send>>,
}

#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    config: Config,
    /// The root URL with a trailing slash, so paths extend it.
    base: Url,
    lists: Semaphore,
    transfers: Arc<Semaphore>,
    /// No request starts before this instant, after a 429 or 503.
    cooldown: Mutex<Instant>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("config", &self.inner.config)
            .finish_non_exhaustive()
    }
}

impl Client {
    pub fn new(config: Config) -> Result<Self, ConfigError> {
        config.validate()?;
        let mut base = config.url.clone();
        if !base.path().ends_with('/') {
            let path = format!("{}/", base.path());
            base.set_path(&path);
        }
        // Redirects are followed by hand, only within the origin, because
        // every request carries the Basic credentials.
        // The read timeout bounds every stall, including in the middle of a
        // file a player holds open, so a hung server frees its slot.
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(config.request_timeout)
            .read_timeout(config.request_timeout)
            .build()
            .map_err(|_| ConfigError::Url)?;
        Ok(Self {
            inner: Arc::new(Inner {
                http,
                base,
                lists: Semaphore::new(config.list_concurrency),
                transfers: Arc::new(Semaphore::new(config.max_transfers)),
                cooldown: Mutex::new(Instant::now()),
                config,
            }),
        })
    }

    pub fn config(&self) -> &Config {
        &self.inner.config
    }

    /// The direct children of the collection at `path`, a `/`-separated
    /// path below the root such as `/音声/RJ01000001`.
    pub async fn list(&self, path: &str) -> Result<Vec<Entry>, Error> {
        let url = self.url_for(path, true)?;
        let (members, url) = self.propfind(path, &url, "1").await?;
        let own = segments(&url);
        let mut entries = Vec::with_capacity(members.len());
        let mut saw_self = false;
        for member in members {
            let target = self.href_target(path, &url, &member.href)?;
            if target == own {
                saw_self = true;
                // A collection the server would not describe, such as a
                // 403 or 423 in its own response, is an outage, not an
                // empty directory.
                if !member.described {
                    return Err(Error::Http {
                        status: member.status.unwrap_or(500),
                        path: path.to_owned(),
                    });
                }
                if !member.collection {
                    return Err(Error::protocol(path, "not a collection"));
                }
                continue;
            }
            if target.len() != own.len() + 1 || target[..own.len()] != own[..] {
                return Err(Error::protocol(path, "the listing names a path outside it"));
            }
            let name = target.last().cloned().unwrap_or_default();
            check_name(path, &name)?;
            entries.push(entry(path, name, &member)?);
        }
        if !saw_self && entries.is_empty() {
            // An empty multistatus is not a listing of an empty directory.
            return Err(Error::protocol(
                path,
                "the listing does not include the collection",
            ));
        }
        Ok(entries)
    }

    /// The file or collection at `path`.
    pub async fn stat(&self, path: &str) -> Result<Entry, Error> {
        let url = self.url_for(path, false)?;
        let (members, url) = self.propfind(path, &url, "0").await?;
        let own = segments(&url);
        let member = members
            .iter()
            .find(|m| {
                self.href_target(path, &url, &m.href)
                    .is_ok_and(|t| t == own)
            })
            .ok_or_else(|| Error::protocol(path, "the response does not describe the path"))?;
        let name = own.last().cloned().unwrap_or_default();
        entry(path, name, member)
    }

    /// Starts a `GET` of the file at `path`, forwarding `Range` and
    /// `If-Range` from `headers`. Answers 200, 206 and 416 are returned as
    /// they are; the caller forwards them.
    pub async fn open(&self, path: &str, headers: &HeaderMap) -> Result<Transfer, Error> {
        let url = self.url_for(path, false)?;
        let timeout = self.inner.config.request_timeout;
        // Waits for a slot as long as a request may take, then gives up, so
        // a pile of stalled players cannot queue everyone else forever.
        let permit: OwnedSemaphorePermit =
            tokio::time::timeout(timeout, self.inner.transfers.clone().acquire_owned())
                .await
                .map_err(|_| Error::Transport {
                    path: path.to_owned(),
                    message: "too many transfers are open".into(),
                })?
                .map_err(|_| Error::protocol(path, "client closed"))?;
        let mut forwarded = HeaderMap::new();
        for name in [RANGE, IF_RANGE] {
            if let Some(value) = headers.get(&name) {
                forwarded.insert(name, value.clone());
            }
        }
        // The bytes are forwarded as the file, so they must not be
        // compressed on the way.
        forwarded.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
        let response = self
            .send(path, &url, |http, url| {
                http.get(url).headers(forwarded.clone())
            })
            .await?;
        let status = response.status();
        match status.as_u16() {
            200 | 206 | 416 => {}
            404 | 410 => {
                return Err(Error::NotFound {
                    path: path.to_owned(),
                });
            }
            code => {
                return Err(Error::Http {
                    status: code,
                    path: path.to_owned(),
                });
            }
        }
        let headers = response.headers().clone();
        let owned_path = path.to_owned();
        let body = response
            .bytes_stream()
            .map(move |chunk| {
                // The permit lives as long as the stream.
                let _held = &permit;
                chunk.map_err(|e| Error::Transport {
                    path: owned_path.clone(),
                    message: e.to_string(),
                })
            })
            .boxed();
        Ok(Transfer {
            status,
            headers,
            body,
        })
    }

    async fn propfind(
        &self,
        path: &str,
        url: &Url,
        depth: &'static str,
    ) -> Result<(Vec<Member>, Url), Error> {
        let _permit = self
            .inner
            .lists
            .acquire()
            .await
            .map_err(|_| Error::protocol(path, "client closed"))?;
        let method = Method::from_bytes(b"PROPFIND").expect("a valid method");
        let response = self
            .send(path, url, |http, url| {
                http.request(method.clone(), url)
                    .header("Depth", depth)
                    .header(CONTENT_TYPE, "application/xml; charset=utf-8")
                    .body(xml::PROPFIND_BODY)
                    .timeout(self.inner.config.request_timeout)
            })
            .await?;
        match response.status().as_u16() {
            207 => {}
            404 | 410 => {
                return Err(Error::NotFound {
                    path: path.to_owned(),
                });
            }
            code => {
                return Err(Error::Http {
                    status: code,
                    path: path.to_owned(),
                });
            }
        }
        // After a redirect the hrefs name the new location.
        let answered = response.url().clone();
        let body = read_limited(path, response, self.inner.config.max_response_bytes).await?;
        let members = xml::parse(&body).map_err(|message| Error::protocol(path, message))?;
        // Another successful property does not make a failed type query
        // evidence that the member is a file.
        if let Some(status) = members.iter().find_map(|m| m.propstat_error) {
            return Err(Error::Http {
                status,
                path: path.to_owned(),
            });
        }
        Ok((members, answered))
    }

    /// Sends a request with credentials, waiting out 429 and 503 pauses
    /// within the budget, retrying connection errors and retryable
    /// statuses, and following same-origin redirects.
    async fn send(
        &self,
        path: &str,
        url: &Url,
        build: impl Fn(&reqwest::Client, Url) -> reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, Error> {
        let config = &self.inner.config;
        let mut budget = config.cooldown_budget;
        let mut attempt = 0;
        loop {
            attempt += 1;
            self.wait_cooldown(&mut budget).await?;
            // Each attempt gets the request timeout until its headers
            // arrive. Cooldown waits stay outside it, under their budget.
            let result =
                tokio::time::timeout(config.request_timeout, self.follow(path, url, &build))
                    .await
                    .unwrap_or_else(|_| {
                        Err(Error::Transport {
                            path: path.to_owned(),
                            message: "no answer in time".into(),
                        })
                    });
            let retry = match &result {
                Ok(response) if is_retryable(response.status().as_u16()) => {
                    let status = response.status().as_u16();
                    if matches!(status, 429 | 503) {
                        let pause = retry_after(response.headers())
                            .unwrap_or_else(|| self.retry_delay(attempt));
                        self.extend_cooldown(pause);
                    }
                    true
                }
                Ok(_) => false,
                Err(Error::Transport { .. }) => true,
                Err(_) => false,
            };
            if !retry || attempt >= config.attempts {
                return result;
            }
            tokio::time::sleep(self.retry_delay(attempt)).await;
        }
    }

    async fn follow(
        &self,
        path: &str,
        url: &Url,
        build: &impl Fn(&reqwest::Client, Url) -> reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, Error> {
        let config = &self.inner.config;
        let mut current = url.clone();
        for _ in 0..=MAX_REDIRECTS {
            let response = build(&self.inner.http, current.clone())
                .basic_auth(&config.username, Some(&config.password))
                .send()
                .await
                .map_err(|e| Error::Transport {
                    path: path.to_owned(),
                    message: e.without_url().to_string(),
                })?;
            if !response.status().is_redirection() || response.status() == StatusCode::NOT_MODIFIED
            {
                return Ok(response);
            }
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| current.join(v).ok())
                .ok_or_else(|| Error::Redirect {
                    path: path.to_owned(),
                    message: "a redirect without a usable Location".into(),
                })?;
            if location.origin() != self.inner.base.origin() {
                return Err(Error::Redirect {
                    path: path.to_owned(),
                    message: "to another origin".into(),
                });
            }
            current = location;
        }
        Err(Error::Redirect {
            path: path.to_owned(),
            message: "too many redirects".into(),
        })
    }

    /// Waits until the shared pause is over, charging the wait to the
    /// call's budget. Another request can extend the pause meanwhile, so
    /// the deadline is read again after every sleep.
    async fn wait_cooldown(&self, budget: &mut Duration) -> Result<(), Error> {
        loop {
            let until = *self.inner.cooldown.lock().expect("cooldown mutex poisoned");
            let wait = until.saturating_duration_since(Instant::now());
            if wait.is_zero() {
                return Ok(());
            }
            if wait > *budget {
                return Err(Error::RateLimited);
            }
            *budget -= wait;
            tokio::time::sleep(wait).await;
        }
    }

    /// Keeps the server's deadline even past the wait budget, so later
    /// calls fail fast instead of contacting it early. A Retry-After can be
    /// anything, so it is capped at a day to stay clear of `Instant`
    /// overflow.
    fn extend_cooldown(&self, pause: Duration) {
        let pause = pause.min(MAX_COOLDOWN);
        let mut until = self.inner.cooldown.lock().expect("cooldown mutex poisoned");
        *until = (*until).max(Instant::now() + pause);
    }

    /// Exponential backoff with full jitter.
    fn retry_delay(&self, attempt: u32) -> Duration {
        let ceiling = self
            .inner
            .config
            .retry_delay
            .saturating_mul(1 << attempt.saturating_sub(1).min(16))
            .min(MAX_RETRY_DELAY);
        let entropy = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| u64::from(d.subsec_nanos()))
            .unwrap_or(0);
        let ceiling_micros = u64::try_from(ceiling.as_micros()).unwrap_or(u64::MAX);
        Duration::from_micros(entropy % ceiling_micros.saturating_add(1))
    }

    /// The URL of `path` below the root. Collections get a trailing slash,
    /// which many servers otherwise answer with a redirect.
    fn url_for(&self, path: &str, collection: bool) -> Result<Url, Error> {
        let mut url = self.inner.base.clone();
        let mut encoded = url.path().to_owned();
        encoded.pop();
        for segment in path.split('/').filter(|s| !s.is_empty()) {
            check_name(path, segment)?;
            encoded.push('/');
            // URL parsing strips TAB/CR/LF before resolving dot segments.
            // Escape controls and literal '%' first; set_path preserves these escapes.
            encoded.extend(utf8_percent_encode(segment, PATH_ESCAPES));
        }
        if collection {
            encoded.push('/');
        }
        url.set_path(&encoded);
        Ok(url)
    }

    /// The decoded path segments an `href` points at. It may be an
    /// absolute URL or an absolute path; either way it must stay on the
    /// server's origin. The segments are checked as written, before any
    /// URL normalization could fold `%2e` or `a\..` away, and must decode
    /// to UTF-8: a name changed on the way would address another file.
    fn href_target(&self, path: &str, request: &Url, href: &str) -> Result<Vec<String>, Error> {
        let raw = if href.starts_with('/') {
            href
        } else {
            let url = Url::parse(href).map_err(|_| Error::protocol(path, "an unreadable href"))?;
            if url.origin() != request.origin() {
                return Err(Error::protocol(path, "an href on another origin"));
            }
            let after_scheme = href.split_once("://").map_or("", |(_, rest)| rest);
            after_scheme.find('/').map_or("/", |i| &after_scheme[i..])
        };
        let raw = raw.split(['?', '#']).next().unwrap_or_default();
        raw.split('/')
            .filter(|segment| !segment.is_empty())
            .map(|segment| {
                let name = percent_encoding::percent_decode_str(segment)
                    .decode_utf8()
                    .map_err(|_| Error::protocol(path, "an href that is not UTF-8"))?
                    .into_owned();
                check_name(path, &name)?;
                Ok(name)
            })
            .collect()
    }
}

/// Decoded path segments, without the empty one a trailing slash leaves.
fn segments(url: &Url) -> Vec<String> {
    let mut out: Vec<String> = url
        .path_segments()
        .map(|s| {
            s.map(|segment| {
                percent_encoding::percent_decode_str(segment)
                    .decode_utf8_lossy()
                    .into_owned()
            })
            .collect()
        })
        .unwrap_or_default();
    if out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out
}

/// Names that could step outside the root, or that a path cannot carry.
fn check_name(path: &str, name: &str) -> Result<(), Error> {
    let bad = name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']);
    if bad {
        return Err(Error::InvalidPath {
            path: format!("{path}: {name:?}"),
        });
    }
    Ok(())
}

fn entry(path: &str, name: String, member: &Member) -> Result<Entry, Error> {
    if !member.described {
        // The server listed the member but would not describe it, which
        // says nothing about whether it is a file.
        return Err(Error::Http {
            status: member.status.unwrap_or(500),
            path: format!("{path}/{name}"),
        });
    }
    Ok(Entry {
        name,
        is_dir: member.collection,
        size: (!member.collection).then_some(member.size).flatten(),
        modified: member.modified,
        etag: member.etag.clone(),
    })
}

async fn read_limited(
    path: &str,
    response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, Error> {
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| Error::Transport {
            path: path.to_owned(),
            message: e.without_url().to_string(),
        })?;
        if body.len() + chunk.len() > limit {
            return Err(Error::protocol(path, "the listing is too large"));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// `Retry-After` as seconds or an HTTP date.
fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let value = headers.get(RETRY_AFTER)?.to_str().ok()?.trim();
    if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
        return Some(Duration::from_secs(value.parse().unwrap_or(u64::MAX)));
    }
    httpdate::parse_http_date(value)
        .ok()
        .map(|date| date.duration_since(SystemTime::now()).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(url: &str) -> Client {
        Client::new(Config::new(url.parse().unwrap(), "u", "p")).unwrap()
    }

    #[test]
    fn paths_are_encoded_below_the_root() {
        let c = client("https://nas.lan/dav");
        assert_eq!(
            c.url_for("/音声/01 a#b?.mp3", false).unwrap().as_str(),
            "https://nas.lan/dav/%E9%9F%B3%E5%A3%B0/01%20a%23b%3F.mp3"
        );
        assert_eq!(
            c.url_for("/音声", true).unwrap().as_str(),
            "https://nas.lan/dav/%E9%9F%B3%E5%A3%B0/"
        );
        for bad in ["/a/../b", "/a\\b", "/./a"] {
            assert!(c.url_for(bad, false).is_err(), "{bad}");
        }
    }

    #[test]
    fn hrefs_are_checked_before_normalization_and_decoded_strictly() {
        let c = client("https://nas.lan/dav/");
        let request = c.url_for("/x", true).unwrap();
        for bad in [
            "/dav/x/%2e/",
            "/dav/x/a\\..\\t.mp3",
            "/dav/x/%FF.mp3",
            "/dav/x/a%2Fb",
        ] {
            assert!(c.href_target("/x", &request, bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn hrefs_resolve_as_paths_or_urls_on_the_same_origin_only() {
        let c = client("https://nas.lan/dav/");
        let request = c.url_for("/x", true).unwrap();
        let rel = c.href_target("/x", &request, "/dav/x/a%20b").unwrap();
        let abs = c
            .href_target("/x", &request, "https://nas.lan/dav/x/a%20b")
            .unwrap();
        assert_eq!(rel, abs);
        assert_eq!(rel.last().unwrap(), "a b");
        assert!(
            c.href_target("/x", &request, "https://evil.lan/dav/x/a")
                .is_err()
        );
    }
}
