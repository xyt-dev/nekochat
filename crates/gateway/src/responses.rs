//! Stateless native OpenAI Responses, including OpenCode Go's usage extensions.
use std::collections::{HashMap, VecDeque};

use eventsource_stream::Eventsource;
use futures_util::{
    StreamExt,
    stream::{self, BoxStream},
};
use nekochat_core::{Protocol, ReasoningStyle, Usage};
use serde_json::{Value, json};

use crate::http::{check_status, endpoint, role_name, transport};
use crate::{ChatRequest, GatewayError, Provider, StreamEvent, USER_AGENT};

pub(crate) fn input_messages(messages: &[crate::ChatMessage]) -> Vec<Value> {
    messages
        .iter()
        .map(|message| {
            let content = if message.images.is_empty() {
                json!(message.content)
            } else {
                let mut parts = Vec::with_capacity(message.images.len() + 1);
                if !message.content.is_empty() {
                    parts.push(json!({"type":"input_text","text":message.content}));
                }
                parts.extend(
                    message
                        .images
                        .iter()
                        .map(|image| json!({"type":"input_image","image_url":image.data_url()})),
                );
                json!(parts)
            };
            let role = if message.role == crate::ChatRole::System {
                "developer"
            } else {
                role_name(message.role)
            };
            json!({"role":role,"content":content})
        })
        .collect()
}

fn body(request: &ChatRequest<'_>) -> Result<Value, GatewayError> {
    request
        .parameters
        .validate_for_protocol(&Protocol::Responses)
        .map_err(|message| GatewayError::Protocol(message.into()))?;
    if request.reasoning == ReasoningStyle::DeepSeek {
        return Err(GatewayError::Protocol(
            "Responses 不支持 DeepSeek thinking 模式".into(),
        ));
    }
    let mut input = input_messages(request.messages);
    if let Some(tools) = request.tools {
        tools.extend_responses(&mut input);
    }
    let mut body = json!({"model":request.model,"input":input,"stream":true,"store":false});
    if let Some(tools) = request.tools {
        body["include"] = json!(["reasoning.encrypted_content"]);
        if !tools.definitions.is_empty() {
            body["tools"] = tools.response_definitions(false);
        }
    }
    if let Some(effort) = request.effort {
        body["reasoning"] = json!({"effort":effort,"summary":"auto"});
    }
    let parameters = request.parameters;
    for (key, value) in [
        (
            "temperature",
            parameters.temperature.map(|value| json!(value)),
        ),
        ("top_p", parameters.top_p.map(|value| json!(value))),
        (
            "max_output_tokens",
            parameters.max_output_tokens.map(|value| json!(value)),
        ),
    ] {
        if let Some(value) = value {
            body[key] = value;
        }
    }
    Ok(body)
}

pub(crate) fn usage(value: &Value) -> Usage {
    if value.pointer("/opencode/tokens").is_some() {
        return crate::parse_usage(value);
    }
    let count = |path: &str| value.pointer(path).and_then(Value::as_u64).unwrap_or(0);
    let money = |path: &str| {
        value
            .pointer(path)
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite() && *value >= 0.0)
    };
    let cached_in = count("/input_tokens_details/cached_tokens");
    let reasoning = count("/output_tokens_details/reasoning_tokens");
    Usage {
        cached_in,
        uncached_in: count("/input_tokens").saturating_sub(cached_in),
        reasoning,
        output: count("/output_tokens").saturating_sub(reasoning),
        upstream_cost_usd: money("/cost"),
        allowance_usd: money("/opencode/allowance"),
    }
}

