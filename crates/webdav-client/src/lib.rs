//! A read-only WebDAV client for media libraries on a NAS or any server
//! that speaks WebDAV (book/src/storage.md, "webdav").
//!
//! - Listings use `PROPFIND` with `Depth: 1` and only the four properties a
//!   library needs. Every `response` of the 207 answer is checked on its
//!   own: its decoded `href` must be a direct child of the listed
//!   collection, so a server cannot point the caller outside it.
//! - Files are read with `GET`, forwarding `Range` and `If-Range`.
//! - Every request carries HTTP Basic credentials, so redirects are only
//!   followed within the server's origin.
//! - 429 and 503 pause the whole client within a wait budget, and
//!   connection errors and server errors are retried a few times.
//! - [`Error::kind`] tells a missing path apart from a server that cannot
//!   answer, which a scanner must never take as deleted files.
//!
//! Like `openlist-client`, the crate does not depend on kikoeru-neo.

mod client;
mod config;
mod error;
mod xml;

pub use client::{Client, Entry, Transfer};
pub use config::{Config, ConfigError};
pub use error::{Error, ErrorKind};
