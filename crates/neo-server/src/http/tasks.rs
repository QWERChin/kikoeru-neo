//! `GET /api/tasks/{id}`, the admin task list, cancel, moving a queued scan
//! to the front and the event stream.

use std::{convert::Infallible, time::Duration};

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
};
use futures_util::{Stream, stream};
use neo_core::dto::{ErrorCode, Fraction, Page, PlayableMedia, TaskKind, TaskSnapshot, TaskState};
use neo_db::tasks::{self as task_db, TaskRow};
use serde::Deserialize;

use super::{
    AppState, HttpError,
    extract::{AdminUser, ApiPath, ApiQuery, CurrentUser},
};
use crate::tasks::{Moved, Scheduler};

/// How often the event stream looks for changes. Progress is written at
/// most once a second, so this sees every write.
const EVENT_POLL: Duration = Duration::from_millis(500);
const EVENT_PING: Duration = Duration::from_secs(15);
const STATES: [&str; 6] = [
    "queued",
    "running",
    "done",
    "failed",
    "cancelled",
    "interrupted",
];
/// Every kind `snapshot` knows, as the list's `exclude_kind` accepts them.
const KINDS: [&str; 5] = ["scan", "probe", "transcode", "metadata", "analysis"];

fn is_terminal(state: &str) -> bool {
    matches!(state, "done" | "failed" | "cancelled" | "interrupted")
}

/// Admins see every task. Other users see only the transcode and analysis
/// tasks they asked for through `prepare` or the loudness and waveform
/// endpoints, and get 404 for anything else, so task IDs do not reveal what
/// other users play.
pub async fn get(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiPath(id): ApiPath<i64>,
) -> Result<Json<TaskSnapshot>, HttpError> {
    let row = state
        .scheduler()
        .get(id)
        .await?
        .ok_or_else(HttpError::not_found)?;
    let visible = user.is_admin
        || (matches!(row.kind.as_str(), "transcode" | "analysis")
            && state.scheduler().is_requester(id, user.id));
    if !visible {
        return Err(HttpError::not_found());
    }
    snapshot(row, state.scheduler()).map(Json)
}

/// Where a queued scan stands in the scan lane. A cancelled scan may wait in
/// the queue a moment after its row ended, and gets no position.
fn queue_position(row: &TaskRow, scheduler: &Scheduler) -> Option<usize> {
    (row.state == "queued")
        .then(|| scheduler.queue_position(row.id))
        .flatten()
}

fn snapshot(row: TaskRow, scheduler: &Scheduler) -> Result<TaskSnapshot, HttpError> {
    let position = queue_position(&row, scheduler);
    snapshot_at(row, position)
}

fn snapshot_at(row: TaskRow, position: Option<usize>) -> Result<TaskSnapshot, HttpError> {
    let kind = match row.kind.as_str() {
        "scan" => TaskKind::Scan,
        "probe" => TaskKind::Probe,
        "transcode" => TaskKind::Transcode,
        "metadata" => TaskKind::Metadata,
        "analysis" => TaskKind::Analysis,
        other => return Err(HttpError::internal(format!("unknown task kind {other:?}"))),
    };
    let state = match row.state.as_str() {
        "queued" => TaskState::Queued,
        "running" => TaskState::Running,
        "done" => TaskState::Done,
        "failed" => TaskState::Failed,
        "cancelled" => TaskState::Cancelled,
        "interrupted" => TaskState::Interrupted,
        other => return Err(HttpError::internal(format!("unknown task state {other:?}"))),
    };
    // Only a finished transcode exposes its result, as the contract says.
    let result = match (kind, state, row.result.as_deref()) {
        (TaskKind::Transcode, TaskState::Done, Some(json)) => {
            Some(serde_json::from_str::<PlayableMedia>(json).map_err(HttpError::internal)?)
        }
        _ => None,
    };
    let time = |ms: i64| ms.try_into().map_err(HttpError::internal);
    Ok(TaskSnapshot {
        id: row.id.try_into().map_err(HttpError::internal)?,
        kind,
        state,
        progress: row.progress.map(Fraction),
        message: row.message,
        result,
        queue_position: position
            .map(u32::try_from)
            .transpose()
            .map_err(HttpError::internal)?,
        created_at: time(row.created_at)?,
        updated_at: time(row.updated_at)?,
    })
}

#[derive(Deserialize)]
pub struct ListParams {
    page: Option<String>,
    page_size: Option<String>,
    state: Option<String>,
    exclude_kind: Option<String>,
}

/// The parameter's value when it is one of `allowed`, so it can go into
/// the query as it is.
fn one_of<'a>(
    name: &str,
    value: Option<&'a str>,
    allowed: &[&str],
) -> Result<Option<&'a str>, HttpError> {
    match value {
        None => Ok(None),
        Some(v) if allowed.contains(&v) => Ok(Some(v)),
        Some(v) => Err(HttpError::invalid_request(format!(
            "{name} 只能是 {}，收到 {v:?}",
            allowed.join("、")
        ))),
    }
}

