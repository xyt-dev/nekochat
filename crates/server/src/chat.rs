//! One chat turn: validation, context assembly, the streaming generation task and its SSE.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures_util::{StreamExt, stream};
use nekochat_core::{Metrics, Price, ReasoningSpec, ReasoningStyle, Usage};
use nekochat_gateway::{ChatMessage, ChatRequest, ChatRole, GatewayError, Provider, StreamEvent};
use nekochat_store::{
    Conversation, DEFAULT_TITLE, ListedModel, Node, NodeRole, NodeStatus, Store, StoreError,
    UsagePurpose, UsageRecord, new_id, now_ms,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::api::user::{NodeView, model_label};
use crate::api::{ApiError, JsonBody, blocking};
use crate::state::{AppState, TurnGuard};

/// Idle interval between SSE comment lines; each write lets hyper notice a vanished client.
const KEEP_ALIVE: Duration = Duration::from_secs(1);
/// Events buffered between the generation task and the SSE response.
const EVENT_BUFFER: usize = 64;
/// A fresh conversation is titled with this many leading characters of its first message.
const TITLE_CHARS: usize = 30;

/// Body of `POST /api/conversations/{id}/turns`.
#[derive(Deserialize)]
pub struct TurnRequest {
    #[serde(flatten)]
    action: Action,
    model_id: String,
    effort: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum Action {
    /// A new user message under the current leaf.
    Send { content: String },
    /// A replacement for user message `node_id`: a sibling under the same parent.
    Edit { node_id: String, content: String },
    /// Another reply to user message `node_id`.
    Regenerate { node_id: String },
}

/// The turn's user message.
struct Anchor {
    parent: Option<String>,
    content: String,
    /// Id of the existing user message answered again; `None` stores `content` as a new one.
    existing: Option<String>,
}

/// What the generation task needs from the database.
struct Prepared {
    conv_id: String,
    session_key: String,
    messages: Vec<ChatMessage>,
    /// The stored new user message; `None` when regenerating.
    user_node: Option<Node>,
    /// The turn's user message, parent of the reply.
    reply_parent: String,
}

/// `POST /api/conversations/{id}/turns`: validates, stores the user message, then streams the
/// reply as SSE (`start`, `reasoning`/`text` deltas, then `done` or `error`).
pub async fn post_turn(
    State(state): State<AppState>,
    Path(conv_id): Path<String>,
    JsonBody(req): JsonBody<TurnRequest>,
) -> Result<Response, ApiError> {
    let TurnRequest {
        action,
        model_id,
        effort,
    } = req;
    let target_id = match &action {
        Action::Send { .. } => None,
        Action::Edit { node_id, .. } | Action::Regenerate { node_id } => Some(node_id.clone()),
    };
    // Claim first: the running turn saves its reply before releasing the claim, so the leaf and
    // title read below are never older than that reply.
    let (guard, cancel) = state
        .begin_turn(&conv_id)
        .ok_or_else(|| ApiError::new(StatusCode::CONFLICT, "该对话正在生成回复"))?;
    let store = state.store.clone();
    let (conv, model, target) = blocking(move || {
        let target = match target_id {
            Some(node_id) => store.get_node(&conv_id, &node_id)?,
            None => None,
        };
        Ok((
            store.get_conversation(&conv_id)?,
            store.get_model(&model_id)?,
            target,
        ))
    })
    .await?;

    let conv = conv.ok_or_else(ApiError::not_found)?;
    let model = model.ok_or_else(|| ApiError::bad_request("模型未上架"))?;
    let provider = state
        .provider(&model.spec.provider)
        .ok_or_else(|| ApiError::bad_request("模型的提供商不在配置中"))?
        .clone();
    let effort = resolve_effort(&model.spec.reasoning, effort)?;
    let anchor = match action {
        Action::Send { content } => Anchor {
            parent: conv.leaf_id.clone(),
            content: non_empty(content)?,
            existing: None,
        },
        Action::Edit { content, .. } => Anchor {
            parent: user_node(target)?.parent_id,
            content: non_empty(content)?,
            existing: None,
        },
        Action::Regenerate { .. } => {
            let node = user_node(target)?;
            Anchor {
                parent: node.parent_id,
                content: node.content,
                existing: Some(node.id),
            }
        }
    };

    let store = state.store.clone();
    let prepared = blocking(move || prepare(&store, conv, anchor)).await?;
    let turn = Turn {
        label: model_label(&model.spec.display_name, &provider.label),
        store: state.store,
        http: state.http,
        provider,
        model,
        effort,
        prepared,
        assistant_id: new_id(),
    };
    let (tx, rx) = mpsc::channel(EVENT_BUFFER);
    tokio::spawn(turn.run(Outbox { tx, cancel }, guard));
    let events = stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|ev| (Ok::<_, Infallible>(ev), rx))
    });
    Ok(Sse::new(events)
        .keep_alive(KeepAlive::new().interval(KEEP_ALIVE))
        .into_response())
}

