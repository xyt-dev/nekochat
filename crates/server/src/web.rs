//! The web frontend: embedded `web/dist` with SPA fallback, and custom themes from
//! `<data_dir>/themes`.

use std::borrow::Cow;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::api::{ApiError, wrong_method};
use crate::state::AppState;

/// Where the built frontend comes from.
pub trait WebAssets: Send + Sync + 'static {
    /// The file at `path` (relative, no leading slash), if any.
    fn get(&self, path: &str) -> Option<Cow<'static, [u8]>>;
}

/// `web/dist` embedded at compile time (read from disk in debug builds); may be empty.
#[derive(rust_embed::RustEmbed)]
#[folder = "../../web/dist"]
#[allow_missing = true]
pub struct EmbeddedDist;

impl WebAssets for EmbeddedDist {
    fn get(&self, path: &str) -> Option<Cow<'static, [u8]>> {
        <EmbeddedDist as rust_embed::RustEmbed>::get(path).map(|f| f.data)
    }
}

const INDEX: &str = "index.html";
const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const NO_CACHE: &str = "no-cache";

/// Fallback for every unrouted request: JSON 404 under `/api` and `/themes`, otherwise the
/// requested file or, failing that, `index.html`.
pub async fn fallback(assets: Arc<dyn WebAssets>, req: Request) -> Response {
    let path = req.uri().path();
    if path == "/api" || path.starts_with("/api/") || path.starts_with("/themes/") {
        return ApiError::new(StatusCode::NOT_FOUND, "接口不存在").into_response();
    }
    if req.method() != Method::GET && req.method() != Method::HEAD {
        return wrong_method().await.into_response();
    }
    let rel = path.trim_start_matches('/');
    let safe = !rel.is_empty()
        && !rel.ends_with('/')
        && !rel.contains('\\')
        && rel
            .split('/')
            .all(|seg| !seg.is_empty() && seg != "." && seg != "..");
    if safe && let Some(data) = assets.get(rel) {
        let cache = if rel.starts_with("assets/") {
            IMMUTABLE
        } else {
            NO_CACHE
        };
        return file_response(rel, data, cache);
    }
    match assets.get(INDEX) {
        Some(data) => file_response(INDEX, data, NO_CACHE),
        None => ApiError::new(StatusCode::NOT_FOUND, "前端未构建：先运行 web 构建").into_response(),
    }
}

fn file_response(path: &str, data: Cow<'static, [u8]>, cache: &'static str) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let mut res = data.into_owned().into_response();
    let headers = res.headers_mut();
    if let Ok(v) = HeaderValue::from_str(mime.as_ref()) {
        headers.insert(header::CONTENT_TYPE, v);
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    res
}

/// A custom theme file `<data_dir>/themes/<id>.css`.
#[derive(Serialize)]
pub struct ThemeView {
    id: String,
    name: String,
    url: String,
}

/// `<id>.css` with `id` matching `[A-Za-z0-9_-]+`.
fn theme_id(file: &str) -> Option<&str> {
    let id = file.strip_suffix(".css")?;
    (!id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
    .then_some(id)
}

/// `GET /api/themes`: the custom themes, sorted by name.
pub async fn list_themes(State(state): State<AppState>) -> Result<Json<Vec<ThemeView>>, ApiError> {
    let dir = state.config.data_dir.join("themes");
    let mut entries = match tokio::fs::read_dir(&dir).await {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Json(Vec::new())),
        Err(e) => {
            tracing::error!(error = %e, "reading themes dir failed");
            return Err(ApiError::internal());
        }
    };
    let mut themes = Vec::new();
    loop {
        let entry = match entries.next_entry().await {
            Ok(Some(entry)) => entry,
            Ok(None) => break,
            Err(e) => {
                tracing::error!(error = %e, "reading themes dir failed");
                return Err(ApiError::internal());
            }
        };
        let is_file = entry.file_type().await.is_ok_and(|t| t.is_file());
        let file = entry.file_name();
        if let (true, Some(id)) = (is_file, file.to_str().and_then(theme_id)) {
            themes.push(ThemeView {
                id: id.to_string(),
                name: id.to_string(),
                url: format!("/themes/{id}.css"),
            });
        }
    }
    themes.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Json(themes))
}

/// `GET /themes/{file}`: one custom theme stylesheet.
pub async fn theme_file(State(state): State<AppState>, Path(file): Path<String>) -> Response {
    if theme_id(&file).is_none() {
        return ApiError::not_found().into_response();
    }
    match tokio::fs::read(state.config.data_dir.join("themes").join(&file)).await {
        Ok(css) => (
            [
                (header::CONTENT_TYPE, "text/css; charset=utf-8"),
                (header::CACHE_CONTROL, NO_CACHE),
            ],
            css,
        )
            .into_response(),
        Err(_) => ApiError::not_found().into_response(),
    }
}