fn terminal_message(value: &Value, incomplete: bool) -> String {
    value
        .pointer("/response/error/message")
        .or_else(|| value.pointer("/error/message"))
        .or_else(|| value.get("message"))
        .and_then(Value::as_str)
        .map(|message| message.chars().take(500).collect())
        .unwrap_or_else(|| {
            if incomplete {
                let reason = value
                    .pointer("/response/incomplete_details/reason")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                format!("Responses 回复未完成：{reason}")
            } else {
                "Responses 上游返回失败".into()
            }
        })
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
enum TextKind {
    Text,
    Refusal,
    Reasoning,
    Summary,
}

impl TextKind {
    fn event(self, text: String) -> StreamEvent {
        match self {
            Self::Text | Self::Refusal => StreamEvent::Text(text),
            Self::Reasoning | Self::Summary => StreamEvent::Reasoning(text),
        }
    }
}

#[derive(Default)]
struct ItemOutput {
    id: Option<String>,
    index: Option<u64>,
    // Responses snapshots are cumulative. Byte offsets avoid retaining/copying every delta.
    lengths: HashMap<(TextKind, u64), usize>,
    included: bool,
}

impl ItemOutput {
    fn text(
        &mut self,
        kind: TextKind,
        index: u64,
        text: &str,
        snapshot: bool,
        pending: &mut VecDeque<Result<StreamEvent, GatewayError>>,
    ) {
        if text.is_empty() {
            return;
        }
        let length = self.lengths.entry((kind, index)).or_default();
        let suffix = if snapshot {
            // get() also guards UTF-8 boundaries if an upstream sends an inconsistent snapshot.
            let suffix = text.get(*length..).unwrap_or_default();
            *length = (*length).max(text.len());
            suffix
        } else {
            *length += text.len();
            text
        };
        if !suffix.is_empty() {
            pending.push_back(Ok(kind.event(suffix.to_owned())));
        }
    }

    fn part(
        &mut self,
        part: &Value,
        index: u64,
        pending: &mut VecDeque<Result<StreamEvent, GatewayError>>,
    ) {
        let (kind, field) = match part.get("type").and_then(Value::as_str) {
            Some("output_text") => (TextKind::Text, "text"),
            Some("refusal") => (TextKind::Refusal, "refusal"),
            Some("reasoning_text") => (TextKind::Reasoning, "text"),
            Some("summary_text") => (TextKind::Summary, "text"),
            _ => return,
        };
        if let Some(text) = part.get(field).and_then(Value::as_str) {
            self.text(kind, index, text, true, pending);
        }
    }
}

#[derive(Default)]
struct ResponseOutput {
    items: Vec<ItemOutput>,
}

impl ResponseOutput {
    fn item(&mut self, index: Option<u64>, id: Option<&str>) -> &mut ItemOutput {
        let index = index.or_else(|| id.is_none().then_some(0));
        // Compatible relays may reuse or reorder output_index; explicit item IDs are authoritative.
        let position = id.and_then(|id| self.items.iter().position(|item| item.id.as_deref() == Some(id)))
            .or_else(|| index.and_then(|index| self.items.iter().rposition(|item| {
                item.index == Some(index) && (id.is_none() || item.id.is_none())
            })));
        let item = if let Some(position) = position {
            &mut self.items[position]
        } else {
            self.items.push(ItemOutput::default());
            self.items.last_mut().unwrap()
        };
        if item.index.is_none() {
            item.index = index;
        }
        if item.id.is_none() {
            item.id = id.map(str::to_owned);
        }
        item
    }

    fn snapshot(
        &mut self,
        index: Option<u64>,
        item: Value,
        include_tool_items: bool,
        pending: &mut VecDeque<Result<StreamEvent, GatewayError>>,
    ) {
        let output = self.item(index, item.get("id").and_then(Value::as_str));
        for field in ["summary", "content"] {
            if let Some(parts) = item.get(field).and_then(Value::as_array) {
                for (index, part) in parts.iter().enumerate() {
                    output.part(part, index as u64, pending);
                }
            }
        }
        if include_tool_items && !output.included {
            output.included = true;
            pending.push_back(Ok(StreamEvent::ResponseItem(item)));
        }
    }
}

pub async fn stream_responses(
    http: &reqwest::Client,
    provider: &Provider,
    request: &ChatRequest<'_>,
) -> Result<BoxStream<'static, Result<StreamEvent, GatewayError>>, GatewayError> {
    let mut builder = http
        .post(endpoint(&provider.base_url, "/responses"))
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .bearer_auth(&provider.api_key)
        .json(&body(request)?);
    if let Some(header) = &provider.session_header {
        builder = builder.header(header.as_str(), request.session_id);
    }
    let response = check_status(builder.send().await.map_err(transport)?).await?;
    Ok(event_stream(response, request.tools.is_some()))
}

