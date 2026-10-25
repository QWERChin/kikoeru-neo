//! `GET` and `PUT /api/admin/settings` and the proxy check
//! (book/src/contract.md, "服务器设置").

use axum::{Json, extract::State, http::StatusCode};
use neo_core::dto::{ErrorCode, ProxyCheckResult, ServerSettings};
use neo_db::{now_ms, server_settings};
use neo_scanner::metadata::{MetadataError, ProxyChoice, ProxyUrl};

use super::{
    AppState, HttpError,
    extract::{AdminUser, ApiJson},
};

pub async fn get(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
) -> Result<Json<ServerSettings>, HttpError> {
    let metadata_proxy = server_settings::metadata_proxy(state.db()).await?;
    Ok(Json(ServerSettings { metadata_proxy }))
}

/// `PUT /api/admin/settings`: saves the proxy, then hands it to the running
/// metadata clients, so it applies without a restart.
pub async fn put(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    ApiJson(body): ApiJson<ServerSettings>,
) -> Result<Json<ServerSettings>, HttpError> {
    let proxy = parse(&body)?;
    let stored = proxy.as_ref().map(|p| p.as_str().to_owned());
    // On a task of its own: a client that disconnects between the write and
    // the switch would otherwise leave the clients on the old proxy.
    super::roots::run_to_end(async move { apply(&state, proxy).await }).await?;
    Ok(Json(ServerSettings {
        metadata_proxy: stored,
    }))
}

async fn apply(state: &AppState, proxy: Option<ProxyUrl>) -> Result<(), HttpError> {
    let stored = proxy.as_ref().map(ProxyUrl::as_str);
    // Held from the write to the switch, so two saves cannot leave the
    // database on one proxy and the clients on the other.
    let _changing = state.settings_changes().lock().await;
    server_settings::set_metadata_proxy(state.db(), stored, now_ms()).await?;
    if let Some(metadata) = state.metadata() {
        metadata
            .provider
            .set_proxy(&choice(proxy.clone()))
            .await
            .map_err(HttpError::internal)?;
    }
    tracing::info!(
        proxy = stored.unwrap_or("from the environment"),
        "metadata proxy changed"
    );
    Ok(())
}

/// `POST /api/admin/settings/proxy-check`: one request to DLsite through
/// the proxy in the body, which is not saved.
pub async fn check(
    State(state): State<AppState>,
    AdminUser(_): AdminUser,
    ApiJson(body): ApiJson<ServerSettings>,
) -> Result<Json<ProxyCheckResult>, HttpError> {
    let proxy = parse(&body)?;
    let Some(metadata) = state.metadata() else {
        return Err(HttpError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::UpstreamError,
            "没有配置元数据来源",
        ));
    };
    match metadata.provider.check(&choice(proxy)).await {
        Ok(reached) => Ok(Json(ProxyCheckResult {
            url: reached.url.to_string(),
            status: reached.status,
            elapsed_ms: u32::try_from(reached.elapsed.as_millis()).unwrap_or(u32::MAX),
        })),
        Err(MetadataError::Unavailable(reason)) => Err(HttpError::new(
            StatusCode::BAD_GATEWAY,
            ErrorCode::UpstreamError,
            format!("连不上 DLsite：{reason}"),
        )),
        Err(MetadataError::NotFound) => Err(HttpError::new(
            StatusCode::BAD_GATEWAY,
            ErrorCode::UpstreamError,
            "连不上 DLsite",
        )),
    }
}

fn parse(body: &ServerSettings) -> Result<Option<ProxyUrl>, HttpError> {
    body.metadata_proxy
        .as_deref()
        .map(ProxyUrl::parse)
        .transpose()
        .map(Option::flatten)
        .map_err(HttpError::invalid_request)
}

fn choice(proxy: Option<ProxyUrl>) -> ProxyChoice {
    proxy.map_or(ProxyChoice::Environment, ProxyChoice::Fixed)
}
