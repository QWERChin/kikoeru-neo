//! `POST /api/works/{id}/refresh`: refetch one work's metadata as a task.

use std::time::Duration;

use axum::{Json, extract::State, http::StatusCode};
use futures_util::FutureExt;
use neo_core::dto::{ErrorCode, Id, RefreshResponse};
use neo_db::tasks::TaskRow;
use neo_scanner::metadata::{FetchOutcome, fetch_and_store};
use serde::Deserialize;
use serde_json::json;

use super::{
    AppState, HttpError,
    extract::{AdminUser, ApiPath, ApiQuery},
    works::work_id,
};
use crate::tasks::{Job, JobError, Lane, TaskSpec};

const KIND: &str = "metadata";
/// One fetch is two requests and a cover, each with up to three retries.
const TIMEOUT: Duration = Duration::from_secs(5 * 60);

#[derive(Deserialize)]
pub struct Params {
    force: Option<String>,
}

pub async fn refresh(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    ApiPath(id): ApiPath<String>,
    ApiQuery(params): ApiQuery<Params>,
) -> Result<(StatusCode, Json<RefreshResponse>), HttpError> {
    let id = work_id(&id)?;
    let force = match params.force.as_deref() {
        None | Some("false") => false,
        Some("true") => true,
        Some(v) => {
            return Err(HttpError::invalid_request(format!(
                "force 只能是 true 或 false，收到 {v:?}"
            )));
        }
    };
    let exists = neo_db::works::exists(state.db(), id).await?;
    if !exists {
        return Err(HttpError::work_not_found());
    }
    if state.metadata().is_none() {
        return Err(HttpError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::UpstreamError,
            "没有配置元数据来源",
        ));
    }
    let submitted = state
        .scheduler()
        .submit(spec(id, force), job(state.clone(), id, force))
        .await
        .map_err(HttpError::internal)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(RefreshResponse {
            task_id: Id::try_from(submitted.id).map_err(HttpError::internal)?,
        }),
    ))
}

/// One fetch per work at a time: a second request returns the running one,
/// whatever its `force`.
fn spec(work_id: i64, force: bool) -> TaskSpec {
    TaskSpec {
        kind: KIND,
        dedupe_key: Some(format!("metadata:{work_id}")),
        payload: json!({ "work_id": work_id, "force": force }),
        lane: Lane::Light,
        timeout: Some(TIMEOUT),
        queue_limit: None,
    }
}

fn job(state: AppState, work_id: i64, force: bool) -> Job {
    Box::new(move |ctx| {
        async move {
            let Some(meta) = state.metadata().cloned() else {
                return Err(JobError("没有配置元数据来源".to_owned()));
            };
            let now = neo_db::now_ms();
            let fetch = fetch_and_store(
                state.db(),
                meta.provider.as_ref(),
                &meta.dirs,
                work_id,
                force,
                now,
            );
            let outcome = tokio::select! {
                outcome = fetch => outcome,
                () = ctx.cancel.cancelled() => return Err(JobError("已取消".to_owned())),
            };
            match outcome {
                Ok(FetchOutcome::Updated | FetchOutcome::ManualKept) => Ok(None),
                Ok(FetchOutcome::NotFound) => Err(JobError("DLsite 上没有这部作品".to_owned())),
                Ok(FetchOutcome::NoWork) => Err(JobError("作品已经不存在".to_owned())),
                Ok(FetchOutcome::Unavailable(e)) => {
                    tracing::warn!(work = work_id, error = %e, "metadata refresh failed");
                    Err(JobError("DLsite 暂时无法访问".to_owned()))
                }
                Ok(FetchOutcome::CoverNotStored(e)) => {
                    tracing::error!(work = work_id, error = %e, "cannot store a cover");
                    Err(JobError("封面写不进数据目录，元数据也没有保存".to_owned()))
                }
                Err(e) => {
                    tracing::error!(work = work_id, error = %e, "metadata refresh failed");
                    Err(JobError("保存元数据时出错".to_owned()))
                }
            }
        }
        .boxed()
    })
}

/// Rebuilds a queued metadata task left by the previous process.
pub fn rebuild_metadata_task(
    state: &AppState,
    row: &TaskRow,
) -> Option<(Lane, Option<Duration>, Job)> {
    if row.kind != KIND {
        return None;
    }
    let payload: serde_json::Value = serde_json::from_str(&row.payload).ok()?;
    let work_id = payload.get("work_id")?.as_i64()?;
    let force = payload
        .get("force")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    Some((
        Lane::Light,
        Some(TIMEOUT),
        job(state.clone(), work_id, force),
    ))
}