/// The effort to request: a listed level, the model's default for `null`, none for models
/// without reasoning.
fn resolve_effort(
    spec: &ReasoningSpec,
    effort: Option<String>,
) -> Result<Option<String>, ApiError> {
    match (&spec.style, effort) {
        (ReasoningStyle::None, None) => Ok(None),
        (ReasoningStyle::None, Some(_)) => Err(ApiError::bad_request("该模型不支持思考强度")),
        (_, None) => Ok(spec.default.clone()),
        (_, Some(e)) if spec.levels.contains(&e) => Ok(Some(e)),
        (_, Some(_)) => Err(ApiError::bad_request("思考强度不在该模型的档位里")),
    }
}

fn user_node(node: Option<Node>) -> Result<Node, ApiError> {
    node.filter(|n| n.role == NodeRole::User)
        .ok_or_else(|| ApiError::bad_request("节点不存在或不是用户消息"))
}

fn non_empty(content: String) -> Result<String, ApiError> {
    if content.trim().is_empty() {
        return Err(ApiError::bad_request("消息内容不能为空"));
    }
    Ok(content)
}

/// Builds the upstream context, stores the new user message and titles a fresh conversation.
fn prepare(store: &Store, conv: Conversation, anchor: Anchor) -> Result<Prepared, StoreError> {
    let messages = context(store, &conv, anchor.parent.as_deref(), &anchor.content)?;
    let title: Option<String> = (conv.title == DEFAULT_TITLE)
        .then(|| anchor.content.chars().take(TITLE_CHARS).collect::<String>())
        .filter(|t| !t.is_empty());
    let (user_node, reply_parent) = match anchor.existing {
        Some(id) => (None, id),
        None => {
            let node = Node {
                id: new_id(),
                conversation_id: conv.id.clone(),
                parent_id: anchor.parent,
                role: NodeRole::User,
                content: anchor.content,
                reasoning: None,
                model_id: None,
                model_label: None,
                effort: None,
                status: NodeStatus::Complete,
                metrics: None,
                created_at: now_ms(),
            };
            store.insert_node(&node)?;
            let reply_parent = node.id.clone();
            (Some(node), reply_parent)
        }
    };
    if let Some(title) = title {
        store.set_title(&conv.id, &title)?;
    }
    Ok(Prepared {
        conv_id: conv.id,
        session_key: conv.session_key,
        messages,
        user_node,
        reply_parent,
    })
}

/// Upstream context: the character prompt (if any), the path from the root down to `parent`,
/// then `user_text`. Content only: reasoning is never sent back, and replies without text
/// (failed, or interrupted while thinking) are skipped.
fn context(
    store: &Store,
    conv: &Conversation,
    parent: Option<&str>,
    user_text: &str,
) -> Result<Vec<ChatMessage>, StoreError> {
    let mut messages = Vec::new();
    if let Some(id) = &conv.character_id
        && let Some(character) = store.get_character(id)?
    {
        messages.push(ChatMessage {
            role: ChatRole::System,
            content: character.system_prompt,
        });
    }
    for node in store.path_to(&conv.id, parent)? {
        let role = match node.role {
            NodeRole::User => ChatRole::User,
            NodeRole::Assistant if node.content.is_empty() => continue,
            NodeRole::Assistant => ChatRole::Assistant,
        };
        messages.push(ChatMessage {
            role,
            content: node.content,
        });
    }
    messages.push(ChatMessage {
        role: ChatRole::User,
        content: user_text.to_owned(),
    });
    Ok(messages)
}

