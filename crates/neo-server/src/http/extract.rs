use axum::{
    extract::{
        FromRequest, FromRequestParts,
        rejection::{JsonRejection, PathRejection, QueryRejection},
    },
    http::{
        StatusCode,
        header::{AUTHORIZATION, COOKIE},
        request::Parts,
    },
};
use neo_db::users::{self, UserRecord};

use super::{AppState, HttpError};
use crate::token::{Audience, Verified};

/// `axum::Json` with contract-shaped 400 errors instead of plain text.
#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(HttpError))]
pub struct ApiJson<T>(pub T);

impl From<JsonRejection> for HttpError {
    fn from(rejection: JsonRejection) -> Self {
        Self::invalid_request(format!("请求体格式不对：{}", rejection.body_text()))
    }
}

/// `axum::extract::Query` with contract-shaped 400 errors.
#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Query), rejection(HttpError))]
pub struct ApiQuery<T>(pub T);

impl From<QueryRejection> for HttpError {
    fn from(rejection: QueryRejection) -> Self {
        Self::invalid_request(format!("参数格式不对：{}", rejection.body_text()))
    }
}

/// `axum::extract::Path` with contract-shaped 400 errors.
#[derive(FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(HttpError))]
pub struct ApiPath<T>(pub T);

impl From<PathRejection> for HttpError {
    fn from(rejection: PathRejection) -> Self {
        Self::invalid_request(format!("路径参数不对：{}", rejection.body_text()))
    }
}

/// The user behind a valid API token. Each request reads the user row, so a
/// deleted user or a bumped `token_version` takes effect at once. A media
/// token is never accepted here.
pub struct CurrentUser(pub UserRecord);

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = HttpError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, HttpError> {
        let token = bearer(parts).ok_or_else(HttpError::unauthorized)?;
        let claims = state
            .tokens()
            .verify(token, Audience::Api)
            .ok_or_else(HttpError::unauthorized)?;
        Ok(Self(current_user(state, claims).await?))
    }
}

async fn current_user(state: &AppState, claims: Verified) -> Result<UserRecord, HttpError> {
    users::find_by_id(state.db(), claims.user_id)
        .await?
        .filter(|u| u.token_version == claims.token_version)
        .ok_or_else(HttpError::unauthorized)
}

fn bearer(parts: &Parts) -> Option<&str> {
    parts
        .headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
}

/// An administrator behind a valid API token. Anyone else gets 403.
// Handlers that only need the check ignore the user; others read it.
#[allow(dead_code)]
pub struct AdminUser(pub UserRecord);

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = HttpError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, HttpError> {
        let CurrentUser(user) = CurrentUser::from_request_parts(parts, state).await?;
        if !user.is_admin {
            return Err(HttpError::forbidden());
        }
        Ok(Self(user))
    }
}

/// The user behind a media request: an API token in the Authorization
/// header, or a media token in `?token=` for elements such as `<img>` and
/// `<audio>` that cannot send headers. A media token in the header is not
/// accepted, and neither is an API token in the URL, where logs and
/// browser history would keep it.
// Handlers that only need the check ignore the user; others read it.
#[allow(dead_code)]
pub struct MediaUser(pub UserRecord);

#[derive(serde::Deserialize)]
pub struct TokenParam {
    pub token: Option<String>,
}

/// Whether the request carries an `Authorization: Bearer` header, which
/// [`MediaUser`] then prefers over a `token` in the query.
pub fn bearer_present(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("Bearer "))
}

impl FromRequestParts<AppState> for MediaUser {
    type Rejection = HttpError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, HttpError> {
        let claims = if let Some(token) = bearer(parts) {
            state.tokens().verify(token, Audience::Api)
        } else {
            query_token(parts).and_then(|t| state.tokens().verify(&t, Audience::Media))
        };
        let claims = claims.ok_or_else(HttpError::unauthorized)?;
        Ok(Self(current_user(state, claims).await?))
    }
}

fn query_token(parts: &Parts) -> Option<String> {
    axum::extract::Query::<TokenParam>::try_from_uri(&parts.uri)
        .ok()
        .and_then(|q| q.0.token)
}

/// The cookies the frontend keeps its media token in for covers: one for
/// HTTPS and one for plain HTTP, because a `Secure` cookie written over
/// HTTPS cannot be replaced from HTTP on the same host. The HTTPS one is
/// tried first.
pub const COVER_COOKIES: [&str; 2] = ["__Secure-kikoeru-media", "kikoeru-media"];

/// [`MediaUser`] for covers, which also takes the media token from the
/// [`COVER_COOKIES`]. An `<img>` URL with the token in it changes
/// whenever the token is renewed, every six hours, and the browser then
/// downloads every cover it had cached again. The cookie is `SameSite=Strict`
/// and limited to `/api/works`, so another site cannot load covers with it,
/// and only this extractor reads it, so it opens nothing else.
#[allow(dead_code)]
pub struct CoverUser(pub UserRecord);

impl FromRequestParts<AppState> for CoverUser {
    type Rejection = HttpError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, HttpError> {
        let claims = if let Some(token) = bearer(parts) {
            state.tokens().verify(token, Audience::Api)
        } else if let Some(token) = query_token(parts) {
            state.tokens().verify(&token, Audience::Media)
        } else {
            // A stale cookie from the other scheme may sit next to a fresh
            // one. A revoked token still has a valid signature, so each is
            // checked against the user table before the next is tried.
            let candidates: Vec<&str> = COVER_COOKIES
                .iter()
                .flat_map(|name| cookies(parts, name))
                .collect();
            for token in candidates {
                if let Some(claims) = state.tokens().verify(token, Audience::Media) {
                    match current_user(state, claims).await {
                        Ok(user) => return Ok(Self(user)),
                        Err(e) if e.status == StatusCode::UNAUTHORIZED => {}
                        Err(e) => return Err(e),
                    }
                }
            }
            None
        };
        let claims = claims.ok_or_else(HttpError::unauthorized)?;
        Ok(Self(current_user(state, claims).await?))
    }
}

/// The values of the cookies called `name`, from every `Cookie` header.
fn cookies<'a>(parts: &'a Parts, name: &'a str) -> impl Iterator<Item = &'a str> {
    parts
        .headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .filter(move |(key, _)| *key == name)
        .map(|(_, value)| value)
}
