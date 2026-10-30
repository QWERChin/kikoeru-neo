//! The `tasks` table. State changes follow "后台任务" in
//! book/src/data-model.md: `queued → running → done | failed | cancelled`,
//! `queued → cancelled`, and `running → interrupted` at startup.

#[derive(Debug, Clone, PartialEq)]
pub struct TaskRow {
    pub id: i64,
    pub kind: String,
    pub dedupe_key: Option<String>,
    pub state: String,
    pub payload: String,
    pub result: Option<String>,
    pub progress: Option<f64>,
    pub message: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// What startup recovery found.
#[derive(Debug, Clone, PartialEq)]
pub struct Recovered {
    /// Tasks the previous process left `running`, now `interrupted`.
    pub interrupted: Vec<TaskRow>,
    /// Tasks still `queued`, oldest first.
    pub queued: Vec<TaskRow>,
}

#[derive(Debug)]
pub enum Enqueued {
    Created(i64),
    /// PostgreSQL returned an insert ID, but the commit acknowledgement was
    /// lost. Keep the original job until its transaction outcome is known.
    CommitUnknown {
        id: i64,
        /// The full PostgreSQL xid8, kept as text without narrowing to xid32.
        xid: String,
        error: sqlx::Error,
    },
    /// An active task with the same `dedupe_key` already exists.
    Existing(i64),
    /// `queue_limit` tasks of the same kind are already waiting.
    QueueFull,
}

/// The outcome of a PostgreSQL enqueue transaction whose acknowledgement was lost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitStatus {
    InProgress,
    Committed,
    Aborted,
    /// PostgreSQL has discarded the transaction's status.
    Unavailable,
}

/// The end states a running or queued task can move to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndState {
    Done,
    Failed,
    Cancelled,
    Interrupted,
}

impl EndState {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }
}
