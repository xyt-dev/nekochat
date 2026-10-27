//! Transport details shared by the native model protocols.
use serde_json::Value;

use crate::{ChatRole, GatewayError};

pub(crate) fn endpoint(base_url: &str, path: &str) -> String {
    let base = base_url.strip_suffix('/').unwrap_or(base_url);
    format!("{base}{path}")
}

pub(crate) fn role_name(role: ChatRole) -> &'static str {
    match role {
        ChatRole::System => "system",
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
    }
}

pub(crate) fn transport(error: reqwest::Error) -> GatewayError {
    GatewayError::Transport(error.to_string())
}

pub(crate) async fn check_status(
    response: reqwest::Response,
) -> Result<reqwest::Response, GatewayError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let text = response.text().await.unwrap_or_default();
    let message = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|value| value.pointer("/error/message")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| text.chars().take(500).collect());
    Err(GatewayError::Http {
        status: status.as_u16(),
        message,
    })
}
