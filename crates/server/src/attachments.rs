//! Local conversation-scoped image upload and retrieval; bytes never leave through public JSON.
use crate::{
    api::{ApiError, blocking},
    state::AppState,
};
use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use nekochat_store::Attachment;

pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

fn media_type(bytes: &[u8]) -> Result<&'static str, ApiError> {
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return Err(ApiError::bad_request("图片大小必须为 1 字节至 8 MiB"));
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Ok("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Ok("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Ok("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Ok("image/webp")
    } else {
        Err(ApiError::bad_request(
            "只支持 PNG、JPEG、WebP、GIF 图片（按文件内容识别）",
        ))
    }
}

pub async fn upload(
    State(state): State<AppState>,
    Path(conv): Path<String>,
    bytes: Bytes,
) -> Result<Response, ApiError> {
    let media_type = media_type(&bytes)?;
    let store = state.store.clone();
    let attachment = blocking(move || store.insert_attachment(&conv, media_type, &bytes)).await?;
    Ok((
        StatusCode::CREATED,
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        Json(attachment),
    )
        .into_response())
}

pub async fn read(
    State(state): State<AppState>,
    Path((conv, id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let store = state.store.clone();
    let (attachment, bytes) = blocking(move || store.get_attachment(&conv, &id))
        .await?
        .ok_or_else(ApiError::not_found)?;
    Ok((
        [
            (header::CONTENT_TYPE, attachment.media_type),
            (header::CACHE_CONTROL, "no-store".into()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".into()),
        ],
        bytes,
    )
        .into_response())
}

pub async fn metadata(
    State(state): State<AppState>,
    Path((conv, id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let store = state.store.clone();
    let attachment: Attachment = blocking(move || {
        store
            .attachment_metadata(&conv, &[id])
            .map(|mut items| items.remove(0))
    })
    .await?;
    Ok((
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        Json(attachment),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn magic_not_extension_or_client_type_controls_image_format() {
        assert_eq!(media_type(b"\x89PNG\r\n\x1a\n").unwrap(), "image/png");
        assert_eq!(media_type(b"\xff\xd8\xff").unwrap(), "image/jpeg");
        assert_eq!(media_type(b"GIF89a").unwrap(), "image/gif");
        assert_eq!(media_type(b"RIFF1234WEBP").unwrap(), "image/webp");
        assert!(media_type(b"<svg onload=alert(1)>").is_err());
        assert!(media_type(b"").is_err());
        let mut large = vec![0; MAX_IMAGE_BYTES + 1];
        large[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        assert!(media_type(&large).is_err());
    }
}
