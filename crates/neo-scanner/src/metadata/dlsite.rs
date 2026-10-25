//! DLsite as a metadata source. It reads the two JSON endpoints the old
//! server used instead of the HTML work page: `product.json` for the work
//! itself (title, circle, voice actors, genres, cover) and the product info
//! endpoint for the numbers that change (downloads, rating, rank, price).
//! Both answer in the requested language and need no HTML parsing.

use std::time::Duration;

use neo_core::date::is_calendar_date;
use reqwest::{Client, ClientBuilder, StatusCode};
use serde_json::Value;
use tokio::{sync::Mutex, time::Instant};
use url::Url;

use super::{
    Circle, MetadataError, MetadataProvider, MetadataSource, RawTag, WorkMetadata,
    proxy::{self, ProxyChoice, Reachable, SharedClient},
};

#[derive(Debug, Clone)]
pub struct DlsiteConfig {
    pub base: Url,
    /// DLsite's locale for names, such as `zh_CN`.
    pub locale: String,
    /// The least time between two requests (book: at least 1 second).
    pub spacing: Duration,
    /// Retries after the first attempt; each waits twice as long as the
    /// last, starting at `backoff`.
    pub retries: u32,
    pub backoff: Duration,
    pub timeout: Duration,
    /// Hosts the provider may contact besides `base`. Cover URLs come from
    /// DLsite's answers, so anything else (an internal address in a forged
    /// or tampered answer) is refused, and redirects are held to the same
    /// list.
    pub hosts: Vec<String>,
    /// The proxy at start. [`MetadataProvider::set_proxy`] replaces it.
    pub proxy: ProxyChoice,
}

impl Default for DlsiteConfig {
    fn default() -> Self {
        Self {
            base: Url::parse("https://www.dlsite.com/").expect("static URL"),
            // The old server's tagLanguage, so genre names match the
            // imported tags and the dictionary's canonical names.
            locale: "zh_CN".to_owned(),
            spacing: Duration::from_secs(1),
            retries: 3,
            backoff: Duration::from_secs(1),
            timeout: Duration::from_secs(20),
            hosts: vec!["img.dlsite.jp".to_owned()],
            proxy: ProxyChoice::Environment,
        }
    }
}

#[derive(Clone, Copy)]
enum Fetched {
    Api,
    Cover,
}

pub struct DlsiteProvider {
    client: SharedClient,
    config: DlsiteConfig,
    /// When the last request went out, shared by every caller so that
    /// concurrent scans and refreshes still keep the spacing.
    last: Mutex<Option<Instant>>,
}

impl DlsiteProvider {
    pub fn new(config: DlsiteConfig) -> Result<Self, MetadataError> {
        let client = proxy::build(builder(&config), &config.proxy)?;
        Ok(Self {
            client: SharedClient::new(client),
            config,
            last: Mutex::new(None),
        })
    }

    async fn pace(&self) {
        let mut last = self.last.lock().await;
        if let Some(at) = *last {
            tokio::time::sleep_until(at + self.config.spacing).await;
        }
        *last = Some(Instant::now());
    }

