//! The local backend's roots API ("媒体库管理接口" in book/src/contract.md),
//! for the folders an import adds. The shell goes through HTTP like the
//! page would, so these roots get the same checks and take effect without
//! a restart.

use anyhow::{Result, bail};
use neo_core::dto::{RootCreateRequest, RootInputSettings, RootSettings, RootStatus};

pub struct Api {
    client: reqwest::Client,
    base: String,
    token: String,
}

impl Api {
    pub fn new(port: u16, token: String) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder().no_proxy().build()?,
            base: format!("http://127.0.0.1:{port}/api/admin/roots"),
            token,
        })
    }

    pub async fn list(&self) -> Result<Vec<RootStatus>> {
        let response = self
            .client
            .get(&self.base)
            .bearer_auth(&self.token)
            .send()
            .await?;
        Ok(checked(response).await?.json().await?)
    }

    /// Adds a local root under a new random ID. An ID is never reused: a
    /// deleted root's tracks stay as missing and come back when a root with
    /// the same ID finds them again.
    pub async fn add_local(&self, display_name: &str, path: &str) -> Result<RootStatus> {
        let request = RootCreateRequest {
            id: new_id()?,
            display_name: display_name.to_owned(),
            // Null watch and interval take the desktop defaults: watched,
            // no timed scans.
            settings: RootInputSettings::Local {
                path: path.to_owned(),
                watch: None,
                auto_scan_interval: None,
            },
            skip_check: None,
        };
        let response = self
            .client
            .post(&self.base)
            .bearer_auth(&self.token)
            .json(&request)
            .send()
            .await?;
        Ok(checked(response).await?.json().await?)
    }
}

/// The paths of the local roots, from both config.toml and the database.
pub fn local_paths(roots: &[RootStatus]) -> Vec<String> {
    roots
        .iter()
        .filter_map(|r| match &r.settings {
            RootSettings::Local { path, .. } => Some(path.clone()),
            RootSettings::OpenList { .. } | RootSettings::WebDav { .. } => None,
        })
        .collect()
}

/// Turns an error response into its `message`.
async fn checked(response: reqwest::Response) -> Result<reqwest::Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body: serde_json::Value = response.json().await.unwrap_or_default();
    match body["error"]["message"].as_str() {
        Some(message) => bail!("{message}"),
        None => bail!("the roots API answered {status}"),
    }
}

fn new_id() -> Result<String> {
    let mut bytes = [0u8; 6];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("no system randomness: {e}"))?;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let id = format!("local-{hex}");
    // The contract's rule: lowercase letters, digits, `-` and `_`, at most
    // 32 characters, starting with a letter or digit.
    debug_assert!(id.len() <= 32);
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_ids_follow_the_contract_and_differ() {
        let a = new_id().unwrap();
        assert!(a.starts_with("local-") && a.len() == 18, "{a}");
        assert!(
            a.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        );
        assert_ne!(a, new_id().unwrap());
    }
}
