use axum::{Json, extract::State, http::StatusCode};
use neo_core::dto::{
    ErrorCode, Id, RootHealth, RootStatus, ScanRequest, ScanResponse, TagMergeRequest,
};
use neo_db::tags::{self, MergeOutcome};

use super::{
    AppState, HttpError, RootEntry,
    extract::{AdminUser, ApiJson},
};
use crate::scan;

/// The roots from config.toml in config order, then those added through
/// the API in the order they were added. A root that left config.toml keeps
/// its database row but is not listed, because nothing can scan it.
pub async fn roots(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
) -> Result<Json<Vec<RootStatus>>, HttpError> {
    let roots = state.roots();
    let mut out = Vec::with_capacity(roots.len());
    for root in &roots {
        out.push(root_status(&state, root).await?);
    }
    Ok(Json(out))
}

/// One root as `GET /api/admin/roots` lists it.
pub(super) async fn root_status(
    state: &AppState,
    root: &RootEntry,
) -> Result<RootStatus, HttpError> {
    let db = state.db();
    let automation = state.automation();
    let next_scan_at = crate::autoscan::next_scan_at(state.db(), automation, root)
        .await
        .map_err(HttpError::internal)?;
    let id = &root.id;
    let row = neo_db::roots::status(db, id)
        .await?
        .ok_or_else(|| HttpError::internal(format!("root {id} has no database row")))?;
    Ok(RootStatus {
        id: id.clone(),
        display_name: row.display_name,
        kind: root.kind,
        status: match row.status.as_str() {
            "ok" => RootHealth::Ok,
            "unavailable" => RootHealth::Unavailable,
            _ => RootHealth::Unknown,
        },
        last_scan_at: row
            .last_scan_at
            .map(TryInto::try_into)
            .transpose()
            .map_err(HttpError::internal)?,
        work_count: row.work_count.try_into().map_err(HttpError::internal)?,
        source: root.source,
        settings: root.settings.clone(),
        next_scan_at: next_scan_at
            .map(TryInto::try_into)
            .transpose()
            .map_err(HttpError::internal)?,
        watching: automation.is_watching(id),
    })
}

/// Starts a scan of one root, or of every root that has a storage backend
/// when `root` is null. A root already being scanned answers with its
/// running task.
pub async fn scan(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    ApiJson(request): ApiJson<ScanRequest>,
) -> Result<(StatusCode, Json<ScanResponse>), HttpError> {
    // Held until the tasks are recorded, so a root cannot be removed
    // between finding its storage and queueing its scan. The admission runs
    // to the end even if the client goes away, still holding it.
    let changes = state.lock_root_changes().await;
    let task_ids = super::roots::run_to_end(async move {
        let _changes = changes;
        let roots: Vec<_> = match &request.root {
            Some(id) => {
                let root = state
                    .roots()
                    .into_iter()
                    .find(|r| &r.id == id)
                    .ok_or_else(|| {
                        HttpError::new(StatusCode::NOT_FOUND, ErrorCode::NotFound, "没有这个媒体库")
                    })?;
                if state.storage(id).is_none() {
                    return Err(HttpError::new(
                        StatusCode::SERVICE_UNAVAILABLE,
                        ErrorCode::StorageUnavailable,
                        format!("媒体库 {id} 加载失败，请查看服务器日志"),
                    ));
                }
                vec![(root.id.clone(), root.kind)]
            }
            // Roots whose backend failed to load are skipped.
            None => state
                .roots()
                .iter()
                .filter(|r| state.storage(&r.id).is_some())
                .map(|r| (r.id.clone(), r.kind))
                .collect(),
        };
        let full = request.full.unwrap_or(false);
        let env = state.scan_env().with_full(full);
        let mut task_ids = Vec::with_capacity(roots.len());
        for (id, kind) in roots {
            let storage = state
                .storage(&id)
                .ok_or_else(|| HttpError::internal(format!("root {id} lost its storage")))?;
            let submitted = state
                .scheduler()
                .submit(
                    scan::spec(&id, full),
                    scan::job(env.clone(), id.clone(), storage, kind),
                )
                .await
                .map_err(HttpError::internal)?;
            task_ids.push(Id::try_from(submitted.id).map_err(HttpError::internal)?);
        }
        Ok(task_ids)
    })
    .await?;
    Ok((StatusCode::ACCEPTED, Json(ScanResponse { task_ids })))
}

pub async fn merge_tags(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    ApiJson(request): ApiJson<TagMergeRequest>,
) -> Result<StatusCode, HttpError> {
    let (from, into) = (i64::from(request.from), i64::from(request.into));
    match tags::merge(state.db(), from, into, neo_db::now_ms()).await? {
        MergeOutcome::Merged => Ok(StatusCode::NO_CONTENT),
        MergeOutcome::SameTag => Err(HttpError::invalid_request("不能把标签合并进它自己")),
        MergeOutcome::NotFound => Err(HttpError::invalid_request("要合并的标签不存在")),
    }
}