    /// GET with spacing and retries, body included, so a transfer that
    /// breaks off after the headers is retried too. 404 is a definite answer
    /// and is not retried; other 4xx answers are not either, since repeating
    /// them cannot help. `kind` says what a redirect the client did not
    /// follow means.
    async fn get(&self, url: Url, kind: Fetched) -> Result<Vec<u8>, MetadataError> {
        let mut delay = self.config.backoff;
        let mut attempt = 0;
        loop {
            self.pace().await;
            let error = match self.client.get().get(url.clone()).send().await {
                Ok(response) if response.status().is_success() => match response.bytes().await {
                    Ok(body) => return Ok(body.to_vec()),
                    Err(e) => format!("{url}: {e}"),
                },
                Ok(response) if response.status() == StatusCode::NOT_FOUND => {
                    return Err(MetadataError::NotFound);
                }
                // The client follows redirects within `hosts` only, so this
                // one points elsewhere, and asking again gets the same
                // answer. A cover there counts as no cover. The API sent
                // elsewhere, say to a maintenance page, is DLsite being down.
                Ok(response) if response.status().is_redirection() => {
                    return Err(match kind {
                        Fetched::Cover => MetadataError::NotFound,
                        Fetched::Api => MetadataError::Unavailable(format!(
                            "{url}: redirected off DLsite (HTTP {})",
                            response.status()
                        )),
                    });
                }
                Ok(response)
                    if response.status().is_client_error()
                        && response.status() != StatusCode::TOO_MANY_REQUESTS =>
                {
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
            tracing::debug!(%error, attempt, "retrying a DLsite request");
            tokio::time::sleep(delay).await;
            delay *= 2;
        }
    }

    async fn get_json(&self, url: Url) -> Result<Value, MetadataError> {
        let body = self.get(url.clone(), Fetched::Api).await?;
        serde_json::from_slice(&body).map_err(|e| MetadataError::Unavailable(format!("{url}: {e}")))
    }

    fn url(&self, path: &str, query: &[(&str, &str)]) -> Result<Url, MetadataError> {
        let mut url = self
            .config
            .base
            .join(path)
            .map_err(|e| MetadataError::Unavailable(e.to_string()))?;
        url.query_pairs_mut().extend_pairs(query);
        Ok(url)
    }
}

/// DLsite keeps each kind of work in its own section of the site.
fn section(code: &str) -> &'static str {
    match code.get(..2) {
        Some("BJ") => "books",
        Some("VJ") => "pro",
        _ => "maniax",
    }
}

#[async_trait::async_trait]
impl MetadataProvider for DlsiteProvider {
    async fn fetch(&self, code: &str) -> Result<WorkMetadata, MetadataError> {
        let section = section(code);
        let product = self.url(
            &format!("{section}/api/=/product.json"),
            &[("workno", code), ("locale", &self.config.locale)],
        )?;
        let product = match self.get_json(product).await? {
            Value::Array(mut items) if !items.is_empty() => items.swap_remove(0),
            Value::Array(_) => return Err(MetadataError::NotFound),
            other => {
                return Err(MetadataError::Unavailable(format!(
                    "product.json for {code} is not a list: {}",
                    truncate(&other.to_string())
                )));
            }
        };
        let mut meta = parse_product(&product).ok_or_else(|| {
            MetadataError::Unavailable(format!("product.json for {code} has no title"))
        })?;
        // The numbers are a separate request. Missing them is not a reason
        // to drop the rest.
        let info = self.url(
            &format!("{section}/product/info/ajax"),
            &[("product_id", code), ("locale", &self.config.locale)],
        )?;
        match self.get_json(info).await {
            Ok(info) => {
                if let Some(info) = info.get(code) {
                    apply_info(&mut meta, info);
                }
            }
            Err(MetadataError::NotFound) => {}
            Err(e) => tracing::warn!(code, error = %e, "DLsite product info unavailable"),
        }
        Ok(meta)
    }

    async fn cover(&self, url: &Url) -> Result<Vec<u8>, MetadataError> {
        if !self.serves(url) {
            // Not an outage: asking again gets the same address.
            tracing::warn!(%url, "refusing a cover outside DLsite");
            return Err(MetadataError::NotFound);
        }
        self.get(url.clone(), Fetched::Cover).await
    }

    async fn set_proxy(&self, proxy: &ProxyChoice) -> Result<(), MetadataError> {
        self.client
            .replace(proxy::build(builder(&self.config), proxy)?);
        Ok(())
    }

