//! User-facing conversation API.

use std::borrow::Cow;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use nekochat_core::{Background, Metrics};
use nekochat_store::{Conversation, Node, NodeRole, NodeStatus, SettingKey, StoreError};
use serde::{Deserialize, Serialize};

use crate::accounts::{CurrentUser, require_conversation};
use crate::api::{ApiError, JsonBody, blocking};
use crate::state::AppState;

#[derive(Serialize)]
pub struct ConversationSummary {
    id: String,
    title: String,
    updated_at: i64,
    character_id: Option<String>,
}

impl From<Conversation> for ConversationSummary {
    fn from(c: Conversation) -> ConversationSummary {
        ConversationSummary {
            id: c.id,
            title: c.title,
            updated_at: c.updated_at,
            character_id: c.character_id,
        }
    }
}

#[derive(Serialize)]
pub struct PickerCharacter {
    id: String,
    display_name: String,
    avatar: Option<String>,
    background: Option<Background>,
    vision: bool,
}

#[derive(Serialize)]
pub struct CharactersView {
    default_character_id: Option<String>,
    characters: Vec<PickerCharacter>,
}

impl CharactersView {
    pub fn contains(&self, id: &str) -> bool {
        self.characters.iter().any(|c| c.id == id)
    }
}
pub async fn available_characters(state: &AppState) -> Result<CharactersView, ApiError> {
    let store = state.store.clone();
    let (default_id, listed, models) = blocking(move || {
        Ok((
            store.get_setting(SettingKey::DefaultCharacter)?,
            store.list_characters()?,
            store.list_models()?,
        ))
    })
    .await?;
    let characters: Vec<_> = listed
        .into_iter()
        .filter(|c| {
            c.spec.enabled
                && c.spec.model_id.as_ref().is_some_and(|id| {
                    models
                        .iter()
                        .any(|m| &m.id == id && state.provider(&m.spec.provider).is_some())
                })
        })
        .map(|c| PickerCharacter {
            id: c.id,
            display_name: c.spec.display_name,
            avatar: c.spec.avatar,
            background: c.spec.background,
            vision: models
                .iter()
                .find(|m| Some(&m.id) == c.spec.model_id.as_ref())
                .is_some_and(|m| m.spec.vision),
        })
        .collect();
    let default_character_id = default_id
        .filter(|id| characters.iter().any(|c| &c.id == id))
        .or_else(|| characters.first().map(|c| c.id.clone()));
    Ok(CharactersView {
        default_character_id,
        characters,
    })
}

pub async fn list_characters(
    State(state): State<AppState>,
) -> Result<Json<CharactersView>, ApiError> {
    Ok(Json(available_characters(&state).await?))
}

/// `GET /api/conversations`, newest first.
pub async fn list_conversations(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
) -> Result<Json<Vec<ConversationSummary>>, ApiError> {
    let store = state.store.clone();
    let convs = blocking(move || store.list_user_conversations(&user.id)).await?;
    Ok(Json(convs.into_iter().map(Into::into).collect()))
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CreateConversation {
    character_id: Option<String>,
}

pub async fn create_conversation(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    JsonBody(body): JsonBody<CreateConversation>,
) -> Result<(StatusCode, Json<ConversationSummary>), ApiError> {
    let available = available_characters(&state).await?;
    let id = body
        .character_id
        .or(available.default_character_id)
        .filter(|id| available.characters.iter().any(|c| &c.id == id))
        .ok_or_else(|| ApiError::bad_request("没有可用角色，请管理员配置并绑定已上架模型"))?;
    let store = state.store.clone();
    let (conv, owner) = blocking(move || {
        let conv = store.create_owned_conversation(&user.id, Some(&id))?;
        Ok((conv, user.id))
    }).await?;
    state.account_events.publish(&owner, &conv.id);
    Ok((StatusCode::CREATED, Json(conv.into())))
}
/// Token counts as the client sees them; cost fields stay admin-only.
#[derive(Serialize)]
pub struct UsageView {
    cached_in: u64,
    uncached_in: u64,
    reasoning: u64,
    output: u64,
}

/// Generation metrics without any cost data.
#[derive(Serialize)]
pub struct MetricsView {
    ttft_ms: Option<u64>,
    total_ms: u64,
    tokens_per_sec: Option<f64>,
    reasoning_ms: Option<u64>,
    usage: UsageView,
}

impl From<&Metrics> for MetricsView {
    fn from(m: &Metrics) -> MetricsView {
        MetricsView {
            ttft_ms: m.ttft_ms,
            total_ms: m.total_ms,
            tokens_per_sec: m.tokens_per_sec,
            reasoning_ms: m.reasoning_ms,
            usage: UsageView {
                cached_in: m.usage.cached_in,
                uncached_in: m.usage.uncached_in,
                reasoning: m.usage.reasoning,
                output: m.usage.output,
            },
        }
    }
}

/// A message node exposes only the public role snapshot and token/time metrics.
#[derive(Serialize)]
pub struct NodeView<'a> {
    id: Cow<'a, str>,
    parent_id: Option<Cow<'a, str>>,
    role: NodeRole,
    content: Cow<'a, str>,
    attachments: Cow<'a, [nekochat_store::Attachment]>,
    reasoning: Option<Cow<'a, str>>,
    character_id: Option<Cow<'a, str>>,
    character_name: Option<Cow<'a, str>>,
    character_avatar: Option<Cow<'a, str>>,
    status: NodeStatus,
    metrics: Option<MetricsView>,
    created_at: i64,
}

