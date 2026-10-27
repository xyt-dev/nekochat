//! Local-only resource and background image delivery. Never fetches a URL.
use crate::{api::ApiError, state::AppState};
use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};

pub const MAX_BACKGROUND: usize = 5 * 1024 * 1024;

fn component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

pub fn resource_url(pack: &str, asset: &str) -> Option<String> {
    (component(pack) && component(asset)).then(|| format!("/api/resources/{pack}/{asset}"))
}

pub async fn packs(State(state): State<AppState>) -> Json<Vec<Value>> {
    Json(state.resources.packs().into_iter().map(|pack| {
        let mut value = serde_json::to_value(&pack).expect("serializable pack");
        value["assets"] = json!(pack.assets.into_iter().filter_map(|asset| {
            Some(json!({"id": asset.id, "name": asset.name, "url": resource_url(&pack.id, &asset.id)?}))
        }).collect::<Vec<_>>());
        value
    }).collect())
}

fn image_response(bytes: Bytes, mime: &str) -> Response {
    (
        [
            ("content-type", mime),
            ("x-content-type-options", "nosniff"),
            ("cache-control", "private, max-age=86400"),
        ],
        bytes,
    )
        .into_response()
}

pub async fn resource(
    State(state): State<AppState>,
    Path((pack, id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    resource_url(&pack, &id).ok_or_else(ApiError::not_found)?;
    let asset = state
        .resources
        .asset(&pack, &id)
        .ok_or_else(ApiError::not_found)?;
    if signature(&asset.bytes).is_none_or(|(_, mime)| mime != asset.mime) {
        return Err(ApiError::not_found());
    }
    Ok(image_response(asset.bytes, asset.mime))
}

fn signature(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(("png", "image/png"))
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some(("jpg", "image/jpeg"))
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(("gif", "image/gif"))
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some(("webp", "image/webp"))
    } else {
        None
    }
}

fn background_file(file: &str) -> bool {
    let Some((id, ext)) = file.rsplit_once('.') else {
        return false;
    };
    component(file)
        && uuid::Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == id)
        && ["png", "jpg", "webp", "gif"].contains(&ext)
}

async fn read_background(state: &AppState, file: &str) -> Option<Vec<u8>> {
    if !background_file(file) {
        return None;
    }
    let root = state.config.data_dir.clone();
    let file = file.to_owned();
    tokio::task::spawn_blocking(move || {
        use rustix::fs::{Mode, OFlags, openat};
        use std::{fs::File, io::Read};
        let root = File::open(root).ok()?;
        let directory = openat(
            &root,
            "backgrounds",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .ok()?;
        let source = File::from(
            openat(
                &directory,
                file.as_str(),
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .ok()?,
        );
        let metadata = source.metadata().ok()?;
        if !metadata.is_file() || metadata.len() > MAX_BACKGROUND as u64 {
            return None;
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        source
            .take(MAX_BACKGROUND as u64 + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() > MAX_BACKGROUND {
            return None;
        }
        let (ext, _) = signature(&bytes)?;
        file.ends_with(&format!(".{ext}")).then_some(bytes)
    })
    .await
    .ok()
    .flatten()
}

pub async fn background_url_allowed(state: &AppState, url: &str) -> bool {
    if let Some(file) = url.strip_prefix("/api/backgrounds/") {
        return read_background(state, file).await.is_some();
    }
    if let Some(path) = url.strip_prefix("/api/resources/") {
        if let Some((pack, asset)) = path.split_once('/') {
            return resource_url(pack, asset).as_deref() == Some(url)
                && state.resources.asset(pack, asset).is_some();
        }
    }
    false
}

pub async fn background(
    State(state): State<AppState>,
    Path(file): Path<String>,
) -> Result<Response, ApiError> {
    let bytes = read_background(&state, &file)
        .await
        .ok_or_else(ApiError::not_found)?;
    let (_, mime) = signature(&bytes).ok_or_else(ApiError::not_found)?;
    Ok(image_response(Bytes::from(bytes), mime))
}

pub async fn upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    bytes: Bytes,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    if bytes.len() > MAX_BACKGROUND {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "背景图片不能超过5MiB",
        ));
    }
    let (ext, mime) =
        signature(&bytes).ok_or_else(|| ApiError::bad_request("仅支持 PNG/JPEG/WebP/GIF 图片"))?;
    if headers.get("content-type").and_then(|h| h.to_str().ok()) != Some(mime) {
        return Err(ApiError::bad_request("图片签名与类型不匹配"));
    }
    let root = state.config.data_dir.clone();
    let file = format!("{}.{ext}", uuid::Uuid::new_v4());
    let output_name = file.clone();
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        use rustix::fs::{Mode, OFlags, openat};
        use std::{fs::File, io::Write};
        std::fs::create_dir_all(root.join("backgrounds"))?;
        let root = File::open(root)?;
        let directory = openat(
            &root,
            "backgrounds",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        let mut output = File::from(openat(
            &directory,
            output_name.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )?);
        output.write_all(&bytes)?;
        Ok(())
    })
    .await
    .map_err(|_| ApiError::internal())?
    .map_err(|_| ApiError::internal())?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"url":format!("/api/backgrounds/{file}")})),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_safe_manifest_components_and_image_signatures() {
        for value in ["..", "../file", "a/b", "a%2fb", "a?b", "a#b", "//host"] {
            assert!(resource_url(value, "a.gif").is_none());
        }
        assert!(resource_url("nekowhale", "abc.gif").is_some());
        assert!(signature(b"<svg></svg>").is_none());
        assert!(signature(b"<!doctype html>").is_none());
        assert!(!background_file("../../../etc/passwd"));
    }
}