    /// The base itself, without following a redirect: a reset connection
    /// is what a blocked DLsite looks like, and a redirect still proves
    /// the way there works.
    async fn check(&self, proxy: &ProxyChoice) -> Result<Reachable, MetadataError> {
        let client = proxy::build(
            builder(&self.config)
                .redirect(reqwest::redirect::Policy::none())
                .timeout(proxy::CHECK_TIMEOUT),
            proxy,
        )?;
        let url = self.config.base.clone();
        self.pace().await;
        let started = Instant::now();
        let response = client
            .get(url.clone())
            .send()
            .await
            .map_err(|e| MetadataError::Unavailable(proxy::describe(&e)))?;
        Ok(Reachable {
            url,
            status: response.status().as_u16(),
            elapsed: started.elapsed(),
        })
    }

    /// The base's origin, or an extra host on its default port. The port
    /// counts too, so another service on the same host is not DLsite.
    fn serves(&self, url: &Url) -> bool {
        if !matches!(url.scheme(), "https" | "http") {
            return false;
        }
        let base = &self.config.base;
        let on_base = url.host_str() == base.host_str()
            && url.port_or_known_default() == base.port_or_known_default();
        on_base
            || (url.port().is_none()
                && url
                    .host_str()
                    .is_some_and(|h| self.config.hosts.iter().any(|a| a == h)))
    }
}

fn builder(config: &DlsiteConfig) -> ClientBuilder {
    let allowed = allowed_hosts(config);
    let redirects = reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= 5 {
            attempt.error("too many redirects")
        } else if allowed
            .iter()
            .any(|h| Some(h.as_str()) == attempt.url().host_str())
        {
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

fn allowed_hosts(config: &DlsiteConfig) -> Vec<String> {
    let mut hosts = config.hosts.clone();
    hosts.extend(config.base.host_str().map(str::to_owned));
    hosts
}

fn truncate(s: &str) -> String {
    s.chars().take(200).collect()
}

/// `RG12345` and its sibling prefixes, packed as the old server did
/// (`type * 10^9 + number`), so circles keep their imported IDs.
pub(super) fn circle_id(maker_id: &str) -> Option<i64> {
    let kind = match maker_id.get(..2)? {
        "RG" => 0,
        "BG" => 1,
        "VG" => 2,
        _ => return None,
    };
    let number: i64 = maker_id.get(2..)?.parse().ok()?;
    (number > 0).then_some(kind * 1_000_000_000 + number)
}

fn parse_product(p: &Value) -> Option<WorkMetadata> {
    let title = p.get("product_name")?.as_str()?;
    // Some answers carry the page title, "<name> [circle] | DLsite".
    let title = match title.rfind(" [") {
        Some(i) if title.ends_with("| DLsite") => &title[..i],
        _ => title,
    }
    .trim()
    .to_owned();
    if title.is_empty() {
        return None;
    }
    let text = |key: &str| p.get(key).and_then(Value::as_str);
    let circle = text("maker_id").and_then(circle_id).map(|id| Circle {
        id,
        name: text("maker_name").unwrap_or_default().to_owned(),
    });
    let release_date = text("regist_date")
        .and_then(|d| d.get(..10))
        .filter(|d| is_calendar_date(d))
        .map(str::to_owned);
    let vas = p
        .pointer("/creaters/voice_by")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| v.get("name")?.as_str())
        .map(|n| n.trim().to_owned())
        .filter(|n| !n.is_empty())
        .collect();
    let tags = p
        .get("genres")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|g| {
            Some(RawTag {
                name: g.get("name")?.as_str()?.trim().to_owned(),
                dlsite_id: g.get("id").and_then(Value::as_i64),
            })
        })
        .filter(|t| !t.name.is_empty())
        .collect();
    let cover_url = p
        .pointer("/image_main/url")
        .and_then(Value::as_str)
        .and_then(|u| {
            let u = if u.starts_with("//") {
                format!("https:{u}")
            } else {
                u.to_owned()
            };
            Url::parse(&u).ok()
        });
    Some(WorkMetadata {
        source: MetadataSource::Dlsite,
        title,
        circle,
        release_date,
        // DLsite's age category 3 is adult only; 1 is all ages, 2 is R-15.
        nsfw: p.get("age_category").and_then(Value::as_i64) == Some(3),
        vas,
        tags,
        dl_count: None,
        price: p.get("price").and_then(Value::as_i64),
        rate_average: None,
        rate_count: None,
        rank: None,
        rate_count_detail: None,
        cover_url,
    })
}

fn apply_info(meta: &mut WorkMetadata, info: &Value) {
    // dl_count arrives as a number or as a string.
    let number = |key: &str| {
        info.get(key).and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.replace(',', "").parse().ok()))
        })
    };
    meta.dl_count = number("dl_count").or(meta.dl_count);
    meta.price = number("price").or(meta.price);
    meta.rate_count = number("rate_count").or(meta.rate_count);
    meta.rate_average = info
        .get("rate_average_2dp")
        .and_then(Value::as_f64)
        .or(meta.rate_average);
    let non_empty = |key: &str| {
        info.get(key)
            .filter(|v| v.as_array().is_some_and(|a| !a.is_empty()))
            .cloned()
    };
    meta.rank = non_empty("rank");
    meta.rate_count_detail = non_empty("rate_count_detail");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With tokio's clock paused, the sleeps in `pace` advance it by
    /// exactly what they wait, so the spacing is checked without depending
    /// on how busy the machine is.
    #[tokio::test(start_paused = true)]
    async fn requests_keep_their_spacing() {
        let provider = std::sync::Arc::new(
            DlsiteProvider::new(DlsiteConfig {
                spacing: Duration::from_millis(60),
                ..DlsiteConfig::default()
            })
            .unwrap(),
        );
        let start = Instant::now();
        provider.pace().await;
        assert_eq!(
            Instant::now() - start,
            Duration::ZERO,
            "the first goes at once"
        );
        provider.pace().await;
        provider.pace().await;
        assert_eq!(Instant::now() - start, Duration::from_millis(120));
        // Concurrent callers share the spacing.
        let waiting: Vec<_> = (0..3)
            .map(|_| {
                let provider = std::sync::Arc::clone(&provider);
                tokio::spawn(async move {
                    provider.pace().await;
                    Instant::now()
                })
            })
            .collect();
        let mut times = Vec::new();
        for task in waiting {
            times.push(task.await.unwrap() - start);
        }
        times.sort();
        assert_eq!(
            times,
            [180, 240, 300].map(Duration::from_millis),
            "one spacing apart"
        );
    }

    #[test]
    fn circles_keep_the_old_numbering() {
        assert_eq!(circle_id("RG12345"), Some(12345));
        assert_eq!(circle_id("BG00001"), Some(1_000_000_001));
        assert_eq!(circle_id("VG3026"), Some(2_000_003_026));
        assert_eq!(circle_id("XX1"), None);
        assert_eq!(circle_id("RG"), None);
    }

    #[test]
    fn page_titles_lose_their_suffix() {
        let p = serde_json::json!({"product_name": "作品 [社团] | DLsite"});
        assert_eq!(parse_product(&p).unwrap().title, "作品");
        let p = serde_json::json!({"product_name": "名字 [带括号]"});
        assert_eq!(parse_product(&p).unwrap().title, "名字 [带括号]");
        assert!(parse_product(&serde_json::json!({"product_name": " "})).is_none());
    }

    #[test]
    fn release_dates_that_do_not_exist_are_left_out() {
        let date = |regist: &str| {
            let p = serde_json::json!({"product_name": "作品", "regist_date": regist});
            parse_product(&p).unwrap().release_date
        };
        assert_eq!(date("2024-02-29 16:00:00").as_deref(), Some("2024-02-29"));
        assert_eq!(date("2024-02-30 16:00:00"), None);
        assert_eq!(date("2024/02/01 16:00:00"), None);
    }

    #[test]
    fn sections_follow_the_prefix() {
        assert_eq!(section("RJ01234567"), "maniax");
        assert_eq!(section("BJ123456"), "books");
        assert_eq!(section("VJ01246802"), "pro");
    }
}