pub(crate) fn event_stream(
    response: reqwest::Response,
    include_tool_items: bool,
) -> BoxStream<'static, Result<StreamEvent, GatewayError>> {
    let events = response.bytes_stream().eventsource();
    stream::unfold(
        (events, VecDeque::new(), false, ResponseOutput::default()),
        move |(mut events, mut pending, done, mut output)| async move {
            let mut done = done;
            loop {
                if let Some(event) = pending.pop_front() {
                    return Some((event, (events, pending, done, output)));
                }
                if done {
                    return None;
                }
                let event = match events.next().await {
                    Some(Ok(event)) => event,
                    Some(Err(error)) => {
                        return Some((
                            Err(GatewayError::Transport(error.to_string())),
                            (events, pending, true, output),
                        ));
                    }
                    None => {
                        return Some((
                            Err(GatewayError::Transport(
                                "Responses stream ended before response.completed".into(),
                            )),
                            (events, pending, true, output),
                        ));
                    }
                };
                let data = event.data.trim();
                if data.is_empty() {
                    continue;
                }
                if data == "[DONE]" {
                    return Some((
                        Err(GatewayError::Transport(
                            "Responses ended without response.completed".into(),
                        )),
                        (events, pending, true, output),
                    ));
                }
                let mut value: Value = match serde_json::from_str(data) {
                    Ok(value) => value,
                    Err(error) => {
                        return Some((
                            Err(GatewayError::Protocol(format!(
                                "invalid Responses SSE JSON: {error}"
                            ))),
                            (events, pending, true, output),
                        ));
                    }
                };
                let kind = value
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or(&event.event);
                match kind {
                    "response.output_item.done" => {
                        let index = value.get("output_index").and_then(Value::as_u64);
                        if let Some(item) = value.as_object_mut().and_then(|value| value.remove("item")) {
                            output.snapshot(index, item, include_tool_items, &mut pending);
                        }
                    }
                    "response.output_text.delta" | "response.refusal.delta"
                    | "response.reasoning_summary_text.delta" | "response.reasoning_text.delta"
                    | "response.output_text.done" | "response.refusal.done"
                    | "response.reasoning_summary_text.done" | "response.reasoning_text.done" => {
                        let (text_kind, field) = match kind {
                            "response.refusal.delta" | "response.refusal.done" => (TextKind::Refusal, "refusal"),
                            "response.reasoning_summary_text.delta" | "response.reasoning_summary_text.done" => (TextKind::Summary, "text"),
                            "response.reasoning_text.delta" | "response.reasoning_text.done" => (TextKind::Reasoning, "text"),
                            _ => (TextKind::Text, "text"),
                        };
                        let snapshot = kind.ends_with(".done");
                        if let Some(text) = value.get(if snapshot { field } else { "delta" }).and_then(Value::as_str) {
                            let item = output.item(
                                value.get("output_index").and_then(Value::as_u64),
                                value.get("item_id").and_then(Value::as_str),
                            );
                            let index_field = if text_kind == TextKind::Summary { "summary_index" } else { "content_index" };
                            let index = value.get(index_field).and_then(Value::as_u64).unwrap_or(0);
                            item.text(text_kind, index, text, snapshot, &mut pending);
                        }
                    }
                    "response.content_part.done" | "response.reasoning_summary_part.done" => {
                        if let Some(part) = value.get("part") {
                            let item = output.item(
                                value.get("output_index").and_then(Value::as_u64),
                                value.get("item_id").and_then(Value::as_str),
                            );
                            let index_field = if kind == "response.reasoning_summary_part.done" { "summary_index" } else { "content_index" };
                            let index = value.get(index_field).and_then(Value::as_u64).unwrap_or(0);
                            item.part(part, index, &mut pending);
                        }
                    }
                    "response.completed" | "response.failed" | "response.incomplete" => {
                        let status = value.pointer("/response/status").and_then(Value::as_str);
                        let incomplete = kind == "response.incomplete" || status == Some("incomplete");
                        let failed = kind == "response.failed" || status == Some("failed") || incomplete;
                        if let Some(items) = value.get_mut("response").and_then(Value::as_object_mut).and_then(|response| response.remove("output")) {
                            if let Value::Array(items) = items {
                                for (index, item) in items.into_iter().enumerate() {
                                    output.snapshot(Some(index as u64), item, include_tool_items, &mut pending);
                                }
                            }
                        }
                        if let Some(record) = value
                            .pointer("/response/usage")
                            .filter(|value| value.is_object())
                        {
                            pending.push_back(Ok(StreamEvent::Usage(usage(record))));
                        }
                        if failed {
                            pending.push_back(Err(GatewayError::Protocol(terminal_message(
                                &value, incomplete,
                            ))));
                        }
                        done = true;
                    }
                    "error" => {
                        pending.push_back(Err(GatewayError::Protocol(terminal_message(
                            &value, false,
                        ))));
                        done = true;
                    }
                    _ => {}
                }
            }
        },
    )
    .boxed()
}