fn public_node_status(node: &Node) -> NodeStatus {
    if node.role == NodeRole::Assistant && node.status == NodeStatus::Complete && node.content.trim().is_empty() {
        NodeStatus::Error
    } else {
        node.status.clone()
    }
}

impl From<Node> for NodeView<'static> {
    fn from(n: Node) -> Self {
        let status = public_node_status(&n);
        Self {
            id: Cow::Owned(n.id),
            parent_id: n.parent_id.map(Cow::Owned),
            role: n.role,
            content: Cow::Owned(n.content),
            attachments: Cow::Owned(n.attachments),
            reasoning: n.reasoning.map(Cow::Owned),
            character_id: n.character_id.map(Cow::Owned),
            character_name: n.character_name.map(Cow::Owned),
            character_avatar: n.character_avatar.map(Cow::Owned),
            status,
            metrics: n.metrics.as_ref().map(MetricsView::from),
            created_at: n.created_at,
        }
    }
}

impl<'a> From<&'a Node> for NodeView<'a> {
    fn from(n: &'a Node) -> Self {
        Self {
            id: Cow::Borrowed(&n.id),
            parent_id: n.parent_id.as_deref().map(Cow::Borrowed),
            role: n.role.clone(),
            content: Cow::Borrowed(&n.content),
            attachments: Cow::Borrowed(&n.attachments),
            reasoning: n.reasoning.as_deref().map(Cow::Borrowed),
            character_id: n.character_id.as_deref().map(Cow::Borrowed),
            character_name: n.character_name.as_deref().map(Cow::Borrowed),
            character_avatar: n.character_avatar.as_deref().map(Cow::Borrowed),
            status: public_node_status(n),
            metrics: n.metrics.as_ref().map(MetricsView::from),
            created_at: n.created_at,
        }
    }
}

#[derive(Serialize)]
pub struct ConversationView {
    id: String,
    title: String,
    character_id: Option<String>,
    leaf_id: Option<String>,
    nodes: Vec<NodeView<'static>>,
    active_turn_id: Option<String>,
}

/// `GET /api/conversations/{id}` with every node; the client derives paths and siblings.
pub async fn get_conversation(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<Json<ConversationView>, ApiError> {
    require_conversation(&state, &user, &id).await?;
    let active_turn_id = state
        .generation(&id)
        .map(|generation| generation.id.clone());
    let store = state.store.clone();
    let (conv, nodes) = blocking(move || {
        let conv = store.get_conversation(&id)?.ok_or(StoreError::NotFound)?;
        let nodes = store.nodes(&id)?;
        Ok((conv, nodes))
    })
    .await?;
    Ok(Json(ConversationView {
        id: conv.id,
        title: conv.title,
        character_id: conv.character_id,
        leaf_id: conv.leaf_id,
        nodes: nodes.into_iter().map(Into::into).collect(),
        active_turn_id,
    }))
}

/// `DELETE /api/conversations/{id}`; a running turn is cancelled first.
pub async fn delete_conversation(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    require_conversation(&state, &user, &id).await?;
    let deletion = state.begin_deletion(&id);
    let store = state.store.clone();
    let id = blocking(move || {
        store.delete_conversation(&id)?;
        Ok(id)
    }).await?;
    drop(deletion);
    state.account_events.publish(&user.id, &id);
    Ok(StatusCode::NO_CONTENT)
}

/// Clearing history is not an implicit stop action.
pub async fn clear_conversation(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    require_conversation(&state, &user, &id).await?;
    let (guard, _) = state
        .begin_turn(&id)
        .ok_or_else(|| ApiError::new(StatusCode::CONFLICT, "请先停止生成，再清空对话"))?;
    let store = state.store.clone();
    let id = blocking(move || {
        store.clear_conversation(&id)?;
        Ok(id)
    }).await?;
    drop(guard);
    state.account_events.publish(&user.id, &id);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct LeafRequest {
    node_id: String,
    descend: bool,
}

#[derive(Serialize)]
pub struct LeafView {
    leaf_id: String,
}

/// `PUT /api/conversations/{id}/leaf`; `descend` follows the newest children down. Refused while
/// a turn runs there: the turn would move the leaf back when it saves its reply.
pub async fn set_leaf(
    State(state): State<AppState>,
    Extension(user): Extension<CurrentUser>,
    Path(id): Path<String>,
    JsonBody(req): JsonBody<LeafRequest>,
) -> Result<Json<LeafView>, ApiError> {
    require_conversation(&state, &user, &id).await?;
    if state.turn_running(&id) {
        return Err(ApiError::new(StatusCode::CONFLICT, "该对话正在生成回复"));
    }
    let store = state.store.clone();
    let (id, leaf_id) = blocking(move || {
        let leaf = if req.descend {
            store.newest_leaf_under(&id, &req.node_id)?
        } else {
            req.node_id
        };
        store.set_leaf(&id, Some(&leaf))?;
        Ok((id, leaf))
    })
    .await?;
    state.account_events.publish(&user.id, &id);
    Ok(Json(LeafView { leaf_id }))
}
