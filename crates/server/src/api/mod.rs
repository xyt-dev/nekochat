//! JSON API: error type and router.

pub mod admin;
pub mod user;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use nekochat_store::StoreError;
use serde_json::json;

/// An error response with body `{"error": "<中文说明>"}`.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: &'static str,
}

impl ApiError {
    pub fn new(status: StatusCode, message: &'static str) -> ApiError {
        ApiError { status, message }
    }

    pub fn not_found() -> ApiError {
        ApiError::new(StatusCode::NOT_FOUND, "未找到")
    }

    pub fn unauthorized() -> ApiError {
        ApiError::new(StatusCode::UNAUTHORIZED, "未登录")
    }

    pub fn bad_request(message: &'static str) -> ApiError {
        ApiError::new(StatusCode::BAD_REQUEST, message)
    }

    pub fn internal() -> ApiError {
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "服务器内部错误")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

impl From<StoreError> for ApiError {
    fn from(e: StoreError) -> ApiError {
        match e {
            StoreError::NotFound => ApiError::not_found(),
            StoreError::Conflict(_) => ApiError::new(StatusCode::CONFLICT, "与已有记录冲突"),
            StoreError::Sqlite(_) | StoreError::Json(_) => {
                tracing::error!(error = %e, "store error");
                ApiError::internal()
            }
        }
    }
}

/// Runs a blocking store call off the async runtime.
pub async fn blocking<T, F>(f: F) -> Result<T, ApiError>
where
    F: FnOnce() -> Result<T, StoreError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| ApiError::internal())?
        .map_err(ApiError::from)
}
