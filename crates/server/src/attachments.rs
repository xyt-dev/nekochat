//! Local conversation-owned uploads. Public metadata never includes bytes or extracted text.
use crate::{
    accounts::{CurrentUser, require_conversation},
    api::{ApiError, blocking},
    state::AppState,
};
use axum::{
    Extension, Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use nekochat_store::Attachment;
use percent_encoding::{NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use std::io::Write;

pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_EXTRACTED_TEXT_BYTES: usize = 2 * 1024 * 1024;

fn image_media_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

fn filename(headers: &HeaderMap) -> Result<Option<String>, ApiError> {
    let Some(value) = headers.get("x-file-name") else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .map_err(|_| ApiError::bad_request("文件名必须使用 UTF-8 百分号编码"))?;
    let bytes = value.as_bytes();
    for (i, byte) in bytes.iter().enumerate() {
        if *byte == b'%'
            && (i + 2 >= bytes.len()
                || !bytes[i + 1].is_ascii_hexdigit()
                || !bytes[i + 2].is_ascii_hexdigit())
        {
            return Err(ApiError::bad_request("文件名百分号编码无效"));
        }
    }
    let decoded = percent_decode_str(value)
        .decode_utf8()
        .map_err(|_| ApiError::bad_request("文件名不是有效 UTF-8"))?;
    let name = decoded
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .trim();
    if name.is_empty()
        || matches!(name, "." | "..")
        || name.chars().any(char::is_control)
        || name.len() > 255
    {
        return Err(ApiError::bad_request("文件名无效或超过 255 字节"));
    }
    Ok(Some(name.to_owned()))
}

struct ParsedUpload {
    name: String,
    kind: &'static str,
    media_type: &'static str,
    text: Option<String>,
}

fn parse_upload(name: Option<String>, bytes: &[u8]) -> Result<ParsedUpload, ApiError> {
    if bytes.is_empty() || bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(ApiError::bad_request("文件大小必须为 1 字节至 16 MiB"));
    }
    if let Some(media_type) = image_media_type(bytes) {
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(ApiError::bad_request("图片大小不能超过 8 MiB"));
        }
        let extension = match media_type {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/webp" => "webp",
            _ => "gif",
        };
        return Ok(ParsedUpload {
            name: name.unwrap_or_else(|| format!("image.{extension}")),
            kind: "image",
            media_type,
            text: None,
        });
    }
    let name = name.unwrap_or_else(|| "document.txt".into());
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    let (media_type, text) = if bytes.starts_with(b"%PDF-") {
        ("application/pdf", extract_pdf(bytes)?)
    } else {
        if extension == "pdf" {
            return Err(ApiError::bad_request("PDF 文件损坏或格式无效"));
        }
        let media_type = match extension.as_str() {
            "md" | "markdown" | "mdown" => "text/markdown; charset=utf-8",
            "json" | "jsonl" | "ndjson" => "application/json",
            "csv" => "text/csv; charset=utf-8",
            "" | "txt" | "text" | "log" | "rst" | "adoc" | "tex" | "tsv" | "rs" | "py" | "js"
            | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "c" | "h" | "cc" | "cpp" | "hpp" | "cs"
            | "go" | "java" | "kt" | "kts" | "swift" | "rb" | "php" | "sh" | "bash" | "zsh"
            | "fish" | "ps1" | "sql" | "html" | "htm" | "xml" | "css" | "scss" | "less"
            | "yaml" | "yml" | "toml" | "ini" | "cfg" | "conf" | "vue" | "svelte" | "lua" | "r"
            | "dart" | "ex" | "exs" | "erl" | "hrl" | "clj" | "cljs" | "scala" | "pl" | "pm"
            | "bat" | "dockerfile" => "text/plain; charset=utf-8",
            _ => {
                return Err(ApiError::bad_request(
                    "不支持此文件格式；请上传 PDF、UTF-8 文本、Markdown 或代码文件",
                ));
            }
        };
        let text = std::str::from_utf8(bytes).map_err(|_| {
            ApiError::bad_request("文本文件必须使用有效 UTF-8 编码；不支持二进制文件")
        })?;
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        if text.len() > MAX_EXTRACTED_TEXT_BYTES {
            return Err(ApiError::bad_request(
                "文档提取文本不能超过 2 MiB，不会截断内容",
            ));
        }
        if text
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t' | '\u{c}'))
        {
            return Err(ApiError::bad_request(
                "不支持二进制文件或包含控制字符的文本",
            ));
        }
        (media_type, text.to_owned())
    };
    if text.trim().is_empty() {
        return Err(ApiError::bad_request(
            "文档没有可提取的文本；扫描 PDF 需要先进行 OCR",
        ));
    }
    Ok(ParsedUpload {
        name,
        kind: "document",
        media_type,
        text: Some(text),
    })
}