/// Cost in USD of `u` at `price` (USD per million tokens); `None` unless all three prices are set.
pub fn cost_usd(price: &Price, u: &Usage) -> Option<f64> {
    Some(
        (u.cached_in as f64 * price.cached_in?
            + u.uncached_in as f64 * price.uncached_in?
            + (u.output + u.reasoning) as f64 * price.output?)
            / 1e6,
    )
}

fn event(name: &'static str, data: Value) -> Event {
    Event::default().event(name).data(data.to_string())
}

fn error_event(message: String, node: Option<Node>) -> Event {
    event(
        "error",
        json!({"message": message, "node": node.map(NodeView::from)}),
    )
}

/// How a generation ended.
enum End {
    /// The upstream finished normally.
    Done,
    /// The upstream failed, before or during the reply.
    Failed(GatewayError),
    /// The SSE client went away.
    Disconnected,
    /// The conversation was deleted or cleared.
    Cancelled,
}

impl End {
    /// Status of the reply to keep; `None` when nothing may be kept.
    fn status(&self) -> Option<NodeStatus> {
        match self {
            End::Done => Some(NodeStatus::Complete),
            End::Failed(_) => Some(NodeStatus::Error),
            End::Disconnected => Some(NodeStatus::Interrupted),
            End::Cancelled => None,
        }
    }
}

/// The turn's SSE side and its cancellation.
struct Outbox {
    tx: mpsc::Sender<Event>,
    cancel: CancellationToken,
}

impl Outbox {
    async fn send(&self, event: Event) -> Result<(), End> {
        tokio::select! {
            biased;
            () = self.cancel.cancelled() => Err(End::Cancelled),
            sent = self.tx.send(event) => sent.map_err(|_| End::Disconnected),
        }
    }

    /// Resolves once the turn must stop, whatever the upstream does.
    async fn stopped(&self) -> End {
        tokio::select! {
            biased;
            () = self.cancel.cancelled() => End::Cancelled,
            () = self.tx.closed() => End::Disconnected,
        }
    }
}

/// What has streamed so far.
#[derive(Default)]
struct Progress {
    content: String,
    reasoning: String,
    usage: Option<Usage>,
    /// Arrival of the first and of the latest reasoning/text delta.
    first: Option<Instant>,
    last: Option<Instant>,
}

impl Progress {
    fn metrics(&self, t0: Instant, end: Instant) -> Metrics {
        let tokens_per_sec = match (&self.usage, self.first, self.last) {
            (Some(u), Some(first), Some(last)) => {
                let span = last - first;
                // Spans below the millisecond resolution of the other metrics count as zero.
                (span.as_millis() > 0).then(|| (u.output + u.reasoning) as f64 / span.as_secs_f64())
            }
            _ => None,
        };
        Metrics {
            ttft_ms: self.first.map(|first| millis(first - t0)),
            total_ms: millis(end - t0),
            tokens_per_sec,
            usage: self.usage.clone().unwrap_or_default(),
        }
    }
}

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// A validated turn, ready to stream.
struct Turn {
    store: Arc<Store>,
    http: reqwest::Client,
    provider: Provider,
    model: ListedModel,
    label: String,
    effort: Option<String>,
    prepared: Prepared,
    assistant_id: String,
}

impl Turn {
    /// `out` outlives `guard`: once the SSE stream ends, the conversation is free again.
    async fn run(self, out: Outbox, guard: TurnGuard) {
        self.drive(&out, guard).await;
    }

