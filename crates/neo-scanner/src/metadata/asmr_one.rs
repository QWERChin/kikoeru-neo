//! asmr.one as a fallback metadata source for works DLsite cannot answer
//! for (book/src/scanning.md, "备用来源"). It follows kikoeru 0.6.17's
//! `src/scraper/asmrOne.ts`: the same mirrors in the same order, the same
//! `locale=zh-cn` cookie, and the kikoeru `workInfo` answer.

use std::time::{Duration, SystemTime};

use neo_core::date::is_calendar_date;
use reqwest::{Client, ClientBuilder, StatusCode, header};
use serde_json::Value;
use tokio::{sync::Mutex, time::Instant};
use url::Url;

use super::{
    Circle, MetadataError, MetadataProvider, MetadataSource, RawTag, WorkMetadata,
    proxy::{self, ProxyChoice, SharedClient},
};

/// The mirrors 0.6.17 tries, in its order.
pub const DEFAULT_MIRRORS: [&str; 4] = [
    "https://api.asmr.one/",
    "https://api.asmr-200.com/",
    "https://api.asmr-100.com/",
    "https://api.asmr-300.com/",
];

#[derive(Debug, Clone)]
pub struct AsmrOneConfig {
    /// Probed in order; each ends with `/`.
    pub mirrors: Vec<Url>,
    /// The least time between two requests. asmr.one is run by one person,
    /// so it is slower than DLsite's.
    pub spacing: Duration,
    pub retries: u32,
    pub backoff: Duration,
    pub timeout: Duration,
    /// How long a mirror may take to answer the probe.
    pub probe_timeout: Duration,
    /// How long a chosen mirror is used before probing again.
    pub mirror_ttl: Duration,
    /// The longest one call waits out 429 and 503 answers.
    pub wait_budget: Duration,
    /// The proxy at start. [`MetadataProvider::set_proxy`] replaces it.
    pub proxy: ProxyChoice,
}

impl Default for AsmrOneConfig {
    fn default() -> Self {
        Self {
            mirrors: DEFAULT_MIRRORS
                .iter()
                .map(|m| Url::parse(m).expect("static URL"))
                .collect(),
            spacing: Duration::from_secs(2),
            retries: 3,
            backoff: Duration::from_secs(1),
            timeout: Duration::from_secs(20),
            probe_timeout: Duration::from_secs(5),
            mirror_ttl: Duration::from_secs(20 * 60),
            wait_budget: Duration::from_secs(120),
            proxy: ProxyChoice::Environment,
        }
    }
}

pub struct AsmrOneProvider {
    client: SharedClient,
    config: AsmrOneConfig,
    last: Mutex<Option<Instant>>,
    /// The mirror in use and when it was chosen.
    chosen: Mutex<Option<(Url, Instant)>>,
}

/// Sent with every request, as 0.6.17 does, so tag names come in Chinese.
const LOCALE_COOKIE: &str = "locale=zh-cn";

impl AsmrOneProvider {
    pub fn new(config: AsmrOneConfig) -> Result<Self, MetadataError> {
        let client = proxy::build(builder(&config), &config.proxy)?;
        Ok(Self {
            client: SharedClient::new(client),
            config,
            last: Mutex::new(None),
            chosen: Mutex::new(None),
        })
    }

    async fn pace(&self) {
        let mut last = self.last.lock().await;
        if let Some(at) = *last {
            tokio::time::sleep_until(at + self.config.spacing).await;
        }
        *last = Some(Instant::now());
    }