/// Bounds output during extraction rather than allocating a huge string and truncating it.
#[derive(Default)]
struct TextSink {
    bytes: Vec<u8>,
    exceeded: bool,
}
impl Write for TextSink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_EXTRACTED_TEXT_BYTES.saturating_sub(self.bytes.len()) {
            self.exceeded = true;
            return Err(std::io::Error::other("extracted text exceeds limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn extract_pdf(bytes: &[u8]) -> Result<String, ApiError> {
    // The upstream parser still contains unchecked assumptions for malformed PDFs. Keep those
    // failures inside this upload operation, never persist a partial extraction.
    std::panic::catch_unwind(|| {
        let doc = pdf_extract::Document::load_mem(bytes)
            .map_err(|_| ApiError::bad_request("PDF 文件损坏或格式无效"))?;
        if doc.is_encrypted() || doc.was_encrypted() {
            return Err(ApiError::bad_request("不支持加密 PDF，请先解密后再上传"));
        }
        let mut sink = TextSink::default();
        let result = {
            let mut output = pdf_extract::PlainTextOutput::new(&mut sink as &mut dyn Write);
            pdf_extract::output_doc(&doc, &mut output)
        };
        if sink.exceeded {
            return Err(ApiError::bad_request(
                "文档提取文本不能超过 2 MiB，不会截断内容",
            ));
        }
        result.map_err(|_| {
            ApiError::bad_request("无法提取 PDF 文本，文件可能损坏或使用不支持的编码")
        })?;
        String::from_utf8(sink.bytes)
            .map_err(|_| ApiError::bad_request("PDF 提取文本不是有效 UTF-8"))
    })
    .map_err(|_| ApiError::bad_request("PDF 文件损坏或无法解析"))?
}

pub async fn upload(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path(conv): Path<String>,
    headers: HeaderMap,
    bytes: Bytes,
) -> Result<Response, ApiError> {
    require_conversation(&state, &user, &conv).await?;
    let name = filename(&headers)?;
    let store = state.store.clone();
    let attachment = blocking(move || {
        // Extraction is CPU-bound and must not block the async request executor.
        let parsed = parse_upload(name, &bytes);
        match parsed {
            Ok(parsed) => store
                .insert_attachment(
                    &conv,
                    &parsed.name,
                    parsed.kind,
                    parsed.media_type,
                    &bytes,
                    parsed.text.as_deref(),
                )
                .map(Ok),
            Err(error) => Ok(Err(error)),
        }
    })
    .await??;
    Ok((
        StatusCode::CREATED,
        [
            (header::CACHE_CONTROL, "private, no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        Json(attachment),
    )
        .into_response())
}

pub async fn read(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path((conv, id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    require_conversation(&state, &user, &conv).await?;
    let store = state.store.clone();
    let (attachment, bytes) = blocking(move || store.get_attachment(&conv, &id))
        .await?
        .ok_or_else(ApiError::not_found)?;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        attachment
            .media_type
            .parse()
            .map_err(|_| ApiError::bad_request("附件内容类型无效"))?,
    );
    headers.insert(header::CACHE_CONTROL, "private, no-store".parse().unwrap());
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
    if attachment.kind == "document" {
        let encoded = utf8_percent_encode(&attachment.name, NON_ALPHANUMERIC);
        headers.insert(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"document\"; filename*=UTF-8''{encoded}")
                .parse()
                .map_err(|_| ApiError::bad_request("附件文件名无效"))?,
        );
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            "sandbox; default-src 'none'".parse().unwrap(),
        );
    }
    Ok((headers, bytes).into_response())
}

pub async fn metadata(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path((conv, id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    require_conversation(&state, &user, &conv).await?;
    let store = state.store.clone();
    let attachment: Attachment = blocking(move || {
        store
            .attachment_metadata(&conv, &[id])
            .map(|mut items| items.remove(0))
    })
    .await?;
    Ok((
        [
            (header::CACHE_CONTROL, "private, no-store"),
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
        for (bytes, expected) in [
            (b"\x89PNG\r\n\x1a\n".as_slice(), "image/png"),
            (b"\xff\xd8\xff".as_slice(), "image/jpeg"),
            (b"GIF89a".as_slice(), "image/gif"),
            (b"RIFF1234WEBP".as_slice(), "image/webp"),
        ] {
            let parsed = parse_upload(Some("disguised.txt".into()), bytes).unwrap();
            assert_eq!(parsed.kind, "image");
            assert_eq!(parsed.media_type, expected);
            assert!(parsed.text.is_none());
        }
        assert!(parse_upload(Some("active.svg".into()), b"<svg onload=alert(1)>").is_err());
        assert!(parse_upload(None, b"").is_err());
        let mut large = vec![0; MAX_IMAGE_BYTES + 1];
        large[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        assert!(parse_upload(None, &large).is_err());
    }

    #[test]
    fn utf8_filename_decode_preserves_name_but_removes_path_and_rejects_header_controls() {
        let mut headers = HeaderMap::new();
        headers.insert("x-file-name", "..%2F%E7%AC%94%E8%AE%B0.md".parse().unwrap());
        assert_eq!(filename(&headers).unwrap().as_deref(), Some("笔记.md"));
        for invalid in ["bad%0D%0A.txt", "bad%FF.txt", "bad%ZZ.txt", "..%2F"] {
            headers.insert("x-file-name", invalid.parse().unwrap());
            assert!(filename(&headers).is_err());
        }
    }

    #[test]
    fn text_limit_is_in_utf8_bytes_and_never_silently_truncates() {
        let exact = vec![b'a'; MAX_EXTRACTED_TEXT_BYTES];
        assert_eq!(
            parse_upload(Some("limit.txt".into()), &exact)
                .unwrap()
                .text
                .unwrap()
                .len(),
            MAX_EXTRACTED_TEXT_BYTES
        );
        assert!(
            parse_upload(
                Some("limit.txt".into()),
                &vec![b'a'; MAX_EXTRACTED_TEXT_BYTES + 1]
            )
            .is_err()
        );
        let mut bom = b"\xef\xbb\xbf".to_vec();
        bom.extend_from_slice(&exact);
        assert_eq!(
            parse_upload(Some("bom.txt".into()), &bom)
                .unwrap()
                .text
                .unwrap()
                .len(),
            MAX_EXTRACTED_TEXT_BYTES
        );
        let multibyte = "汉".repeat(MAX_EXTRACTED_TEXT_BYTES / 3 + 1);
        assert!(parse_upload(Some("utf8.txt".into()), multibyte.as_bytes()).is_err());
        let mut sink = TextSink::default();
        sink.write_all(&exact).unwrap();
        assert!(sink.write_all(b"x").is_err());
        assert!(sink.exceeded);
        assert_eq!(sink.bytes.len(), MAX_EXTRACTED_TEXT_BYTES);
    }
}
