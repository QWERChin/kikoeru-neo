//! The proxy that metadata and cover requests go through (book/src/contract.md,
//! "服务器设置"). An admin can change it while the server runs, so each
//! source keeps its client in a [`SharedClient`] and rebuilds it on change.

use std::{
    error::Error as _,
    sync::{PoisonError, RwLock},
    time::Duration,
};

use reqwest::{Client, ClientBuilder};
use url::Url;

use super::MetadataError;

/// How long a proxy check waits for DLsite's answer.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(10);

/// A proxy address that passed the contract's rules, in its normalised form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyUrl(String);

impl ProxyUrl {
    /// `None` for an empty value, which clears the setting. The error is a
    /// Chinese sentence for the admin who typed the value.
    pub fn parse(text: &str) -> Result<Option<Self>, String> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(None);
        }
        let example = "比如 http://127.0.0.1:7890";
        let url = Url::parse(text).map_err(|_| format!("不是有效的代理地址，{example}"))?;
        if !matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h") {
            return Err(format!(
                "代理的协议只能是 http、https、socks5 或 socks5h，{example}"
            ));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err("代理地址不能带用户名和密码".into());
        }
        let host = match url.host_str() {
            Some(host) if !host.is_empty() => host.to_ascii_lowercase(),
            _ => return Err(format!("代理地址缺少主机名，{example}")),
        };
        if !matches!(url.path(), "" | "/") || url.query().is_some() || url.fragment().is_some() {
            return Err("代理地址只能有协议、主机和端口，不能带路径或参数".into());
        }
        // `Url` drops a port equal to the scheme's default for http and
        // https only. socks5 has no default to `Url`, so its port stays.
        let normalised = match url.port() {
            Some(port) => format!("{}://{host}:{port}", url.scheme()),
            None => format!("{}://{host}", url.scheme()),
        };
        reqwest::Proxy::all(&normalised).map_err(|e| format!("不是有效的代理地址：{e}"))?;
        Ok(Some(Self(normalised)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Where metadata requests go.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ProxyChoice {
    /// What reqwest reads from `HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY` and
    /// `NO_PROXY`, which is direct when none is set.
    #[default]
    Environment,
    /// This proxy for every request, whatever the environment says.
    Fixed(ProxyUrl),
}

impl ProxyChoice {
    /// The choice a stored or submitted setting stands for.
    pub fn from_setting(setting: Option<&str>) -> Result<Self, String> {
        Ok(match setting.map(ProxyUrl::parse).transpose()?.flatten() {
            Some(url) => Self::Fixed(url),
            None => Self::Environment,
        })
    }
}

/// Builds a source's client with `proxy` applied.
pub(super) fn build(builder: ClientBuilder, proxy: &ProxyChoice) -> Result<Client, MetadataError> {
    let builder = match proxy {
        ProxyChoice::Environment => builder,
        // Adding a proxy turns off the environment's, NO_PROXY included.
        ProxyChoice::Fixed(url) => builder.proxy(
            reqwest::Proxy::all(url.as_str())
                .map_err(|e| MetadataError::Unavailable(e.to_string()))?,
        ),
    };
    builder
        .build()
        .map_err(|e| MetadataError::Unavailable(e.to_string()))
}

/// A source's client, replaced as a whole when the proxy changes. Each
/// request takes the client that is current when it starts, so requests
/// already sent finish the way they began.
pub(super) struct SharedClient(RwLock<Client>);

impl SharedClient {
    pub(super) fn new(client: Client) -> Self {
        Self(RwLock::new(client))
    }

    pub(super) fn get(&self) -> Client {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn replace(&self, client: Client) {
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = client;
    }
}

/// A source answered a proxy check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reachable {
    pub url: Url,
    pub status: u16,
    pub elapsed: Duration,
}

/// reqwest's own message only says the request failed. The reason, such as
/// a refused or reset connection, is further down the chain.
pub(super) fn describe(error: &reqwest::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> Result<Option<String>, String> {
        ProxyUrl::parse(text).map(|url| url.map(|u| u.as_str().to_owned()))
    }

    #[test]
    fn normalises_scheme_host_port_and_slash() {
        assert_eq!(
            parsed(" HTTP://LocalHost:7890/ ").unwrap().as_deref(),
            Some("http://localhost:7890")
        );
        assert_eq!(
            parsed("socks5h://Proxy.LAN:1080").unwrap().as_deref(),
            Some("socks5h://proxy.lan:1080")
        );
        assert_eq!(
            parsed("http://127.0.0.1:80").unwrap().as_deref(),
            Some("http://127.0.0.1")
        );
        assert_eq!(
            parsed("socks5://[::1]").unwrap().as_deref(),
            Some("socks5://[::1]")
        );
        assert_eq!(
            parsed("https://[::1]:8443").unwrap().as_deref(),
            Some("https://[::1]:8443")
        );
    }

    #[test]
    fn empty_clears_the_setting() {
        assert_eq!(parsed("").unwrap(), None);
        assert_eq!(parsed("   ").unwrap(), None);
        assert_eq!(
            ProxyChoice::from_setting(None),
            Ok(ProxyChoice::Environment)
        );
        assert_eq!(
            ProxyChoice::from_setting(Some(" ")),
            Ok(ProxyChoice::Environment)
        );
    }

    #[test]
    fn refuses_what_the_contract_rules_out() {
        for text in [
            "127.0.0.1:7890",
            "localhost:7890",
            "ftp://127.0.0.1:21",
            "socks4://127.0.0.1:1080",
            "http://user:secret@127.0.0.1:7890",
            "http://user@127.0.0.1:7890",
            "http://127.0.0.1:7890/path",
            "http://127.0.0.1:7890/?a=1",
            "http://127.0.0.1:7890/#top",
            "socks5://",
            "http://",
        ] {
            assert!(parsed(text).is_err(), "{text} was accepted");
        }
    }
}
