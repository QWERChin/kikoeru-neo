//! Library routes: favorites, history, the user's reviews and the filter
//! lists.

use axum::{Json, extract::State, http::StatusCode};
use neo_core::{
    dto::{CircleCount, Page, Review, ReviewInput, TagCount, VaCount, WorkSummary},
    review::{self, InputError, MAX_TEXT_CHARS},
};
use neo_db::{library, reviews, works::Scope};

use super::{
    AppState, HttpError,
    extract::{ApiJson, ApiPath, ApiQuery, CurrentUser},
    works::{PageParams, user_list, work_id},
};

pub async fn favorites(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiQuery(params): ApiQuery<PageParams>,
) -> Result<Json<Page<WorkSummary>>, HttpError> {
    user_list(&state, user.id, Scope::Favorites, &params).await
}

pub async fn history(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiQuery(params): ApiQuery<PageParams>,
) -> Result<Json<Page<WorkSummary>>, HttpError> {
    user_list(&state, user.id, Scope::History, &params).await
}

pub async fn reviewed(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiQuery(params): ApiQuery<PageParams>,
) -> Result<Json<Page<WorkSummary>>, HttpError> {
    user_list(&state, user.id, Scope::Reviews, &params).await
}

pub async fn put_review(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiPath(id): ApiPath<String>,
    ApiJson(body): ApiJson<ReviewInput>,
) -> Result<Json<Review>, HttpError> {
    let id = work_id(&id)?;
    let valid = review::validate(&body).map_err(|e| {
        HttpError::invalid_request(match e {
            InputError::Rating => "评分要是 1 到 5 的整数".to_owned(),
            InputError::TooLong => format!("短评最多 {MAX_TEXT_CHARS} 个字"),
            InputError::Empty => "评分和短评不能都为空，要清除请用 DELETE".to_owned(),
        })
    })?;
    let now = neo_db::now_ms();
    reviews::put(state.db(), user.id, id, &valid, now)
        .await
        .map_err(HttpError::internal)?
        .map(Json)
        .ok_or_else(HttpError::work_not_found)
}

pub async fn delete_review(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiPath(id): ApiPath<String>,
) -> Result<StatusCode, HttpError> {
    let id = work_id(&id)?;
    if reviews::delete(state.db(), user.id, id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(HttpError::work_not_found())
    }
}

pub async fn add_favorite(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiPath(id): ApiPath<String>,
) -> Result<StatusCode, HttpError> {
    set_favorite(&state, user.id, &id, true).await
}

pub async fn remove_favorite(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    ApiPath(id): ApiPath<String>,
) -> Result<StatusCode, HttpError> {
    set_favorite(&state, user.id, &id, false).await
}

async fn set_favorite(
    state: &AppState,
    user_id: i64,
    id: &str,
    favorite: bool,
) -> Result<StatusCode, HttpError> {
    let id = work_id(id)?;
    let now = neo_db::now_ms();
    if library::set_favorite(state.db(), user_id, id, favorite, now).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(HttpError::work_not_found())
    }
}

pub async fn circles(
    State(state): State<AppState>,
    CurrentUser(_): CurrentUser,
) -> Result<Json<Vec<CircleCount>>, HttpError> {
    library::circles(state.db())
        .await
        .map(Json)
        .map_err(HttpError::internal)
}

pub async fn vas(
    State(state): State<AppState>,
    CurrentUser(_): CurrentUser,
) -> Result<Json<Vec<VaCount>>, HttpError> {
    library::vas(state.db())
        .await
        .map(Json)
        .map_err(HttpError::internal)
}

pub async fn tags(
    State(state): State<AppState>,
    CurrentUser(_): CurrentUser,
) -> Result<Json<Vec<TagCount>>, HttpError> {
    library::tags(state.db())
        .await
        .map(Json)
        .map_err(HttpError::internal)
}
