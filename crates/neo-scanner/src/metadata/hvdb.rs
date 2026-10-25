//! hvdb's work pages as a source of voice actor names, for RJ works the
//! source chain answered without any (book/src/scanning.md, "声优补充").
//! It follows kikoeru 0.6.17's `src/scraper/hvdb.ts`.

use std::time::Duration;

use reqwest::{Client, ClientBuilder, StatusCode};
use tokio::{sync::Mutex, time::Instant};
use url::Url;

use super::{
    MetadataError,
    asmr_one::{retry_after, work_number},
    proxy::{self, ProxyChoice, SharedClient},
};

#[derive(Debug, Clone)]
pub struct HvdbConfig {
    /// Ends with `/`.
    pub base: Url,
    /// The least time between two requests. hvdb is a community site.
    pub spacing: Duration,
    pub retries: u32,
    pub backoff: Duration,
    /// 0.6.17's `hvdbTimeout`.
    pub timeout: Duration,
    /// The longest one call waits out 429 and 503 answers.
    pub wait_budget: Duration,
    /// The proxy at start. [`HvdbProvider::set_proxy`] replaces it.
    pub proxy: ProxyChoice,
}

impl Default for HvdbConfig {
    fn default() -> Self {
        Self {
            base: Url::parse("https://hvdb.me/").expect("static URL"),
            spacing: Duration::from_secs(2),
            retries: 3,
            backoff: Duration::from_secs(1),
            timeout: Duration::from_secs(10),
            wait_budget: Duration::from_secs(120),
            proxy: ProxyChoice::Environment,
        }
    }
}

pub struct HvdbProvider {
    client: SharedClient,
    config: HvdbConfig,
    last: Mutex<Option<Instant>>,
}

impl HvdbProvider {
    pub fn new(config: HvdbConfig) -> Result<Self, MetadataError> {
        let client = proxy::build(builder(&config), &config.proxy)?;
        Ok(Self {
            client: SharedClient::new(client),
            config,
            last: Mutex::new(None),
        })
    }

    /// Sends later requests through `proxy`, as
    /// [`MetadataProvider::set_proxy`](super::MetadataProvider::set_proxy)
    /// does for the sources.
    pub fn set_proxy(&self, proxy: &ProxyChoice) -> Result<(), MetadataError> {
        self.client
            .replace(proxy::build(builder(&self.config), proxy)?);
        Ok(())
    }

    /// The voice actors hvdb lists for `code`, in page order. Only RJ works
    /// are asked; any other code has none.
    pub async fn voice_actors(&self, code: &str) -> Result<Vec<String>, MetadataError> {
        let Some(number) = work_number(code) else {
            return Ok(Vec::new());
        };
        let url = self
            .config
            .base
            .join(&format!("Dashboard/Details/{number}"))
            .map_err(|e| MetadataError::Unavailable(e.to_string()))?;
        let body = self.get(url).await?;
        parse_page(&String::from_utf8_lossy(&body), number).ok_or(MetadataError::NotFound)
    }

    async fn pace(&self) {
        let mut last = self.last.lock().await;
        if let Some(at) = *last {
            tokio::time::sleep_until(at + self.config.spacing).await;
        }
        *last = Some(Instant::now());
    }

    /// GET with spacing, retries and Retry-After. hvdb answers 500 for a
    /// code it does not know, so 500 is a definite answer, not an outage.
    async fn get(&self, url: Url) -> Result<Vec<u8>, MetadataError> {
        let mut delay = self.config.backoff;
        let mut budget = self.config.wait_budget;
        let mut attempt = 0;
        loop {
            self.pace().await;
            let mut wait = None;
            let error = match self.client.get().get(url.clone()).send().await {
                Ok(response) if response.status().is_success() => match response.bytes().await {
                    Ok(body) => return Ok(body.to_vec()),
                    Err(e) => format!("{url}: {e}"),
                },
                // No page lives behind a redirect.
                Ok(response)
                    if matches!(
                        response.status(),
                        StatusCode::NOT_FOUND | StatusCode::INTERNAL_SERVER_ERROR
                    ) || response.status().is_redirection() =>
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
                Ok(response)
                    if matches!(
                        response.status(),
                        StatusCode::BAD_GATEWAY | StatusCode::GATEWAY_TIMEOUT
                    ) =>
                {
                    format!("{url}: HTTP {}", response.status())
                }
                Ok(response) => {
                    return Err(MetadataError::Unavailable(format!(
                        "{url}: HTTP {}",
                        response.status()
                    )));
                }
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
            tracing::debug!(%error, attempt, "retrying an hvdb request");
            tokio::time::sleep(pause).await;
            delay *= 2;
        }
    }
}

const TITLE_PREFIX: &str = "HVDB - ";
const VOICE_ACTOR_LINK: &str = "/Dashboard/CVWorks/";

/// The page is asked for at its final address, so no request skips the
/// spacing by being a redirect hop, and none can leave hvdb.
fn builder(config: &HvdbConfig) -> ClientBuilder {
    Client::builder()
        .timeout(config.timeout)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("kikoeru-neo/", env!("CARGO_PKG_VERSION")))
}

