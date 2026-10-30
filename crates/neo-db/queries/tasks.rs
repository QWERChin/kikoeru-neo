//! The `tasks` table. State changes follow "后台任务" in
//! book/src/data-model.md: `queued → running → done | failed | cancelled`,
//! `queued → cancelled`, and `running → interrupted` at startup.

use neo_core::db::tasks::*;

#[cfg(feature = "testing")]
tokio::task_local! {
    /// Commits when true or rolls back when false, then loses the acknowledgement.
    pub static ENQUEUE_COMMIT_OUTCOME: bool;
    /// Hands the open transaction to the test before losing the acknowledgement.
    pub static ENQUEUE_PENDING: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<(i64, crate::Tx<'static>)>>>;
    /// Reports when the retained runner has checked the database.
    pub static ENQUEUE_CHECKED: std::sync::Arc<tokio::sync::Notify>;
}

/// Inserts a queued task unless an active one has the same `dedupe_key`.
/// The check and the insert share one transaction on the single write
/// connection, so two callers cannot both create a task for one key.
/// A lost PostgreSQL commit acknowledgement returns [`Enqueued::CommitUnknown`]
/// so the scheduler can retain the job while preserving the submit error.
///
/// # Errors
/// Any database error.
pub async fn enqueue(
    write: &crate::Pool,
    kind: &str,
    dedupe_key: Option<&str>,
    payload: &str,
    queue_limit: Option<i64>,
    now: i64,
) -> Result<Enqueued, sqlx::Error> {
    neo_core::db::retry_busy(|| enqueue_once(write, kind, dedupe_key, payload, queue_limit, now))
        .await
}

async fn enqueue_once(
    write: &crate::Pool,
    kind: &str,
    dedupe_key: Option<&str>,
    payload: &str,
    queue_limit: Option<i64>,
    now: i64,
) -> Result<Enqueued, sqlx::Error> {
    let mut tx = write.begin().await?;
    if let Some(key) = dedupe_key {
        let existing = sqlx::query_scalar!(
            r#"SELECT id AS "id!: i64" FROM tasks
               WHERE dedupe_key = $1 AND state IN ('queued', 'running')"#,
            key
        )
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(id) = existing {
            return Ok(Enqueued::Existing(id));
        }
    }
    // The limit is per kind: a scan that queues hundreds of background
    // probes must not make playback transcodes look like a full queue.
    if let Some(limit) = queue_limit {
        let queued = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!: i64" FROM tasks WHERE state = 'queued' AND kind = $1"#,
            kind
        )
        .fetch_one(&mut *tx)
        .await?;
        if queued >= limit {
            return Ok(Enqueued::QueueFull);
        }
    }
    let id = sqlx::query_scalar!(
        r#"INSERT INTO tasks (kind, dedupe_key, state, payload, created_at, updated_at)
           VALUES ($1, $2, 'queued', $3, $4, $5)
           RETURNING id AS "id!: i64""#,
        kind,
        dedupe_key,
        payload,
        now,
        now
    )
    .fetch_one(&mut *tx)
    .await?;
    // PostgreSQL-specific SQL: a row ID alone cannot identify the outcome
    // after a crash rolls back an uncommitted sequence advancement.
    let xid: Option<String> = if <crate::Database as sqlx::Database>::NAME == "PostgreSQL" {
        Some(
            sqlx::query_scalar("SELECT pg_current_xact_id()::text")
                .fetch_one(&mut *tx)
                .await?,
        )
    } else {
        None
    };
    #[cfg(not(feature = "testing"))]
    let committed = tx.commit().await;
    #[cfg(feature = "testing")]
    let committed = if let Some(pending) = ENQUEUE_PENDING
        .try_with(|pending| pending.lock().unwrap().take())
        .ok()
        .flatten()
    {
        pending.send((id, tx)).unwrap();
        Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "enqueue commit acknowledgement lost",
        )
        .into())
    } else {
        match ENQUEUE_COMMIT_OUTCOME.try_with(|commit| *commit) {
            Ok(commit) => {
                if commit {
                    tx.commit().await?;
                } else {
                    tx.rollback().await?;
                }
                Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    "enqueue commit acknowledgement lost",
                )
                .into())
            }
            Err(_) => tx.commit().await,
        }
    };
    if let Err(error) = committed {
        if let Some(xid) = xid
            && matches!(
                error,
                sqlx::Error::Io(_) | sqlx::Error::Tls(_) | sqlx::Error::Protocol(_)
            )
        {
            return Ok(Enqueued::CommitUnknown { id, xid, error });
        }
        return Err(error);
    }
    Ok(Enqueued::Created(id))
}

