/// A row of `users`. It carries the password hash, so it never leaves the
/// server as a response; handlers convert it to `dto::User`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserRecord {
    pub id: i64,
    pub name: String,
    pub password_hash: Option<String>,
    pub is_admin: bool,
    pub token_version: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum CreateError {
    #[error("user {0} already exists")]
    NameTaken(String),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteOutcome {
    Deleted,
    NotFound,
    LastAdmin,
}