pub async fn list(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    ApiQuery(params): ApiQuery<ListParams>,
) -> Result<Json<Page<TaskSnapshot>>, HttpError> {
    let (page, page_size) =
        super::works::paging(params.page.as_deref(), params.page_size.as_deref())?;
    let filter = one_of("state", params.state.as_deref(), &STATES)?;
    let exclude_kind = one_of("exclude_kind", params.exclude_kind.as_deref(), &KINDS)?;
    let db = state.db();
    let total = task_db::count(db, filter, exclude_kind).await?;
    let rows = task_db::list(
        db,
        filter,
        exclude_kind,
        page_size,
        u64::from(page - 1) * u64::from(page_size),
    )
    .await?;
    let items = rows
        .into_iter()
        .map(|row| snapshot(row, state.scheduler()))
        .collect::<Result<_, _>>()?;
    Ok(Json(Page {
        items,
        page,
        page_size,
        total: total.try_into().map_err(HttpError::internal)?,
    }))
}

pub async fn cancel(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    ApiPath(id): ApiPath<i64>,
) -> Result<StatusCode, HttpError> {
    let row = state
        .scheduler()
        .get(id)
        .await?
        .ok_or_else(HttpError::not_found)?;
    if is_terminal(&row.state) {
        return Err(HttpError::invalid_request("任务已经结束"));
    }
    // False only when the task ended between the read and the cancel,
    // which is the same answer as above.
    if !state.scheduler().cancel(id).await? {
        return Err(HttpError::invalid_request("任务已经结束"));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Moves a queued scan to the front of the scan lane. The running scan is
/// left alone: making room for another scan means cancelling it.
pub async fn move_to_front(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    ApiPath(id): ApiPath<i64>,
) -> Result<StatusCode, HttpError> {
    let row = state
        .scheduler()
        .get(id)
        .await?
        .ok_or_else(HttpError::not_found)?;
    if row.kind != "scan" {
        return Err(HttpError::invalid_request("只有扫描任务可以调整顺序"));
    }
    if row.state != "queued" {
        return Err(HttpError::invalid_request("扫描已经开始或已经结束"));
    }
    // NotWaiting when the scan started between the read and the move.
    match state.scheduler().move_to_front(id).await? {
        Moved::Front => Ok(StatusCode::NO_CONTENT),
        Moved::NotWaiting => Err(HttpError::invalid_request("扫描已经开始或已经结束")),
    }
}

/// A snapshot whenever the task changes, the last one when it ends, then
/// the end of the stream. The stream also ends when the server shuts down.
/// Closing the stream only ends the subscription; the task keeps running.
pub async fn events(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    ApiPath(id): ApiPath<i64>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, HttpError> {
    let first = state
        .scheduler()
        .get(id)
        .await?
        .ok_or_else(HttpError::not_found)?;
    let slot = state.event_streams().try_acquire_owned().map_err(|_| {
        HttpError::new(
            StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::QueueFull,
            "同时打开的任务进度连接太多，请关闭一些再试",
        )
    })?;
    struct Watch {
        /// Held for as long as the stream is open.
        _slot: tokio::sync::OwnedSemaphorePermit,
        state: AppState,
        id: i64,
        /// The last row sent, with the queue position it was sent with.
        last: Option<(TaskRow, Option<usize>)>,
        next: Option<TaskRow>,
        done: bool,
    }
    let start = Watch {
        _slot: slot,
        state,
        id,
        last: None,
        next: Some(first),
        done: false,
    };
    let events = stream::unfold(start, |mut w| async move {
        if w.done {
            return None;
        }
        loop {
            let row = match w.next.take() {
                Some(row) => row,
                None => {
                    // A queued task stays queued through shutdown, so the
                    // stream would otherwise keep the connection drain open.
                    tokio::select! {
                        () = tokio::time::sleep(EVENT_POLL) => {}
                        () = w.state.scheduler().shutting_down() => return None,
                    }
                    match w.state.scheduler().get(w.id).await {
                        Ok(Some(row)) => row,
                        // The task row is gone or the database failed: end
                        // the stream rather than spin.
                        Ok(None) | Err(_) => return None,
                    }
                }
            };
            // A queued scan moves up when one ahead of it starts, ends or is
            // moved behind it, none of which touches its own row.
            let position = queue_position(&row, w.state.scheduler());
            let changed = w.last.as_ref().is_none_or(|(last, last_position)| {
                (
                    &last.state,
                    last.progress,
                    &last.message,
                    last.updated_at,
                    *last_position,
                ) != (
                    &row.state,
                    row.progress,
                    &row.message,
                    row.updated_at,
                    position,
                )
            });
            if !changed {
                continue;
            }
            w.done = is_terminal(&row.state);
            let event = match snapshot_at(row.clone(), position)
                .map(|s| Event::default().event("snapshot").json_data(s))
            {
                Ok(Ok(event)) => event,
                _ => return None,
            };
            w.last = Some((row, position));
            return Some((Ok(event), w));
        }
    });
    Ok(Sse::new(events).keep_alive(KeepAlive::new().interval(EVENT_PING).text("ping")))
}
