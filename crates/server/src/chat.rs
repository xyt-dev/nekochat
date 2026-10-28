//! One chat turn: validation, context assembly, the streaming generation task and its SSE.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Extension;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use futures_util::{StreamExt, stream};
use nekochat_core::{Metrics, Price, ReasoningSpec, ReasoningStyle, Usage};
use nekochat_gateway::{
    ChatImage, ChatMessage, ChatRequest, ChatRole, GatewayError, Provider, StreamEvent,
};
use nekochat_resources::{ResourceCatalog, ResourceChunk, ResourceRef};
use nekochat_store::{
    Character, Conversation, DEFAULT_TITLE, ListedModel, Node, NodeRole, NodeStatus, Store,
    StoreError, UsagePurpose, UsageRecord, new_id, now_ms,
};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::accounts::CurrentUser;
use crate::api::user::NodeView;
use crate::api::{ApiError, JsonBody, blocking};
use crate::state::{AppState, TurnGuard};

/// Idle interval between SSE comment lines; each write lets hyper notice a vanished client.
const KEEP_ALIVE: Duration = Duration::from_secs(1);
/// Events buffered between the generation task and the SSE response.
const EVENT_BUFFER: usize = 64;
/// A fresh conversation is titled with this many leading characters of its first message.
const TITLE_CHARS: usize = 30;

fn model_label(display_name: &str, provider_label: &str) -> String {
    format!("{display_name} · {provider_label}")
}

/// Body of `POST /api/conversations/{id}/turns`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnRequest {
    pub action: Action,
    pub character_id: String,
    pub content: Option<String>,
    pub node_id: Option<String>,
    pub attachment_ids: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Send,
    Edit,
    Regenerate,
}

/// Transport-neutral events; future adapters receive the actual resource reference.
pub enum TurnEvent {
    Start {
        user_node: Option<Node>,
        assistant_node_id: String,
        character_id: String,
        character_name: String,
        character_avatar: Option<String>,
    },
    Reasoning(String),
    Chunk(ResourceChunk),
    Done(Node),
    Error {
        message: String,
        node: Option<Node>,
    },
}

pub type TurnStream = futures_util::stream::BoxStream<'static, TurnEvent>;

/// The turn's user message.
struct Anchor {
    parent: Option<String>,
    content: String,
    attachment_ids: Vec<String>,
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
    recent_resources: Vec<String>,
}

/// `POST /api/conversations/{id}/turns`: validates, stores the user message, then streams the
/// reply as SSE (`start`, `reasoning`/`text` deltas, then `done` or `error`).
pub async fn post_turn(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path(conv_id): Path<String>,
    JsonBody(req): JsonBody<TurnRequest>,
) -> Result<Response, ApiError> {
    let events = start_turn(state, user.id, conv_id, req)
        .await?
        .map(|event| Ok::<_, Infallible>(http_event(event)));
    Ok(Sse::new(events)
        .keep_alive(KeepAlive::new().interval(KEEP_ALIVE))
        .into_response())
}