/// Checks a PostgreSQL transaction returned by [`Enqueued::CommitUnknown`].
/// Each poll uses the read pool outside the original transaction, so it
/// neither waits for the write connection nor reuses an old snapshot.
///
/// # Errors
/// Any database error, including an unrecognized transaction status.
pub async fn enqueue_commit_status(
    read: &crate::Pool,
    xid: &str,
) -> Result<CommitStatus, sqlx::Error> {
    // This PostgreSQL-only query is reached only for CommitUnknown.
    let status: Option<String> = sqlx::query_scalar("SELECT pg_xact_status($1::text::xid8)")
        .bind(xid)
        .fetch_one(read)
        .await?;
    #[cfg(feature = "testing")]
    let _ = ENQUEUE_CHECKED.try_with(|checked| checked.notify_one());
    match status.as_deref() {
        Some("in progress") => Ok(CommitStatus::InProgress),
        Some("committed") => Ok(CommitStatus::Committed),
        Some("aborted") => Ok(CommitStatus::Aborted),
        None => Ok(CommitStatus::Unavailable),
        Some(other) => Err(sqlx::Error::Protocol(format!(
            "unexpected transaction status: {other}"
        ))),
    }
}

/// The queued or running task with this `dedupe_key`, if any.
///
/// # Errors
/// Any database error.
pub async fn active_id(read: &crate::Pool, dedupe_key: &str) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT id AS "id!: i64" FROM tasks
           WHERE dedupe_key = $1 AND state IN ('queued', 'running')"#,
        dedupe_key
    )
    .fetch_optional(read)
    .await
}

/// The state of the queued or running task with this `dedupe_key`, if any.
///
/// # Errors
/// Any database error.
pub async fn active_state(
    read: &crate::Pool,
    dedupe_key: &str,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT state FROM tasks WHERE dedupe_key = $1 AND state IN ('queued', 'running')",
        dedupe_key
    )
    .fetch_optional(read)
    .await
}

/// # Errors
/// Any database error.
pub async fn get(read: &crate::Pool, id: i64) -> Result<Option<TaskRow>, sqlx::Error> {
    sqlx::query_as!(
        TaskRow,
        r#"SELECT id AS "id!: i64", kind, dedupe_key, state, payload, result, progress, message,
                  created_at, updated_at
           FROM tasks WHERE id = $1"#,
        id
    )
    .fetch_optional(read)
    .await
}

/// One page of tasks, newest first, optionally of one state and without
/// one kind. No task has an empty kind, so `''` stands for "none left out".
///
/// # Errors
/// Any database error.
pub async fn list(
    read: &crate::Pool,
    state: Option<&str>,
    exclude_kind: Option<&str>,
    limit: u32,
    offset: u64,
) -> Result<Vec<TaskRow>, sqlx::Error> {
    let limit = i64::from(limit);
    let offset = i64::try_from(offset).unwrap_or(i64::MAX);
    sqlx::query_as!(
        TaskRow,
        r#"SELECT id AS "id!: i64", kind, dedupe_key, state, payload, result, progress, message,
                  created_at, updated_at
           FROM tasks WHERE state = COALESCE($1, state) AND kind <> COALESCE($2, '')
           ORDER BY created_at DESC, id DESC LIMIT $3 OFFSET $4"#,
        state,
        exclude_kind,
        limit,
        offset
    )
    .fetch_all(read)
    .await
}

/// The number of tasks [`list`] pages through with the same filters.
///
/// # Errors
/// Any database error.
pub async fn count(
    read: &crate::Pool,
    state: Option<&str>,
    exclude_kind: Option<&str>,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "n!: i64" FROM tasks
           WHERE state = COALESCE($1, state) AND kind <> COALESCE($2, '')"#,
        state,
        exclude_kind
    )
    .fetch_one(read)
    .await
}

/// Sets the human-readable status line of a running task, such as the
/// scan stage. The end state writes its own message.
///
/// # Errors
/// Any database error.
pub async fn set_message(
    write: &crate::Pool,
    id: i64,
    message: &str,
    now: i64,
) -> Result<(), sqlx::Error> {
    neo_core::db::retry_busy(|| async {
        sqlx::query!(
            "UPDATE tasks SET message = $1, updated_at = $2 WHERE id = $3 AND state = 'running'",
            message,
            now,
            id
        )
        .execute(write)
        .await
    })
    .await?;
    Ok(())
}