    /// The mirror to use: the cached one while it is fresh, otherwise the
    /// first that answers the probe. Any HTTP answer counts, because
    /// `workInfo/1`, the probe 0.6.17 uses, answers 404.
    async fn mirror(&self) -> Result<Url, MetadataError> {
        let mut chosen = self.chosen.lock().await;
        if let Some((url, at)) = chosen.as_ref()
            && at.elapsed() < self.config.mirror_ttl
        {
            return Ok(url.clone());
        }
        for mirror in &self.config.mirrors {
            let Ok(probe) = mirror.join("api/workInfo/1") else {
                continue;
            };
            self.pace().await;
            let answer = self
                .client
                .get()
                .get(probe)
                .header(header::COOKIE, LOCALE_COOKIE)
                .timeout(self.config.probe_timeout)
                .send()
                .await;
            match answer {
                Ok(response) if !response.status().is_server_error() => {
                    tracing::info!(mirror = %mirror, "using this asmr.one mirror");
                    *chosen = Some((mirror.clone(), Instant::now()));
                    return Ok(mirror.clone());
                }
                Ok(response) => {
                    tracing::warn!(mirror = %mirror, status = %response.status(), "asmr.one mirror unavailable");
                }
                Err(e) => {
                    tracing::warn!(mirror = %mirror, error = %e, "asmr.one mirror unavailable")
                }
            }
        }
        Err(MetadataError::Unavailable(
            "no asmr.one mirror answered".into(),
        ))
    }

    /// GET with spacing, retries and Retry-After, body included. 404 is a
    /// definite answer; other 4xx answers are not retried either.
    async fn get(&self, url: Url) -> Result<Vec<u8>, MetadataError> {
        let mut delay = self.config.backoff;
        let mut budget = self.config.wait_budget;
        let mut attempt = 0;
        loop {
            self.pace().await;
            let answer = self
                .client
                .get()
                .get(url.clone())
                .header(header::COOKIE, LOCALE_COOKIE)
                .send()
                .await;
            let mut wait = None;
            let error = match answer {
                Ok(response) if response.status().is_success() => match response.bytes().await {
                    Ok(body) => return Ok(body.to_vec()),
                    Err(e) => format!("{url}: {e}"),
                },
                // A redirect the client did not follow left the mirrors,
                // which counts as no such file, not as an outage.
                Ok(response)
                    if response.status() == StatusCode::NOT_FOUND
                        || response.status().is_redirection() =>
                {
                    return Err(MetadataError::NotFound);
                }
                Ok(response)
                    if matches!(
                        response.status(),
                        StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE
                    ) =>
                {
                    wait = retry_after(response.headers());
                    format!("{url}: HTTP {}", response.status())
                }
                Ok(response) if response.status().is_client_error() => {
                    return Err(MetadataError::Unavailable(format!(
                        "{url}: HTTP {}",
                        response.status()
                    )));
                }
                Ok(response) => format!("{url}: HTTP {}", response.status()),
                Err(e) => format!("{url}: {e}"),
            };
            if attempt >= self.config.retries {
                return Err(MetadataError::Unavailable(error));
            }
            attempt += 1;
            let pause = wait.unwrap_or(delay);
            if pause > budget {
                return Err(MetadataError::Unavailable(format!(
                    "{error}, and asked to wait {pause:?}"
                )));
            }
            budget -= pause;
            tracing::debug!(%error, attempt, "retrying an asmr.one request");
            tokio::time::sleep(pause).await;
            delay *= 2;
        }
    }
}

fn builder(config: &AsmrOneConfig) -> ClientBuilder {
    let origins: Vec<url::Origin> = config.mirrors.iter().map(Url::origin).collect();
    let redirects = reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= 5 {
            attempt.error("too many redirects")
        } else if origins.contains(&attempt.url().origin()) {
            attempt.follow()
        } else {
            attempt.stop()
        }
    });
    Client::builder()
        .timeout(config.timeout)
        .redirect(redirects)
        .user_agent(concat!("kikoeru-neo/", env!("CARGO_PKG_VERSION")))
}

/// `Retry-After` as seconds or an HTTP date.
pub(super) fn retry_after(headers: &header::HeaderMap) -> Option<Duration> {
    let value = headers.get(header::RETRY_AFTER)?.to_str().ok()?.trim();
    if let Ok(secs) = value.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    httpdate::parse_http_date(value)
        .ok()
        .map(|at| at.duration_since(SystemTime::now()).unwrap_or_default())
}

/// The number asmr.one files a work under: the code's digits without
/// leading zeros, so RJ01357913 is 1357913. Only RJ works: asmr.one files
/// works by number alone, so BJ123456 would get RJ123456's record.
pub(super) fn work_number(code: &str) -> Option<u64> {
    if !code.starts_with("RJ") {
        return None;
    }
    code.get(2..)?.parse().ok().filter(|n| *n > 0)
}

