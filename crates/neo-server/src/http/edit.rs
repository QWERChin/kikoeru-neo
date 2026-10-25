//! Admin edits: `PUT /api/works/{id}/metadata`, `GET /api/admin/missing`
//! and `POST /api/admin/missing/cleanup`.

use axum::{Json, extract::State};
use neo_core::{
    date::is_calendar_date,
    dto::{CleanupRequest, CleanupResponse, MetadataRequest, MissingReport, WorkDetail},
};
use neo_db::{
    detail,
    edit::{self, MetadataEdit},
};

use super::{
    AppState, HttpError,
    extract::{AdminUser, ApiJson, ApiPath},
    works::work_id,
};

const MAX_TEXT: usize = 500;
const MAX_NAMES: usize = 200;

fn text(field: &str, value: &str) -> Result<String, HttpError> {
    let value = value.trim();
    if value.chars().count() > MAX_TEXT || value.chars().any(char::is_control) {
        return Err(HttpError::invalid_request(format!(
            "{field} 不能超过 {MAX_TEXT} 个字符，也不能有控制字符"
        )));
    }
    Ok(value.to_owned())
}

/// Trims, drops empty names and repeats, and keeps the order given.
fn names(field: &str, values: &[String]) -> Result<Vec<String>, HttpError> {
    if values.len() > MAX_NAMES {
        return Err(HttpError::invalid_request(format!(
            "{field} 最多 {MAX_NAMES} 个"
        )));
    }
    let mut out: Vec<String> = Vec::with_capacity(values.len());
    for v in values {
        let v = text(field, v)?;
        if !v.is_empty() && !out.contains(&v) {
            out.push(v);
        }
    }
    Ok(out)
}

fn validate(request: &MetadataRequest) -> Result<MetadataEdit, HttpError> {
    let title = text("title", &request.title)?;
    if title.is_empty() {
        return Err(HttpError::invalid_request("title 不能为空"));
    }
    let release_date = match &request.release_date {
        None => None,
        Some(d) if is_calendar_date(d) => Some(d.clone()),
        Some(d) => {
            return Err(HttpError::invalid_request(format!(
                "release_date 必须是 YYYY-MM-DD，收到 {d:?}"
            )));
        }
    };
    let circle_name = match &request.circle_name {
        Some(name) => Some(text("circle_name", name)?).filter(|n| !n.is_empty()),
        None => None,
    };
    Ok(MetadataEdit {
        title,
        release_date,
        nsfw: request.nsfw,
        circle_name,
        va_names: names("va_names", &request.va_names)?,
        tag_names: names("tag_names", &request.tag_names)?,
    })
}

const MAX_CLEANUP_IDS: usize = 1000;

pub async fn set_metadata(
    State(state): State<AppState>,
    AdminUser(user): AdminUser,
    ApiPath(id): ApiPath<String>,
    ApiJson(request): ApiJson<MetadataRequest>,
) -> Result<Json<WorkDetail>, HttpError> {
    let id = work_id(&id)?;
    let edit = validate(&request)?;
    let now = neo_db::now_ms();
    if !edit::set_metadata(state.db(), id, &edit, now).await? {
        return Err(HttpError::work_not_found());
    }
    detail::get(state.db(), user.id, id, user.is_admin)
        .await
        .map_err(super::works::detail_error)?
        .map(Json)
        .ok_or_else(HttpError::work_not_found)
}

pub async fn missing(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
) -> Result<Json<MissingReport>, HttpError> {
    edit::missing(state.db())
        .await
        .map(Json)
        .map_err(HttpError::internal)
}

pub async fn cleanup(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    ApiJson(request): ApiJson<CleanupRequest>,
) -> Result<Json<CleanupResponse>, HttpError> {
    // Every ID is checked and deleted inside one write transaction.
    if request.work_ids.len() + request.location_ids.len() > MAX_CLEANUP_IDS {
        return Err(HttpError::invalid_request(format!(
            "一次最多清理 {MAX_CLEANUP_IDS} 项"
        )));
    }
    let works: Vec<i64> = request.work_ids.iter().map(|id| i64::from(*id)).collect();
    let locations: Vec<i64> = request
        .location_ids
        .iter()
        .map(|id| i64::from(*id))
        .collect();
    // A scan holds snapshots of works and locations across transactions,
    // and deleting one under it would abort the scan.
    let _slot = state
        .scheduler()
        .try_scan_slot()
        .ok_or_else(HttpError::scan_running)?;
    // A backup copies the database and then covers/ under this lock. Held
    // over the delete and the cover removal, it keeps a backup from carrying
    // a deleted work's row without its cover.
    let covers = state
        .data_dir()
        .ok()
        .map(|d| d.root().join("covers"))
        .filter(|c| c.is_dir());
    let _covers_lock = match &covers {
        Some(covers) => Some(
            neo_scanner::metadata::wait_for_covers(covers)
                .await
                .map_err(HttpError::internal)?,
        ),
        None => None,
    };
    let done = edit::cleanup(state.db(), &works, &locations).await?;
    // Covers and thumbnails of deleted works are files, outside the
    // transaction. A leftover file only wastes space.
    if let Ok(data_dir) = state.data_dir() {
        for id in &done.works {
            let _ = std::fs::remove_file(data_dir.root().join(format!("covers/{id}.jpg")));
            let thumbs = data_dir.root().join("cache/thumbs");
            if let Ok(entries) = std::fs::read_dir(&thumbs) {
                let prefix = format!("{id}-");
                for entry in entries.flatten() {
                    if entry.file_name().to_string_lossy().starts_with(&prefix) {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }
        }
    }
    Ok(Json(CleanupResponse {
        works_deleted: (done.works.len() as i64)
            .try_into()
            .map_err(HttpError::internal)?,
        locations_deleted: i64::try_from(done.locations)
            .map_err(HttpError::internal)?
            .try_into()
            .map_err(HttpError::internal)?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_trimmed_and_deduplicated() {
        let got = names("x", &[" a ".into(), "".into(), "a".into(), "b".into()]).unwrap();
        assert_eq!(got, ["a", "b"]);
        assert!(names("x", &["bad\u{7}".into()]).is_err());
    }
}
