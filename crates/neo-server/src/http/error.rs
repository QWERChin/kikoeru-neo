use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use neo_core::dto::{ApiError, ErrorCode, ErrorResponse};

/// An error response in the shape fixed by book/src/contract.md.
#[derive(Debug)]
pub struct HttpError {
    pub status: StatusCode,
    pub code: ErrorCode,
    pub message: String,
}

impl HttpError {
    pub fn new(status: StatusCode, code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, ErrorCode::InvalidRequest, message)
    }

    pub fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::Unauthorized,
            "登录已失效，请重新登录",
        )
    }

    pub fn invalid_credentials() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            ErrorCode::InvalidCredentials,
            "用户名或密码不对",
        )
    }

    pub fn not_found() -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            ErrorCode::NotFound,
            "没有这个接口或资源",
        )
    }

    pub fn forbidden() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            ErrorCode::Forbidden,
            "需要管理员权限",
        )
    }

    pub fn work_not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, ErrorCode::NotFound, "没有这个作品")
    }

    pub fn scan_running() -> Self {
        Self::new(
            StatusCode::CONFLICT,
            ErrorCode::ScanRunning,
            "正在扫描媒体库，扫描结束后再试",
        )
    }

    pub fn not_ready() -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::NotReady,
            "服务还在启动，请稍后再试",
        )
    }

    /// Logs the cause and returns a response that does not reveal it.
    pub fn internal(cause: impl std::fmt::Display) -> Self {
        tracing::error!(%cause, "request failed");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::InternalError,
            "服务内部错误",
        )
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let body = ErrorResponse {
            error: ApiError {
                code: self.code,
                message: self.message,
            },
        };
        (self.status, Json(body)).into_response()
    }
}

impl From<sqlx::Error> for HttpError {
    fn from(e: sqlx::Error) -> Self {
        Self::internal(e)
    }
}