/// Moves a queued task to running. Returns false when it is neither
/// queued nor running, for example because it was cancelled while waiting.
/// A running row counts as started, so a retry after a write whose answer
/// was lost (a dropped PostgreSQL connection) still runs the task. Only
/// the task's own runner calls this, and startup recovery has already
/// ended every row a previous process left running.
///
/// # Errors
/// Any database error.
pub async fn start(write: &crate::Pool, id: i64, now: i64) -> Result<bool, sqlx::Error> {
    let done = neo_core::db::retry_busy(|| async {
        sqlx::query!(
            "UPDATE tasks SET state = 'running', progress = 0, updated_at = $1
             WHERE id = $2 AND state IN ('queued', 'running')",
            now,
            id
        )
        .execute(write)
        .await
    })
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Cancels a task that is still queued. Returns false for any other state,
/// so a task whose runner has just started it is never marked cancelled
/// under its running job.
///
/// # Errors
/// Any database error.
pub async fn cancel_queued(write: &crate::Pool, id: i64, now: i64) -> Result<bool, sqlx::Error> {
    let done = neo_core::db::retry_busy(|| async {
        sqlx::query!(
            "UPDATE tasks SET state = 'cancelled', result = NULL, message = NULL, updated_at = $1
             WHERE id = $2 AND state = 'queued'",
            now,
            id
        )
        .execute(write)
        .await
    })
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Puts a task that was marked running back in the queue, for a job that
/// never actually started because the server began shutting down during
/// the start write. Its next start comes after the restart.
///
/// # Errors
/// Any database error.
pub async fn unstart(write: &crate::Pool, id: i64, now: i64) -> Result<(), sqlx::Error> {
    neo_core::db::retry_busy(|| async {
        sqlx::query!(
            "UPDATE tasks SET state = 'queued', progress = NULL, updated_at = $1
             WHERE id = $2 AND state = 'running'",
            now,
            id
        )
        .execute(write)
        .await
    })
    .await?;
    Ok(())
}

/// Ranks a queued scan ahead of every other queued scan, for the order
/// [`recover`] queues them in after a restart. Returns false when the task
/// is not a queued scan.
///
/// # Errors
/// Any database error.
pub async fn move_to_front(write: &crate::Pool, id: i64) -> Result<bool, sqlx::Error> {
    let done = neo_core::db::retry_busy(|| async {
        sqlx::query!(
            "UPDATE tasks
             SET queue_rank = (SELECT COALESCE(MIN(queue_rank), 0) - 1 FROM tasks
                               WHERE state = 'queued' AND kind = 'scan')
             WHERE id = $1 AND state = 'queued' AND kind = 'scan'",
            id
        )
        .execute(write)
        .await
    })
    .await?;
    Ok(done.rows_affected() == 1)
}

/// # Errors
/// Any database error.
pub async fn set_progress(
    write: &crate::Pool,
    id: i64,
    progress: f64,
    now: i64,
) -> Result<(), sqlx::Error> {
    neo_core::db::retry_busy(|| async {
        sqlx::query!(
            "UPDATE tasks SET progress = $1, updated_at = $2 WHERE id = $3 AND state = 'running'",
            progress,
            now,
            id
        )
        .execute(write)
        .await
    })
    .await?;
    Ok(())
}

/// Ends an active task. Returns false when it had already ended, so a late
/// writer cannot overwrite the first end state.
///
/// # Errors
/// Any database error.
pub async fn finish(
    write: &crate::Pool,
    id: i64,
    state: EndState,
    result: Option<&str>,
    message: Option<&str>,
    now: i64,
) -> Result<bool, sqlx::Error> {
    let state = state.as_str();
    let done = neo_core::db::retry_busy(|| async {
        sqlx::query!(
            "UPDATE tasks
             SET state = $1, result = $2, message = $3, updated_at = $4,
                 progress = CASE WHEN $5 = 'done' THEN 1.0 ELSE progress END
             WHERE id = $6 AND state IN ('queued', 'running')",
            state,
            result,
            message,
            now,
            state,
            id
        )
        .execute(write)
        .await
    })
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Startup recovery: tasks left `running` by the previous process become
/// `interrupted`, never `done`. Returns those, for the scheduler to log,
/// and the tasks still `queued` for it to pick up again: oldest first,
/// except that scans moved to the front come first, the last moved first.
///
/// # Errors
/// Any database error.
pub async fn recover(write: &crate::Pool, now: i64) -> Result<Recovered, sqlx::Error> {
    let interrupted = sqlx::query_as!(
        TaskRow,
        r#"UPDATE tasks SET state = 'interrupted', message = '服务重启，任务被中断', updated_at = $1
           WHERE state = 'running'
           RETURNING id AS "id!: i64", kind, dedupe_key, state, payload, result, progress,
                     message, created_at, updated_at"#,
        now
    )
    .fetch_all(write)
    .await?;
    let queued = sqlx::query_as!(
        TaskRow,
        r#"SELECT id AS "id!: i64", kind, dedupe_key, state, payload, result, progress, message,
                  created_at, updated_at
           FROM tasks WHERE state = 'queued' ORDER BY COALESCE(queue_rank, 0), id"#
    )
    .fetch_all(write)
    .await?;
    Ok(Recovered {
        interrupted,
        queued,
    })
}

/// Deletes finished tasks (`done`, `failed`, `cancelled`, `interrupted`)
/// last updated before `older_than`, and of the rest all but the newest
/// `keep`. Queued and running tasks are never deleted. Returns how many
/// rows went.
///
/// # Errors
/// Any database error.
pub async fn prune(write: &crate::Pool, older_than: i64, keep: i64) -> Result<u64, sqlx::Error> {
    let done = neo_core::db::retry_busy(|| async {
        sqlx::query!(
            "DELETE FROM tasks
             WHERE state IN ('done', 'failed', 'cancelled', 'interrupted')
               AND (updated_at < $1
                    OR id NOT IN (SELECT id FROM tasks
                                  WHERE state IN ('done', 'failed', 'cancelled', 'interrupted')
                                  ORDER BY updated_at DESC, id DESC
                                  LIMIT $2))",
            older_than,
            keep
        )
        .execute(write)
        .await
    })
    .await?;
    Ok(done.rows_affected())
}