/// Whether the answer is about this work: its `source_id`, when given,
/// names the same RJ number.
fn same_work(info: &Value, number: u64) -> bool {
    match info.get("source_id").and_then(Value::as_str) {
        None => true,
        Some(id) => id.strip_prefix("RJ").and_then(|n| n.parse::<u64>().ok()) == Some(number),
    }
}

#[async_trait::async_trait]
impl MetadataProvider for AsmrOneProvider {
    async fn fetch(&self, code: &str) -> Result<WorkMetadata, MetadataError> {
        let number = work_number(code).ok_or(MetadataError::NotFound)?;
        let mirror = self.mirror().await?;
        let url = mirror
            .join(&format!("api/workInfo/{number}"))
            .map_err(|e| MetadataError::Unavailable(e.to_string()))?;
        let body = self.get(url.clone()).await?;
        let info: Value = serde_json::from_slice(&body)
            .map_err(|e| MetadataError::Unavailable(format!("{url}: {e}")))?;
        if !same_work(&info, number) {
            tracing::warn!(code, "asmr.one answered about another work");
            return Err(MetadataError::NotFound);
        }
        parse_work_info(&info, &mirror)
            .ok_or_else(|| MetadataError::Unavailable(format!("{url}: no title in the answer")))
    }

    async fn cover(&self, url: &Url) -> Result<Vec<u8>, MetadataError> {
        if !self.serves(url) {
            tracing::warn!(%url, "refusing a cover outside the asmr.one mirrors");
            return Err(MetadataError::NotFound);
        }
        self.get(url.clone()).await
    }

    /// The chosen mirror goes too: the one that answered before may not be
    /// the first to answer through the new proxy.
    async fn set_proxy(&self, proxy: &ProxyChoice) -> Result<(), MetadataError> {
        self.client
            .replace(proxy::build(builder(&self.config), proxy)?);
        *self.chosen.lock().await = None;
        Ok(())
    }

    fn serves(&self, url: &Url) -> bool {
        matches!(url.scheme(), "https" | "http")
            && self
                .config
                .mirrors
                .iter()
                .any(|m| m.origin() == url.origin())
    }
}

