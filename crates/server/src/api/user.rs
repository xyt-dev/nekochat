//! User-facing conversation API.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use nekochat_core::{Background, Metrics};
use nekochat_store::{Conversation, Node, NodeRole, NodeStatus, SettingKey, StoreError};
use serde::{Deserialize, Serialize};

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
    background: Option<Background>,
}

#[derive(Serialize)]
pub struct CharactersView {
    default_character_id: Option<String>,
    characters: Vec<PickerCharacter>,
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
            background: c.spec.background,
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
) -> Result<Json<Vec<ConversationSummary>>, ApiError> {
    let store = state.store.clone();
    let convs = blocking(move || store.list_conversations()).await?;
    Ok(Json(convs.into_iter().map(Into::into).collect()))
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CreateConversation {
    character_id: Option<String>,
}

pub async fn create_conversation(
    State(state): State<AppState>,
    JsonBody(body): JsonBody<CreateConversation>,
) -> Result<(StatusCode, Json<ConversationSummary>), ApiError> {
    let available = available_characters(&state).await?;
    let id = body
        .character_id
        .or(available.default_character_id)
        .filter(|id| available.characters.iter().any(|c| &c.id == id))
        .ok_or_else(|| ApiError::bad_request("没有可用角色，请管理员配置并绑定已上架模型"))?;
    let store = state.store.clone();
    let conv = blocking(move || store.create_conversation(Some(&id))).await?;
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

impl From<Metrics> for MetricsView {
    fn from(m: Metrics) -> MetricsView {
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
pub struct NodeView {
    id: String,
    parent_id: Option<String>,
    role: NodeRole,
    content: String,
    reasoning: Option<String>,
    character_id: Option<String>,
    character_name: Option<String>,
    status: NodeStatus,
    metrics: Option<MetricsView>,
    created_at: i64,
}

impl From<Node> for NodeView {
    fn from(n: Node) -> NodeView {
        NodeView {
            id: n.id,
            parent_id: n.parent_id,
            role: n.role,
            content: n.content,
            reasoning: n.reasoning,
            character_id: n.character_id,
            character_name: n.character_name,
            status: n.status,
            metrics: n.metrics.map(Into::into),
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
    nodes: Vec<NodeView>,
}

/// `GET /api/conversations/{id}` with every node; the client derives paths and siblings.
pub async fn get_conversation(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ConversationView>, ApiError> {
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
    }))
}

/// `DELETE /api/conversations/{id}`; a running turn is cancelled first.
pub async fn delete_conversation(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.cancel_turn(&id);
    let store = state.store.clone();
    blocking(move || store.delete_conversation(&id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/conversations/{id}/clear`; a running turn is cancelled first.
pub async fn clear_conversation(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.cancel_turn(&id);
    let store = state.store.clone();
    blocking(move || store.clear_conversation(&id)).await?;
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
    Path(id): Path<String>,
    JsonBody(req): JsonBody<LeafRequest>,
) -> Result<Json<LeafView>, ApiError> {
    if state.turn_running(&id) {
        return Err(ApiError::new(StatusCode::CONFLICT, "该对话正在生成回复"));
    }
    let store = state.store.clone();
    let leaf_id = blocking(move || {
        let leaf = if req.descend {
            store.newest_leaf_under(&id, &req.node_id)?
        } else {
            req.node_id
        };
        store.set_leaf(&id, Some(&leaf))?;
        Ok(leaf)
    })
    .await?;
    Ok(Json(LeafView { leaf_id }))
}
