//! Favorites and the circle, voice actor and tag lists of M3. The work
//! lists themselves are in `works`.

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error("row {id} has an invalid value: {reason}")]
    InvalidRow { id: String, reason: &'static str },
}
