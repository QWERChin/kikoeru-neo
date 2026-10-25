//! `GET /api/works/{id}`: the summary plus voice actors, tags and the
//! flat track list described in book/src/contract.md.

use super::{reviews::ReviewError, works::ListError};

/// Larger works are refused with 413 rather than sent as one huge list.
pub const MAX_TRACKS: i64 = 5000;

#[derive(Debug, thiserror::Error)]
pub enum DetailError {
    #[error(transparent)]
    List(#[from] ListError),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error(transparent)]
    Review(#[from] ReviewError),
    #[error("work has {0} tracks, more than {MAX_TRACKS}")]
    TooManyTracks(i64),
    #[error("track {id} has an invalid value: {reason}")]
    InvalidTrack { id: i64, reason: &'static str },
}