fn parse_work_info(info: &Value, mirror: &Url) -> Option<WorkMetadata> {
    let title = info.get("title")?.as_str()?.trim().to_owned();
    if title.is_empty() {
        return None;
    }
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_owned);
    // The maker code when there is one, else the top-level number, which
    // is an RG number.
    let nested = info.get("circle");
    let circle_id = nested
        .and_then(|c| text(c, "source_id"))
        .and_then(|id| super::dlsite::circle_id(&id))
        .or_else(|| {
            info.get("circle_id")
                .and_then(Value::as_i64)
                .filter(|n| *n > 0)
        });
    let circle = circle_id.map(|id| Circle {
        id,
        name: nested
            .and_then(|c| text(c, "name"))
            .or_else(|| text(info, "name"))
            .unwrap_or_default(),
    });
    let vas = info
        .get("vas")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| v.get("name")?.as_str())
        .map(|n| n.trim().to_owned())
        .filter(|n| !n.is_empty())
        .collect();
    let tags = info
        .get("tags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| {
            let name = t
                .pointer("/i18n/zh-cn/name")
                .and_then(Value::as_str)
                .or_else(|| t.get("name")?.as_str())?;
            // asmr.one's tag IDs are not DLsite's, so only the name counts.
            Some(RawTag {
                name: name.trim().to_owned(),
                dlsite_id: None,
            })
        })
        .filter(|t| !t.name.is_empty())
        .collect();
    let non_empty = |key: &str| {
        info.get(key)
            .filter(|v| v.as_array().is_some_and(|a| !a.is_empty()))
            .cloned()
    };
    // The cover must come from the mirror that answered: same scheme, host
    // and port.
    let cover_url = info
        .get("mainCoverUrl")
        .and_then(Value::as_str)
        .and_then(|u| Url::parse(u).ok())
        .filter(|u| u.origin() == mirror.origin());
    Some(WorkMetadata {
        source: MetadataSource::AsmrOne,
        title,
        circle,
        release_date: info
            .get("release")
            .and_then(Value::as_str)
            .filter(|d| is_calendar_date(d))
            .map(str::to_owned),
        nsfw: info.get("nsfw").and_then(Value::as_bool).unwrap_or(false),
        vas,
        tags,
        dl_count: info.get("dl_count").and_then(Value::as_i64),
        price: info.get("price").and_then(Value::as_i64),
        rate_average: info.get("rate_average_2dp").and_then(Value::as_f64),
        rate_count: info.get("rate_count").and_then(Value::as_i64),
        rank: non_empty("rank"),
        rate_count_detail: non_empty("rate_count_detail"),
        cover_url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_numbers_drop_the_prefix_and_leading_zeros() {
        assert_eq!(work_number("RJ01357913"), Some(1_357_913));
        assert_eq!(work_number("RJ123456"), Some(123_456));
        assert_eq!(work_number("RJ"), None);
        assert_eq!(work_number("RJ00000000"), None);
        assert_eq!(work_number("BJ123456"), None, "not RJ123456's record");
        assert_eq!(work_number("VJ01246802"), None);
    }

    #[test]
    fn an_answer_about_another_work_is_not_used() {
        let info = |id: &str| serde_json::json!({"source_id": id});
        assert!(same_work(&info("RJ01357913"), 1_357_913));
        assert!(!same_work(&info("RJ01357914"), 1_357_913));
        assert!(!same_work(&info("VJ01357913"), 1_357_913));
        assert!(same_work(&serde_json::json!({}), 1_357_913));
    }

    /// The shape of a real answer from 2026-10-12, trimmed.
    #[test]
    fn a_work_info_answer_becomes_metadata() {
        let mirror = Url::parse("https://api.asmr.one/").unwrap();
        let info = serde_json::json!({
            "id": 1357913,
            "title": "作品",
            "circle_id": 1054416,
            "name": "社团",
            "nsfw": true,
            "release": "2026-06-13",
            "dl_count": 331,
            "price": 2200,
            "rate_average_2dp": 0,
            "rate_count": 0,
            "rate_count_detail": [],
            "rank": null,
            "vas": [{"id": "x", "name": "テスト声優。"}],
            "tags": [{"id": 99, "name": "兔女郎", "i18n": {"zh-cn": {"name": "兔女郎"}, "ja-jp": {"name": "バニーガール"}}}],
            "circle": {"id": 1054416, "name": "社团", "source_id": "RG01054416", "source_type": "DLSITE"},
            "mainCoverUrl": "https://api.asmr.one/api/cover/1357913.jpg?type=main"
        });
        let meta = parse_work_info(&info, &mirror).unwrap();
        assert_eq!(meta.source, MetadataSource::AsmrOne);
        assert_eq!(meta.circle.unwrap().id, 1_054_416);
        assert_eq!(meta.release_date.as_deref(), Some("2026-06-13"));
        let mut impossible = info.clone();
        impossible["release"] = "2026-02-29".into();
        assert_eq!(
            parse_work_info(&impossible, &mirror).unwrap().release_date,
            None
        );
        assert_eq!(meta.tags[0].name, "兔女郎");
        assert_eq!(meta.tags[0].dlsite_id, None);
        assert_eq!(meta.rate_average, Some(0.0));
        assert_eq!(meta.rank, None);
        assert!(meta.cover_url.is_some());

        // Without a maker code, the top-level number is the RG one.
        let mut bare = info.clone();
        bare.as_object_mut().unwrap().remove("circle");
        let circle = parse_work_info(&bare, &mirror).unwrap().circle.unwrap();
        assert_eq!((circle.id, circle.name.as_str()), (1_054_416, "社团"));

        // A cover on another host is dropped.
        let mut elsewhere = info.clone();
        elsewhere["mainCoverUrl"] = "http://10.0.0.1/cover.jpg".into();
        assert!(
            parse_work_info(&elsewhere, &mirror)
                .unwrap()
                .cover_url
                .is_none()
        );
    }
}
