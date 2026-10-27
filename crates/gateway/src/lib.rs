//! Provider-protocol layer: streams one completion from an upstream provider.

mod chat_completions;

pub use chat_completions::{build_body, list_models, parse_usage, stream_chat};

use nekochat_core::{GenerationParameters, ReasoningStyle, Usage};

pub const USER_AGENT: &str = concat!("nekochat/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Debug)]
pub struct Provider {
    pub key: String,
    pub label: String,
    pub base_url: String,
    pub models_url: Option<String>,
    pub api_key: String,
    pub session_header: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatRole {
    System,
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

#[derive(Clone, Debug)]
pub struct ChatRequest<'a> {
    pub model: &'a str,
    pub messages: &'a [ChatMessage],
    pub reasoning: ReasoningStyle,
    pub effort: Option<&'a str>,
    pub session_id: &'a str,
    pub parameters: &'a GenerationParameters,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StreamEvent {
    Reasoning(String),
    Text(String),
    Usage(Usage),
}

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("upstream HTTP {status}: {message}")]
    Http { status: u16, message: String },
    #[error("transport error: {0}")]
    Transport(String),
    #[error("protocol error: {0}")]
    Protocol(String),
}
