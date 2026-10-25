//! The user's own ratings and short reviews (`reviews`, book/src/data-model.md).

#[derive(Debug, thiserror::Error)]
pub enum ReviewError {
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error("review of work {work_id} has an invalid value: {reason}")]
    InvalidRow { work_id: i64, reason: &'static str },
}
