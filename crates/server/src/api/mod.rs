//! JSON API: error type and router.

pub mod admin;
pub mod monitoring;
pub mod user;

use std::borrow::Cow;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Request};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use nekochat_store::StoreError;
use serde_json::json;

/// An error response with body `{"error": "<中文说明>"}`.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: Cow<'static, str>,
}

impl ApiError {
    pub fn new(status: StatusCode, message: &'static str) -> ApiError {
        ApiError {
            status,
            message: Cow::Borrowed(message),
        }
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

impl From<JsonRejection> for ApiError {
    fn from(r: JsonRejection) -> ApiError {
        ApiError {
            status: StatusCode::BAD_REQUEST,
            message: Cow::Owned(format!("请求格式错误：{}", r.body_text())),
        }
    }
}

/// A JSON request body whose rejections are `ApiError`s (400 with a JSON body).
pub struct JsonBody<T>(pub T);

impl<T, S> FromRequest<S> for JsonBody<T>
where
    Json<T>: FromRequest<S, Rejection = JsonRejection>,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, ApiError> {
        let Json(value) = Json::<T>::from_request(req, state).await?;
        Ok(JsonBody(value))
    }
}

/// Fallback for a known path with an unsupported method.
pub async fn wrong_method() -> ApiError {
    ApiError::new(StatusCode::METHOD_NOT_ALLOWED, "不支持的请求方法")
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
            StoreError::Sqlite(_)
            | StoreError::Json(_)
            | StoreError::CheckpointBusy
            | StoreError::UnsupportedSchema { .. } => {
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