/// Starts one guarded turn without an HTTP request or SSE dependency.
pub async fn start_turn(
    state: AppState,
    actor_user_id: String,
    conv_id: String,
    req: TurnRequest,
) -> Result<TurnStream, ApiError> {
    // Every transport must supply its authenticated actor; middleware alone is insufficient.
    // Check before taking a turn claim, reading history or contacting any provider.
    let store = state.store.clone();
    let owner_conv = conv_id.clone();
    let actor_user_id = blocking(move || {
        store.require_conversation_owner(&owner_conv, &actor_user_id)?;
        Ok(actor_user_id)
    })
    .await?;
    let TurnRequest {
        action,
        character_id,
        content,
        node_id,
        attachment_ids,
    } = req;
    let target_id = match action {
        Action::Send => None,
        Action::Edit | Action::Regenerate => {
            Some(node_id.ok_or_else(|| ApiError::bad_request("缺少消息节点"))?)
        }
    };
    // Claim first: the running turn saves its reply before releasing the claim, so the leaf and
    // title read below are never older than that reply.
    let (guard, cancel) = state
        .begin_turn(&conv_id)
        .ok_or_else(|| ApiError::new(StatusCode::CONFLICT, "该对话正在生成回复"))?;
    let store = state.store.clone();
    let (conv, character, target, actor_user_id) = blocking(move || {
        // The account may have been disabled/reassigned between the first check and claim.
        // Once claimed, account invalidation can cancel this turn reliably.
        store.require_conversation_owner(&conv_id, &actor_user_id)?;
        let target = match target_id {
            Some(node_id) => store.get_node(&conv_id, &node_id)?,
            None => None,
        };
        Ok((
            store.get_conversation(&conv_id)?,
            store.get_character(&character_id)?,
            target,
            actor_user_id,
        ))
    })
    .await?;

    let conv = conv.ok_or_else(ApiError::not_found)?;
    let character = character
        .filter(|c| c.spec.enabled)
        .ok_or_else(|| ApiError::bad_request("角色不可用"))?;
    let model_id = character
        .spec
        .model_id
        .clone()
        .ok_or_else(|| ApiError::bad_request("角色未绑定模型"))?;
    let store = state.store.clone();
    let model = blocking(move || store.get_model(&model_id))
        .await?
        .ok_or_else(|| ApiError::bad_request("角色绑定模型未上架"))?;
    character
        .spec
        .parameters
        .validate_for_protocol(&model.spec.protocol)
        .map_err(ApiError::bad_request)?;
    let provider = state.authenticated_provider(&model.spec.provider).await?;
    let effort = resolve_effort(
        &model.spec.reasoning,
        character.spec.parameters.effort.clone(),
    )?;
    let content = content.unwrap_or_default();
    let anchor = match action {
        Action::Send => Anchor {
            parent: conv.leaf_id.clone(),
            content,
            attachment_ids: attachment_ids.unwrap_or_default(),
            existing: None,
        },
        Action::Edit => {
            let node = user_node(target)?;
            Anchor {
                parent: node.parent_id,
                content,
                attachment_ids: attachment_ids
                    .unwrap_or_else(|| node.attachments.into_iter().map(|a| a.id).collect()),
                existing: None,
            }
        }
        Action::Regenerate => {
            let node = user_node(target)?;
            Anchor {
                parent: node.parent_id,
                content: node.content,
                attachment_ids: node.attachments.into_iter().map(|a| a.id).collect(),
                existing: Some(node.id),
            }
        }
    };
    if anchor.attachment_ids.len() > 5 {
        return Err(ApiError::bad_request("每条消息最多 5 个附件"));
    }
    let unique: std::collections::HashSet<_> = anchor.attachment_ids.iter().collect();
    if unique.len() != anchor.attachment_ids.len() {
        return Err(ApiError::bad_request("附件不能重复"));
    }
    if anchor.content.trim().is_empty() && anchor.attachment_ids.is_empty() {
        return Err(ApiError::bad_request("消息内容或附件不能为空"));
    }
    let store = state.store.clone();
    let check_conv = conv.id.clone();
    let check_ids = anchor.attachment_ids.clone();
    let check_parent = anchor.parent.clone();
    let has_images = blocking(move || {
        let current = match store.attachment_metadata(&check_conv, &check_ids) {
            Ok(items) => items,
            Err(StoreError::NotFound) => return Ok(None),
            Err(error) => return Err(error),
        };
        Ok(Some(
            current.iter().any(|attachment| attachment.kind == "image")
                || store
                    .path_to(&check_conv, check_parent.as_deref())?
                    .iter()
                    .any(|node| {
                        node.attachments
                            .iter()
                            .any(|attachment| attachment.kind == "image")
                    }),
        ))
    })
    .await?
    .ok_or_else(|| ApiError::bad_request("附件不存在或不属于当前对话"))?;
    if has_images && !model.spec.vision {
        return Err(ApiError::bad_request(
            "当前角色不支持图片输入，请选择支持图片的角色",
        ));
    }

    let prompt = state
        .resources
        .prompt(
            &character.spec.resource_packs,
            &character.spec.sticker_identity,
            &character.spec.sticker_frequency,
        )
        .map_err(|_| ApiError::bad_request("角色资源包不可用"))?;
    let store = state.store.clone();
    let selected = character.clone();
    let prepared = blocking(move || {
        store.set_user_preferences(&actor_user_id, Some(&selected.id), Some(&conv.id))?;
        prepare(&store, conv, anchor, &selected, &prompt)
    })
    .await?;
    let turn = Turn {
        label: model_label(&model.spec.display_name, &provider.label),
        store: state.store,
        http: state.http,
        provider,
        model,
        effort,
        prepared,
        assistant_id: new_id(),
        character,
        resources: state.resources,
    };
    let (tx, rx) = mpsc::channel(EVENT_BUFFER);
    tokio::spawn(turn.run(Outbox { tx, cancel }, guard));
    Ok(stream::unfold(
        rx,
        |mut rx| async move { rx.recv().await.map(|ev| (ev, rx)) },
    )
    .boxed())
}

