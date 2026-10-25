//! Work metadata from outside sources (book/src/scanning.md, "元数据来源").
//! Every source implements [`MetadataProvider`] and returns the same
//! [`WorkMetadata`]; [`store`] writes it under the `metadata_src` and
//! `cover_src` rules of book/src/data-model.md.

mod asmr_one;
mod chain;
mod dlsite;
mod folder;
mod hvdb;
mod proxy;
mod store;

pub use asmr_one::{AsmrOneConfig, AsmrOneProvider, DEFAULT_MIRRORS};
pub use chain::ProviderChain;
pub use dlsite::{DlsiteConfig, DlsiteProvider};
pub use folder::{FolderCoverSummary, MAX_FOLDER_IMAGE_BYTES, fill_from_folders};
pub use hvdb::{HvdbConfig, HvdbProvider};
pub use proxy::{CHECK_TIMEOUT, ProxyChoice, ProxyUrl, Reachable};
pub use store::{
    CoverDirs, FetchOutcome, FolderCoverError, MetadataSummary, fetch_and_store, fetch_missing,
    fetch_missing_except, lock_covers, recover_cover_swaps, store_folder_cover, wait_for_covers,
};

use serde_json::Value;

/// A circle as DLsite identifies it: the maker code (`RG12345`) packed into
/// the number the old database used, so imported circles keep their IDs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Circle {
    pub id: i64,
    pub name: String,
}

/// A raw tag: its name in the requested language, and DLsite's genre ID
/// when the source has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawTag {
    pub name: String,
    pub dlsite_id: Option<i64>,
}

/// Where metadata came from, stored as `metadata_src` and, for a cover,
/// `cover_src` (book/src/data-model.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataSource {
    Dlsite,
    AsmrOne,
}

impl MetadataSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dlsite => "dlsite",
            Self::AsmrOne => "asmr_one",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkMetadata {
    pub source: MetadataSource,
    pub title: String,
    pub circle: Option<Circle>,
    /// `YYYY-MM-DD`.
    pub release_date: Option<String>,
    pub nsfw: bool,
    pub vas: Vec<String>,
    pub tags: Vec<RawTag>,
    pub dl_count: Option<i64>,
    pub price: Option<i64>,
    pub rate_average: Option<f64>,
    pub rate_count: Option<i64>,
    /// Stored in `works.extra` under these keys, next to whatever else is
    /// there (the import's `legacy_durations`).
    pub rank: Option<Value>,
    pub rate_count_detail: Option<Value>,
    pub cover_url: Option<url::Url>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MetadataError {
    /// The source answered that it has no such work.
    #[error("not found at the metadata source")]
    NotFound,
    /// The source could not answer: no network, a timeout, a 5xx, or an
    /// answer that could not be read. Retrying later may work.
    #[error("metadata source unavailable: {0}")]
    Unavailable(String),
}

#[async_trait::async_trait]
pub trait MetadataProvider: Send + Sync {
    async fn fetch(&self, code: &str) -> Result<WorkMetadata, MetadataError>;

    /// Downloads a cover image found by [`fetch`](Self::fetch).
    async fn cover(&self, url: &url::Url) -> Result<Vec<u8>, MetadataError>;

    /// Whether [`cover`](Self::cover) downloads from this URL's host. A
    /// chain of sources uses it to send each cover to its own source.
    fn serves(&self, _url: &url::Url) -> bool {
        true
    }

    /// A cover from another source, for when the one [`fetch`](Self::fetch)
    /// pointed at could not be downloaded. `None` when there is no other
    /// source or it has no cover for the work.
    async fn fallback_cover(&self, _code: &str) -> Result<Option<Vec<u8>>, MetadataError> {
        Ok(None)
    }

    /// Sends this source's later requests through `proxy`. Requests
    /// already sent finish the way they began.
    async fn set_proxy(&self, _proxy: &ProxyChoice) -> Result<(), MetadataError> {
        Ok(())
    }

    /// One request to the source through `proxy`, which is not kept, so an
    /// admin can try a proxy before saving it. Any HTTP answer counts.
    async fn check(&self, _proxy: &ProxyChoice) -> Result<Reachable, MetadataError> {
        Err(MetadataError::Unavailable(
            "this source cannot be checked".into(),
        ))
    }
}
