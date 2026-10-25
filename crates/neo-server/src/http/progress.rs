//! Progress reports: `PUT`, `POST .../finish` and `DELETE` on
//! `/api/me/progress/{work_id}`.

use axum::{extract::State, http::StatusCode};
use neo_core::dto::{FinishRequest, ProgressRequest};
use neo_db::{now_ms, progress};

use super::{
    AppState, HttpError,
    extract::{ApiJson, ApiPath, CurrentUser},
};

/// The longest path Linux accepts. A group is a directory inside the work,
/// and the history page returns it for every listed work.
const MAX_GROUP_PATH_BYTES: usize = 4096;

fn check_group(group_path: &str) -> Result<(), HttpError> {
    if group_path.len() > MAX_GROUP_PATH_BYTES {
        return Err(HttpError::invalid_request(format!(
            "group_path 最多 {MAX_GROUP_PATH_BYTES} 字节"
        )));
    }
    Ok(())
}

pub async fn report(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiPath(work_id): ApiPath<i64>,
    ApiJson(body): ApiJson<ProgressRequest>,
) -> Result<StatusCode, HttpError> {
    let track_id = i64::from(body.track_id);
    let position_ms = i64::from(body.position_ms);
    if position_ms < 0 {
        return Err(HttpError::invalid_request("position_ms 不能是负数"));
    }
    check_group(&body.group_path)?;
    if let Some(order) = &body.report_order {
        if order.session_id.len() != 32
            || !order
                .session_id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(HttpError::invalid_request(
                "session_id 必须是 32 个小写十六进制字符",
            ));
        }
        if i64::from(order.sequence) <= 0 {
            return Err(HttpError::invalid_request("sequence 必须是正整数"));
        }
    }
    check_track(&state, work_id, track_id).await?;
    progress::report(
        state.db(),
        user.id,
        work_id,
        &progress::ReportInput {
            track_id,
            position_ms,
            group_path: &body.group_path,
            report_order: body
                .report_order
                .as_ref()
                .map(|order| progress::ReportOrder {
                    session_id: &order.session_id,
                    sequence: i64::from(order.sequence),
                }),
        },
        now_ms(),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The player calls this when the last track of the chosen group ends. The
/// backend checks that it really is the last one, so a skipped or reordered
/// queue never marks a work as listened. The "mark as listened" button sends
/// `manual: true` and skips the check.
pub async fn finish(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiPath(work_id): ApiPath<i64>,
    ApiJson(body): ApiJson<FinishRequest>,
) -> Result<StatusCode, HttpError> {
    let track_id = i64::from(body.track_id);
    check_group(&body.group_path)?;
    check_track(&state, work_id, track_id).await?;
    if !body.manual {
        let group = body.group_path.trim_end_matches('/');
        if !progress::ends_group(state.db(), work_id, group, track_id).await? {
            return Err(HttpError::invalid_request("这条轨道不是这个分组的最后一条"));
        }
    }
    progress::finish(
        state.db(),
        user.id,
        work_id,
        track_id,
        &body.group_path,
        now_ms(),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn clear(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiPath(work_id): ApiPath<i64>,
) -> Result<StatusCode, HttpError> {
    progress::clear(state.db(), user.id, work_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn check_track(state: &AppState, work_id: i64, track_id: i64) -> Result<(), HttpError> {
    let place = progress::track_place(state.db(), track_id).await?;
    match place {
        Some(place) if place.work_id == work_id => Ok(()),
        _ => Err(HttpError::invalid_request("这条轨道不属于这部作品")),
    }
}