/// The effort to request: a listed level, the model's default for `null`, none for models
/// without reasoning.
pub(crate) fn resolve_effort(
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

/// Builds the upstream context, stores the new user message and titles a fresh conversation.
fn prepare(
    store: &Store,
    conv: Conversation,
    anchor: Anchor,
    character: &Character,
    resource_prompt: &str,
) -> Result<Prepared, StoreError> {
    let attachments = store.attachment_metadata(&conv.id, &anchor.attachment_ids)?;
    let mut messages = context(
        store,
        &conv,
        anchor.parent.as_deref(),
        &anchor.content,
        &anchor.attachment_ids,
    )?;
    let prompt = [character.spec.system_prompt.as_str(), resource_prompt]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    if !prompt.is_empty() {
        messages.insert(
            0,
            ChatMessage {
                role: ChatRole::System,
                content: prompt,
                images: Vec::new(),
            },
        );
    }
    store.set_character(&conv.id, &character.id)?;
    let recent_resources = store
        .path_to(&conv.id, anchor.parent.as_deref())?
        .iter()
        .rev()
        .filter(|node| node.role == NodeRole::Assistant)
        .take(6)
        .flat_map(|node| recent_refs(&node.content))
        .collect();
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
                attachments,
                reasoning: None,
                model_id: None,
                model_label: None,
                effort: None,
                character_id: None,
                character_name: None,
                character_avatar: None,
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
        recent_resources,
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
    attachment_ids: &[String],
) -> Result<Vec<ChatMessage>, StoreError> {
    let mut messages = Vec::new();
    for node in store.path_to(&conv.id, parent)? {
        let role = match node.role {
            NodeRole::User => ChatRole::User,
            NodeRole::Assistant if node.content.is_empty() => continue,
            NodeRole::Assistant => ChatRole::Assistant,
        };
        messages.push(message_with_attachments(
            store,
            &conv.id,
            role,
            node.content,
            node.attachments
                .iter()
                .map(|attachment| attachment.id.as_str()),
        )?);
    }
    messages.push(message_with_attachments(
        store,
        &conv.id,
        ChatRole::User,
        user_text.to_owned(),
        attachment_ids.iter().map(String::as_str),
    )?);
    Ok(messages)
}

/// Documents remain untrusted user-message content. Only images become multimodal input.
fn message_with_attachments<'a>(
    store: &Store,
    conv: &str,
    role: ChatRole,
    mut content: String,
    ids: impl IntoIterator<Item = &'a str>,
) -> Result<ChatMessage, StoreError> {
    let mut images = Vec::new();
    for id in ids {
        let (attachment, bytes, text) = store
            .attachment_model_content(conv, id)?
            .ok_or(StoreError::NotFound)?;
        if attachment.kind == "image" {
            images.push(ChatImage {
                media_type: attachment.media_type,
                base64: base64::engine::general_purpose::STANDARD
                    .encode(bytes.ok_or(StoreError::NotFound)?),
            });
        } else {
            let text = text.ok_or(StoreError::NotFound)?;
            content.push_str("\n\n--- Attached document: ");
            content.push_str(&attachment.name);
            content.push_str(" ---\n");
            content.push_str(&text);
            content.push_str("\n--- End attached document ---");
        }
    }
    Ok(ChatMessage {
        role,
        content,
        images,
    })
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

fn sticker_markdown(resource: &ResourceRef) -> String {
    let Some(url) = crate::assets::resource_url(&resource.pack_id, &resource.asset_id) else {
        return String::new();
    };
    let alt = resource
        .name
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace(['\r', '\n'], " ");
    format!("![{alt}]({url})")
}

fn recent_refs(content: &str) -> Vec<String> {
    content
        .split("](/api/resources/")
        .skip(1)
        .filter_map(|tail| {
            let path = tail.split_once(')')?.0;
            let (pack, asset) = path.split_once('/')?;
            crate::assets::resource_url(pack, asset)?;
            Some(format!("{pack}/{asset}"))
        })
        .collect()
}

fn http_event(value: TurnEvent) -> Event {
    let (name, data) = match value {
        TurnEvent::Start {
            user_node,
            assistant_node_id,
            character_id,
            character_name,
            character_avatar,
        } => (
            "start",
            json!({"user_node": user_node.map(NodeView::from), "assistant_node_id":assistant_node_id, "character_id":character_id,"character_name":character_name,"character_avatar":character_avatar}),
        ),
        TurnEvent::Reasoning(t) => ("reasoning", json!({"t":t})),
        TurnEvent::Chunk(ResourceChunk::Text(t)) => ("text", json!({"t":t})),
        TurnEvent::Chunk(ResourceChunk::Sticker(resource)) => {
            ("text", json!({"t":sticker_markdown(&resource)}))
        }
        TurnEvent::Done(node) => ("done", json!({"node":NodeView::from(node)})),
        TurnEvent::Error { message, node } => (
            "error",
            json!({"message":message,"node":node.map(NodeView::from)}),
        ),
    };
    Event::default().event(name).data(data.to_string())
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
    tx: mpsc::Sender<TurnEvent>,
    cancel: CancellationToken,
}

impl Outbox {
    async fn send(&self, event: TurnEvent) -> Result<(), End> {
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
    /// Arrival of the first reasoning delta and of the first text delta.
    first_reasoning: Option<Instant>,
    first_text: Option<Instant>,
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
            reasoning_ms: self.first_reasoning.and_then(|start| {
                self.first_text
                    .or(self.last)
                    .map(|end| millis(end.saturating_duration_since(start)))
            }),
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
    character: Character,
    resources: Arc<ResourceCatalog>,
}

impl Turn {
    /// `out` outlives `guard`: once the SSE stream ends, the conversation is free again.
    async fn run(self, out: Outbox, guard: TurnGuard) {
        self.drive(&out, guard).await;
    }

    async fn drive(mut self, out: &Outbox, guard: TurnGuard) {
        let start = TurnEvent::Start {
            user_node: self.prepared.user_node.take(),
            assistant_node_id: self.assistant_id.clone(),
            character_id: self.character.id.clone(),
            character_name: self.character.spec.display_name.clone(),
            character_avatar: self.character.spec.avatar.clone(),
        };
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
            character,
            ..
        } = self;
        let has_output = !progress.content.is_empty() || !progress.reasoning.is_empty();
        let node = (status == NodeStatus::Complete || has_output).then(|| Node {
            id: assistant_id,
            conversation_id: prepared.conv_id.clone(),
            parent_id: Some(prepared.reply_parent),
            role: NodeRole::Assistant,
            content: progress.content,
            attachments: Vec::new(),
            reasoning: Some(progress.reasoning).filter(|r| !r.is_empty()),
            model_id: Some(model.id),
            model_label: Some(label.clone()),
            effort,
            character_id: Some(character.id),
            character_name: Some(character.spec.display_name),
            character_avatar: character.spec.avatar,
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
                Ok(Some(node)) => TurnEvent::Done(node),
                _ => TurnEvent::Error {
                    message: "保存回复失败".into(),
                    node: None,
                },
            },
            End::Failed(e) => TurnEvent::Error {
                message: e.to_string(),
                node: saved.ok().flatten(),
            },
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
            parameters: &self.character.spec.parameters,
        };
        let mut upstream = tokio::select! {
            biased;
            end = out.stopped() => return end,
            connected = async {
                match self.model.spec.protocol {
                    nekochat_core::Protocol::ChatCompletions =>
                        nekochat_gateway::stream_chat(&self.http, &self.provider, &req).await,
                    nekochat_core::Protocol::Responses =>
                        nekochat_gateway::stream_responses(&self.http, &self.provider, &req).await,
                    nekochat_core::Protocol::ChatgptResponses =>
                        nekochat_gateway::stream_chatgpt(&self.http, &self.provider, &req).await,
                }
            } => {
                match connected {
                    Ok(upstream) => upstream,
                    Err(e) => return End::Failed(e),
                }
            }
        };
        let mut filter = match self.resources.filter(
            &self.character.spec.resource_packs,
            &self.prepared.recent_resources,
        ) {
            Ok(filter) => filter,
            Err(e) => return End::Failed(GatewayError::Protocol(e.to_string())),
        };
        loop {
            let item = tokio::select! {
                biased;
                end = out.stopped() => return end,
                item = upstream.next() => item,
            };
            let now = Instant::now();
            let chunks = match item {
                None => {
                    if let Err(end) = emit_chunks(out, progress, filter.finish()).await {
                        return end;
                    }
                    return End::Done;
                }
                Some(Err(e)) => {
                    if let Err(end) = emit_chunks(out, progress, filter.finish()).await {
                        return end;
                    }
                    return End::Failed(e);
                }
                Some(Ok(StreamEvent::Usage(u))) => {
                    progress.usage = Some(u);
                    continue;
                }
                Some(Ok(StreamEvent::Reasoning(t))) => {
                    progress.first_reasoning.get_or_insert(now);
                    progress.reasoning.push_str(&t);
                    if let Err(end) = out.send(TurnEvent::Reasoning(t)).await {
                        return end;
                    }
                    Vec::new()
                }
                Some(Ok(StreamEvent::Text(t))) => {
                    progress.first_text.get_or_insert(now);
                    filter.push(&t)
                }
            };
            progress.first.get_or_insert(now);
            progress.last = Some(now);
            if let Err(end) = emit_chunks(out, progress, chunks).await {
                return end;
            }
        }
    }
}

async fn emit_chunks(
    out: &Outbox,
    progress: &mut Progress,
    chunks: Vec<ResourceChunk>,
) -> Result<(), End> {
    for chunk in chunks {
        match &chunk {
            ResourceChunk::Text(t) => progress.content.push_str(t),
            ResourceChunk::Sticker(resource) => {
                progress.content.push_str(&sticker_markdown(resource))
            }
        }
        out.send(TurnEvent::Chunk(chunk)).await?;
    }
    Ok(())
}
