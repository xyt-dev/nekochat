//! Provider-protocol layer: streams one completion from an upstream provider.

mod chat_completions;
mod chatgpt;
mod http;
mod responses;
mod tools;

pub use chat_completions::{build_body, list_models, parse_usage, stream_chat};
pub use chatgpt::stream_chatgpt;
pub use responses::stream_responses;
pub use tools::{MAX_ARGUMENT_BYTES, ToolAccumulator, ToolCall, ToolCallDelta, ToolContext, ToolDefinition, ToolExchange, ToolResult};

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
    pub images: Vec<ChatImage>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatImage {
    pub media_type: String,
    pub base64: String,
}

impl ChatImage {
    pub fn data_url(&self) -> String {
        format!("data:{};base64,{}", self.media_type, self.base64)
    }
}

#[derive(Clone, Debug)]
pub struct ChatRequest<'a> {
    pub model: &'a str,
    pub messages: &'a [ChatMessage],
    pub reasoning: ReasoningStyle,
    pub effort: Option<&'a str>,
    pub session_id: &'a str,
    pub parameters: &'a GenerationParameters,
    pub tools: Option<&'a ToolContext>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StreamEvent {
    Reasoning(String),
    Text(String),
    Usage(Usage),
    ToolCallDelta(ToolCallDelta),
    /// Complete native output items, including encrypted reasoning for stateless continuation.
    ResponseItem(serde_json::Value),
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
