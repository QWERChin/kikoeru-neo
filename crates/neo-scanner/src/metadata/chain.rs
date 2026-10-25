//! Sources asked in order (book/src/scanning.md, "备用来源"): DLsite first,
//! then the fallback when DLsite fails in any way, as kikoeru 0.6.17 does.
//! hvdb then fills in voice actors when the answer has none ("声优补充").

use std::sync::Arc;

use url::Url;

use super::{HvdbProvider, MetadataError, MetadataProvider, ProxyChoice, Reachable, WorkMetadata};

pub struct ProviderChain {
    primary: Arc<dyn MetadataProvider>,
    fallback: Option<Arc<dyn MetadataProvider>>,
    voice_actors: Option<Arc<HvdbProvider>>,
}

impl ProviderChain {
    pub fn new(
        primary: Arc<dyn MetadataProvider>,
        fallback: Option<Arc<dyn MetadataProvider>>,
    ) -> Self {
        Self {
            primary,
            fallback,
            voice_actors: None,
        }
    }

    /// Asks hvdb for the voice actors of works answered without any.
    pub fn with_voice_actors(self, hvdb: Arc<HvdbProvider>) -> Self {
        Self {
            voice_actors: Some(hvdb),
            ..self
        }
    }

    async fn answer(&self, code: &str) -> Result<WorkMetadata, MetadataError> {
        let first = match self.primary.fetch(code).await {
            Ok(meta) => return Ok(meta),
            Err(e) => e,
        };
        let Some(fallback) = &self.fallback else {
            return Err(first);
        };
        tracing::info!(code, error = %first, "trying the fallback metadata source");
        match (first, fallback.fetch(code).await) {
            (_, Ok(meta)) => Ok(meta),
            (MetadataError::NotFound, Err(MetadataError::NotFound)) => Err(MetadataError::NotFound),
            // Either source may answer next time, so the work waits.
            (first, Err(second)) => Err(MetadataError::Unavailable(format!(
                "{first}; fallback: {second}"
            ))),
        }
    }

    /// hvdb only adds to an answer, so its failures leave the answer as
    /// it is; a later refresh asks again.
    async fn fill_voice_actors(&self, code: &str, meta: WorkMetadata) -> WorkMetadata {
        let Some(hvdb) = self.voice_actors.as_ref().filter(|_| meta.vas.is_empty()) else {
            return meta;
        };
        match hvdb.voice_actors(code).await {
            Ok(vas) => WorkMetadata { vas, ..meta },
            Err(MetadataError::NotFound) => {
                tracing::info!(code, "hvdb has no page for this work");
                meta
            }
            Err(error) => {
                tracing::warn!(code, %error, "no voice actors from hvdb");
                meta
            }
        }
    }
}

#[async_trait::async_trait]
impl MetadataProvider for ProviderChain {
    async fn fetch(&self, code: &str) -> Result<WorkMetadata, MetadataError> {
        let meta = self.answer(code).await?;
        Ok(self.fill_voice_actors(code, meta).await)
    }

    async fn cover(&self, url: &Url) -> Result<Vec<u8>, MetadataError> {
        match &self.fallback {
            Some(fallback) if !self.primary.serves(url) && fallback.serves(url) => {
                fallback.cover(url).await
            }
            _ => self.primary.cover(url).await,
        }
    }

    fn serves(&self, url: &Url) -> bool {
        self.primary.serves(url) || self.fallback.as_ref().is_some_and(|f| f.serves(url))
    }

    /// Every source gets the proxy even when one fails, so a failure leaves
    /// as few of them on the old proxy as possible.
    async fn set_proxy(&self, proxy: &ProxyChoice) -> Result<(), MetadataError> {
        let mut results = vec![self.primary.set_proxy(proxy).await];
        if let Some(fallback) = &self.fallback {
            results.push(fallback.set_proxy(proxy).await);
        }
        if let Some(hvdb) = &self.voice_actors {
            results.push(hvdb.set_proxy(proxy));
        }
        results.into_iter().collect()
    }

    /// DLsite is the source every work is asked of first.
    async fn check(&self, proxy: &ProxyChoice) -> Result<Reachable, MetadataError> {
        self.primary.check(proxy).await
    }

    async fn fallback_cover(&self, code: &str) -> Result<Option<Vec<u8>>, MetadataError> {
        let Some(fallback) = &self.fallback else {
            return Ok(None);
        };
        let meta = match fallback.fetch(code).await {
            Ok(meta) => meta,
            Err(MetadataError::NotFound) => return Ok(None),
            Err(e) => return Err(e),
        };
        let Some(url) = meta.cover_url else {
            return Ok(None);
        };
        match fallback.cover(&url).await {
            Ok(bytes) => Ok(Some(bytes)),
            Err(MetadataError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