/// The voice actor names on a work page, or `None` when the page is not
/// about work `number`. Names with Latin letters are dropped, as 0.6.17
/// does: hvdb lists most people twice, in romaji and in Japanese.
fn parse_page(html: &str, number: u64) -> Option<Vec<String>> {
    let title = decode(between(html, "<title>", "</title>")?);
    let code = title.trim().strip_prefix(TITLE_PREFIX)?;
    if work_number(code.trim()) != Some(number) {
        return None;
    }
    let mut names: Vec<String> = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find("<a ") {
        rest = &rest[start..];
        let Some(tag_end) = rest.find('>') else { break };
        let (tag, after) = rest.split_at(tag_end + 1);
        rest = after;
        if !tag.contains(VOICE_ACTOR_LINK) {
            continue;
        }
        let Some(text_end) = rest.find("</a>") else {
            break;
        };
        let name = decode(&rest[..text_end]);
        let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        rest = &rest[text_end..];
        if !name.is_empty()
            && !name.chars().any(|c| c.is_ascii_alphabetic())
            && !names.contains(&name)
        {
            names.push(name);
        }
    }
    Some(names)
}

fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = text.find(open)? + open.len();
    let len = text[start..].find(close)?;
    Some(&text[start..start + len])
}

/// Decodes the character references hvdb's pages use: numeric ones for
/// everything outside ASCII, and the few named ones HTML escaping writes.
/// Anything else is left as it is.
fn decode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let decoded = rest.find(';').and_then(|semi| {
            let char = match &rest[1..semi] {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                "nbsp" => ' ',
                entity => {
                    let digits = entity.strip_prefix('#')?;
                    let code = match digits.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => digits.parse().ok()?,
                    };
                    char::from_u32(code)?
                }
            };
            Some((char, semi + 1))
        });
        match decoded {
            Some((char, len)) => {
                out.push(char);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of a real work page from 2026-10-12, trimmed, with
    /// made-up names.
    fn page(code: &str) -> String {
        format!(
            r#"<html><head><title>{TITLE_PREFIX}{code}</title></head><body>
            <input id="Name" name="Name" type="text" value="&#x4F5C;&#x54C1;" />
            <a class="detailCircle" href="/Dashboard/CircleWorks/51620">&#xA;  &#x30B5;&#x30FC;&#x30AF;&#x30EB;&#xA;</a>
            <a href="/Dashboard/TagWorks/5">ear cleaning</a>
            <li class="tagit-choice"><span class="tagit-label displayTag">
                <a href="/Dashboard/CVWorks/1">Koe Hitoe</a>
            </span></li>
            <li class="tagit-choice"><span class="tagit-label displayTag">
                <a href="/Dashboard/CVWorks/2">&#x58F0;&#x4E00;&#x91CD;</a>
            </span></li>
            <a href="/Dashboard/CVWorks/3" class="x">&#x58F0;&#x4E00;&#x91CD;</a>
            <a href="/Dashboard/CVWorks/4">
                &#x4E8C; &#x91CD;&amp;&#12354;
            </a>
            </body></html>"#
        )
    }

    #[test]
    fn a_work_page_gives_its_japanese_voice_actor_names() {
        assert_eq!(
            parse_page(&page("RJ01234567"), 1_234_567).unwrap(),
            ["声一重", "二 重&あ"],
            "no romaji, no repeats, no circle or tags"
        );
    }

    #[test]
    fn a_page_about_another_work_gives_nothing() {
        assert_eq!(parse_page(&page("RJ01234568"), 1_234_567), None);
        assert_eq!(parse_page("<html>no title</html>", 1_234_567), None);
        // hvdb may drop the leading zero in the code it shows.
        assert!(parse_page(&page("RJ1234567"), 1_234_567).is_some());
    }

    #[test]
    fn character_references_are_decoded() {
        assert_eq!(decode("&#x58F0;&#22768;&lt;&amp;&gt;"), "声声<&>");
        assert_eq!(decode("a & b &unknown; &#xZZ;"), "a & b &unknown; &#xZZ;");
    }
}
