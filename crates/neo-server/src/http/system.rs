use axum::Json;
use axum::extract::State;
use neo_core::dto::{HealthResponse, HealthStatus, ReadyResponse, ReadyStatus, VersionResponse};

use super::{AppState, HttpError};

pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: HealthStatus::Ok,
    })
}

pub async fn ready(State(state): State<AppState>) -> Result<Json<ReadyResponse>, HttpError> {
    if state.is_ready() {
        Ok(Json(ReadyResponse {
            status: ReadyStatus::Ready,
        }))
    } else {
        Err(HttpError::not_ready())
    }
}

pub async fn version() -> Json<VersionResponse> {
    Json(crate::version())
}
