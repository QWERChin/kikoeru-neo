use std::{
    net::{IpAddr, SocketAddr},
    time::Instant,
};

use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
};
use neo_core::dto::{
    ErrorCode, LoginRequest, LoginResponse, MediaTokenResponse, PasswordRequest, SafeInteger, User,
};
use neo_db::users::{self, UserRecord};

use super::{
    AppState, HttpError,
    extract::{ApiJson, CurrentUser},
    limiter::{self, Reservation, Verdict},
};
use crate::{
    password,
    token::{self, Audience, Issued},
};

fn user_dto(user: &UserRecord) -> Result<User, HttpError> {
    Ok(User {
        id: user.id.try_into().map_err(HttpError::internal)?,
        name: user.name.clone(),
        is_admin: user.is_admin,
    })
}

fn issue(state: &AppState, user: &UserRecord, aud: Audience) -> Result<Issued, HttpError> {
    state
        .tokens()
        .issue(user.id, user.token_version, aud, token::now_secs())
        .map_err(HttpError::internal)
}

fn expires_at(issued: &Issued) -> Result<SafeInteger, HttpError> {
    issued.expires_at_ms.try_into().map_err(HttpError::internal)
}

async fn verify_password(stored: Option<String>, password: String) -> Result<bool, HttpError> {
    tokio::task::spawn_blocking(move || password::verify(stored.as_deref(), &password))
        .await
        .map_err(HttpError::internal)
}

/// The address failed attempts are counted against. `X-Forwarded-For` is
/// read only when the peer is a configured proxy, and only its last entry,
/// which is the one that proxy appended. Earlier entries come from the
/// client and could be anything. A listener on `[::]` sees an IPv4 proxy
/// as `::ffff:a.b.c.d`, so both sides compare in canonical form.
fn client_ip(state: &AppState, peer: SocketAddr, headers: &HeaderMap) -> IpAddr {
    let peer = peer.ip().to_canonical();
    if !state
        .trusted_proxies()
        .iter()
        .any(|p| p.to_canonical() == peer)
    {
        return peer;
    }
    headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(str::trim)
        .rfind(|v| !v.is_empty())
        .and_then(|v| v.parse().ok())
        .unwrap_or(peer)
}

/// Counts the attempt before the password check, then refuses blocked
/// sources or slows down guesses at a name that fails a lot. The caller
/// refunds the reservation only when the password was right.
async fn throttle(state: &AppState, ip: IpAddr, user: &str) -> Result<Reservation, HttpError> {
    match state.limiter().reserve(ip, user, Instant::now()) {
        (Verdict::Allow, Some(reservation)) => Ok(reservation),
        (Verdict::Delay, Some(reservation)) => {
            tokio::time::sleep(limiter::DELAY).await;
            Ok(reservation)
        }
        _ => {
            tracing::warn!(
                name = ?logged_name(user),
                %ip,
                "too many failed password checks, refused"
            );
            Err(HttpError::new(
                StatusCode::TOO_MANY_REQUESTS,
                ErrorCode::TooManyAttempts,
                "密码错误次数太多，请一分钟后再试",
            ))
        }
    }
}

/// A login name as the log shows it, logged with `?` so that a line break
/// in it cannot forge a log line. It comes from the request, so it is cut
/// short.
fn logged_name(name: &str) -> String {
    name.chars().take(64).collect()
}

pub async fn login(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ApiJson(req): ApiJson<LoginRequest>,
) -> Result<Json<LoginResponse>, HttpError> {
    let db = state.db();
    // Checked before reserving: no password is verified here, so it is not
    // a failed attempt.
    if !users::admin_exists(db).await? {
        return Err(HttpError::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::SetupRequired,
            "还没有管理员账号，请在服务器上运行 kikoeru-neo user create --admin <名字>",
        ));
    }
    let ip = client_ip(&state, peer, &headers);
    let reservation = throttle(&state, ip, &req.name).await?;
    let user = users::find_by_name(db, &req.name).await?;
    let stored = user.as_ref().and_then(|u| u.password_hash.clone());
    let ok = verify_password(stored, req.password).await?;
    // Unknown names and accounts without a password count like a wrong
    // password, so probing names is limited the same way.
    let Some(user) = user.filter(|_| ok) else {
        tracing::warn!(name = ?logged_name(&req.name), %ip, "login failed");
        return Err(HttpError::invalid_credentials());
    };
    state.limiter().refund(reservation, Instant::now());
    tracing::info!(name = ?logged_name(&user.name), %ip, "logged in");
    let issued = issue(&state, &user, Audience::Api)?;
    Ok(Json(LoginResponse {
        expires_at: expires_at(&issued)?,
        token: issued.token,
        user: user_dto(&user)?,
    }))
}

pub async fn me(CurrentUser(user): CurrentUser) -> Result<Json<User>, HttpError> {
    Ok(Json(user_dto(&user)?))
}

pub async fn media_token(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<Json<MediaTokenResponse>, HttpError> {
    let issued = issue(&state, &user, Audience::Media)?;
    Ok(Json(MediaTokenResponse {
        expires_at: expires_at(&issued)?,
        token: issued.token,
    }))
}

/// `POST /api/auth/logout-all`: revokes every token of the caller, the one
/// this request carries too.
pub async fn logout_all(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
) -> Result<StatusCode, HttpError> {
    if !users::revoke_tokens(state.db(), user.id).await? {
        return Err(HttpError::unauthorized());
    }
    Ok(StatusCode::NO_CONTENT)
}

/// A stolen API token must not become a way to guess the current password
/// without limit, so this shares the login limiter.
pub async fn change_password(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    CurrentUser(user): CurrentUser,
    ApiJson(req): ApiJson<PasswordRequest>,
) -> Result<StatusCode, HttpError> {
    let ip = client_ip(&state, peer, &headers);
    if !password::is_long_enough(&req.new_password) {
        return Err(HttpError::invalid_request(format!(
            "新密码至少要 {} 个字符",
            password::MIN_LENGTH
        )));
    }
    let reservation = throttle(&state, ip, &user.name).await?;
    if !verify_password(user.password_hash.clone(), req.current_password).await? {
        tracing::warn!(
            name = ?logged_name(&user.name),
            %ip,
            "wrong current password when changing it"
        );
        return Err(HttpError::invalid_credentials());
    }
    state.limiter().refund(reservation, Instant::now());
    let new_password = req.new_password;
    let hash = tokio::task::spawn_blocking(move || password::hash(&new_password))
        .await
        .map_err(HttpError::internal)?
        .map_err(HttpError::internal)?;
    // The version check makes a reset that landed while this request was in
    // flight win, and revokes the token this request was made with.
    if !users::set_password_if_version(state.db(), user.id, user.token_version, &hash).await? {
        return Err(HttpError::unauthorized());
    }
    Ok(StatusCode::NO_CONTENT)
}
