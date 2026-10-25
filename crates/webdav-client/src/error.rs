//! Failures and how a storage layer classifies them.

/// Everything a request can fail with. `Display` never includes response
/// bodies, which can echo credentials.
#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The server answered 404 or 410.
    #[error("{path} does not exist on the WebDAV server")]
    NotFound { path: String },
    /// The HTTP status was not one the request expects.
    #[error("WebDAV server returned HTTP {status} for {path}")]
    Http { status: u16, path: String },
    /// The request never produced a complete response.
    #[error("request for {path} failed: {message}")]
    Transport { path: String, message: String },
    /// The server asked the client to pause for longer than the wait
    /// budget allows.
    #[error("the WebDAV server is throttling requests beyond the wait budget")]
    RateLimited,
    /// A redirect to another origin, which would carry the credentials
    /// there, or too many redirects.
    #[error("refused redirect for {path}: {message}")]
    Redirect { path: String, message: String },
    /// The response did not have the expected shape.
    #[error("unexpected response for {path}: {message}")]
    Protocol { path: String, message: String },
    /// A path or entry name that cannot be used safely.
    #[error("invalid WebDAV path {path:?}")]
    InvalidPath { path: String },
}

/// The three outcomes a storage layer must tell apart. Only
/// [`ErrorKind::NotFound`] is evidence that something is gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    NotFound,
    /// The server could not answer right now, or refused the account.
    Unavailable,
    /// The server answered something that retrying will not change.
    Upstream,
}

impl Error {
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::NotFound { .. } => ErrorKind::NotFound,
            Self::Transport { .. } | Self::RateLimited => ErrorKind::Unavailable,
            // A refused login, a locked resource, throttling and server
            // errors say nothing about whether the file exists.
            Self::Http { status, .. } if is_unavailable(*status) => ErrorKind::Unavailable,
            Self::Http { .. }
            | Self::Redirect { .. }
            | Self::Protocol { .. }
            | Self::InvalidPath { .. } => ErrorKind::Upstream,
        }
    }

    pub(crate) fn protocol(path: &str, message: impl Into<String>) -> Self {
        Self::Protocol {
            path: path.to_owned(),
            message: message.into(),
        }
    }
}

pub(crate) fn is_unavailable(status: u16) -> bool {
    matches!(status, 401 | 403 | 423 | 429) || status >= 500
}

/// Statuses worth another attempt: the server may answer next time.
pub(crate) fn is_retryable(status: u16) -> bool {
    matches!(status, 429 | 502 | 503 | 504) || status == 500
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_are_classified_like_the_storage_chapter_says() {
        let http = |status| Error::Http {
            status,
            path: "/x".into(),
        };
        for status in [401, 403, 423, 429, 500, 502, 503] {
            assert_eq!(http(status).kind(), ErrorKind::Unavailable, "{status}");
        }
        for status in [400, 405, 409, 412] {
            assert_eq!(http(status).kind(), ErrorKind::Upstream, "{status}");
        }
        assert_eq!(
            Error::NotFound { path: "/x".into() }.kind(),
            ErrorKind::NotFound
        );
    }
}
