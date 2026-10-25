#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    Asc,
    Desc,
}

impl Order {
    #[must_use]
    pub fn sql(self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Added,
    Release,
    Rating,
    /// A fixed order derived from the seed and the work ID, so paging with
    /// one seed never repeats or skips a work.
    Random {
        seed: u32,
    },
}

/// Which works a list draws from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Library,
    /// The user's favorites, newest first.
    Favorites,
    /// Works the user has progress on, most recently played first.
    History,
    /// Works the user rated or reviewed, most recently changed first.
    Reviews,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    /// A literal substring of the code, title, circle name or a voice
    /// actor's name.
    pub q: Option<String>,
    pub tag: Option<i64>,
    pub circle: Option<i64>,
    pub va: Option<String>,
    pub root: Option<String>,
    pub include_missing: bool,
}

/// One page of works. Every order ends with the work ID, so pages never
/// overlap or skip a work.
#[derive(Debug, Clone)]
pub struct ListQuery {
    pub user_id: i64,
    pub scope: Scope,
    pub sort: Sort,
    pub order: Order,
    pub filter: Filter,
    pub limit: u32,
    pub offset: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ListError {
    #[error(transparent)]
    Db(#[from] sqlx::Error),
    #[error("work {id} has an invalid value: {reason}")]
    InvalidRow { id: i64, reason: &'static str },
}