    async fn drive(mut self, out: &Outbox, guard: TurnGuard) {
        let start = event(
            "start",
            json!({
                "user_node": self.prepared.user_node.take().map(NodeView::from),
                "assistant_node_id": self.assistant_id,
                "model_label": self.label,
            }),
        );
        if out.send(start).await.is_err() {
            // Nothing generated yet, so nothing to keep.
            return;
        }
        let t0 = Instant::now();
        let mut progress = Progress::default();
        let end = self.stream(out, &mut progress).await;
        let finished = Instant::now();

        let Some(status) = end.status().filter(|_| !out.cancel.is_cancelled()) else {
            tracing::info!(conversation = %self.prepared.conv_id, "turn cancelled");
            return;
        };
        let metrics = progress.metrics(t0, finished);
        let u = &metrics.usage;
        tracing::info!(
            conversation = %self.prepared.conv_id,
            node = %self.assistant_id,
            model = %self.model.id,
            status = ?status,
            cached_in = u.cached_in,
            uncached_in = u.uncached_in,
            reasoning = u.reasoning,
            output = u.output,
            "turn finished"
        );
        if let End::Failed(e) = &end {
            let (kind, http_status) = match e {
                GatewayError::Http { status, .. } => ("http", Some(*status)),
                GatewayError::Transport(_) => ("transport", None),
                GatewayError::Protocol(_) => ("protocol", None),
            };
            tracing::warn!(
                conversation = %self.prepared.conv_id,
                model = %self.model.id,
                kind,
                http_status,
                "upstream error"
            );
        }

        let Turn {
            store,
            provider,
            model,
            label,
            effort,
            prepared,
            assistant_id,
            ..
        } = self;
        let has_output = !progress.content.is_empty() || !progress.reasoning.is_empty();
        let node = (status == NodeStatus::Complete || has_output).then(|| Node {
            id: assistant_id,
            conversation_id: prepared.conv_id.clone(),
            parent_id: Some(prepared.reply_parent),
            role: NodeRole::Assistant,
            content: progress.content,
            reasoning: Some(progress.reasoning).filter(|r| !r.is_empty()),
            model_id: Some(model.id),
            model_label: Some(label.clone()),
            effort,
            status,
            metrics: Some(metrics),
            created_at: now_ms(),
        });
        let record = progress.usage.map(|usage| UsageRecord {
            at_ms: now_ms(),
            provider: provider.key,
            upstream_id: model.spec.upstream_id,
            model_label: label,
            cost_usd: cost_usd(&model.spec.price, &usage),
            usage,
            purpose: UsagePurpose::Chat,
        });
        let saved = blocking(move || {
            if let Some(node) = &node {
                store.insert_node(node)?;
            }
            if let Some(record) = &record {
                store.insert_usage(record)?;
            }
            Ok(node)
        })
        .await;
        if saved.is_err() {
            tracing::warn!(conversation = %prepared.conv_id, "saving the turn failed");
        }
        drop(guard);

        let last = match end {
            End::Done => match saved {
                Ok(Some(node)) => event("done", json!({ "node": NodeView::from(node) })),
                _ => error_event("保存回复失败".into(), None),
            },
            End::Failed(e) => error_event(e.to_string(), saved.ok().flatten()),
            End::Disconnected | End::Cancelled => return,
        };
        // A client that left meanwhile simply misses it.
        let _ = out.tx.send(last).await;
    }

    /// Forwards upstream deltas into `progress` and the SSE until the upstream ends or the turn
    /// must stop. Returning drops the upstream response, which aborts it.
    async fn stream(&self, out: &Outbox, progress: &mut Progress) -> End {
        let req = ChatRequest {
            model: &self.model.spec.upstream_id,
            messages: &self.prepared.messages,
            reasoning: self.model.spec.reasoning.style.clone(),
            effort: self.effort.as_deref(),
            session_id: &self.prepared.session_key,
        };
        let mut upstream = tokio::select! {
            biased;
            end = out.stopped() => return end,
            connected = nekochat_gateway::stream_chat(&self.http, &self.provider, &req) => {
                match connected {
                    Ok(upstream) => upstream,
                    Err(e) => return End::Failed(e),
                }
            }
        };
        loop {
            let item = tokio::select! {
                biased;
                end = out.stopped() => return end,
                item = upstream.next() => item,
            };
            let delta = match item {
                None => return End::Done,
                Some(Err(e)) => return End::Failed(e),
                Some(Ok(StreamEvent::Usage(u))) => {
                    progress.usage = Some(u);
                    continue;
                }
                Some(Ok(StreamEvent::Reasoning(t))) => {
                    progress.reasoning.push_str(&t);
                    event("reasoning", json!({ "t": t }))
                }
                Some(Ok(StreamEvent::Text(t))) => {
                    progress.content.push_str(&t);
                    event("text", json!({ "t": t }))
                }
            };
            let now = Instant::now();
            progress.first.get_or_insert(now);
            progress.last = Some(now);
            if let Err(end) = out.send(delta).await {
                return end;
            }
        }
    }
}
