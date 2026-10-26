//! User-facing conversation API.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use nekochat_core::{Metrics, ReasoningStyle};
use nekochat_store::{Conversation, Node, NodeRole, NodeStatus, SettingKey, StoreError};
use serde::{Deserialize, Serialize};

use crate::api::{ApiError, blocking};
use crate::state::AppState;

#[derive(Serialize)]
pub struct ConversationSummary {
    id: String,
    title: String,
    updated_at: i64,
}

impl From<Conversation> for ConversationSummary {
    fn from(c: Conversation) -> ConversationSummary {
        ConversationSummary {
            id: c.id,
            title: c.title,
            updated_at: c.updated_at,
        }
    }
}

#[derive(Serialize)]
pub struct PickerReasoning {
    levels: Vec<String>,
    default: Option<String>,
}

#[derive(Serialize)]
pub struct PickerModel {
    id: String,
    display_name: String,
    provider_label: String,
    label: String,
    reasoning: PickerReasoning,
    vision: bool,
}

#[derive(Serialize)]
pub struct ModelsView {
    default_model_id: Option<String>,
    models: Vec<PickerModel>,
}

/// `GET /api/models`: listed models whose provider is configured; no prices or upstream ids.
pub async fn list_models(State(state): State<AppState>) -> Result<Json<ModelsView>, ApiError> {
    let store = state.store.clone();
    let (default_model_id, listed) = blocking(move || {
        Ok((
            store.get_setting(SettingKey::DefaultModel)?,
            store.list_models()?,
        ))
    })
    .await?;
    let models = listed
        .into_iter()
        .filter_map(|m| {
            let provider_label = state.provider(&m.spec.provider)?.label.clone();
            let spec = m.spec;
            let levels = match spec.reasoning.style {
                ReasoningStyle::None => Vec::new(),
                _ => spec.reasoning.levels,
            };
            Some(PickerModel {
                id: m.id,
                label: format!("{} · {}", spec.display_name, provider_label),
                display_name: spec.display_name,
                provider_label,
                reasoning: PickerReasoning {
                    levels,
                    default: spec.reasoning.default,
                },
                vision: spec.vision,
            })
        })
        .collect();
    Ok(Json(ModelsView {
        default_model_id,
        models,
    }))
}

/// `GET /api/conversations`, newest first.
pub async fn list_conversations(
    State(state): State<AppState>,
) -> Result<Json<Vec<ConversationSummary>>, ApiError> {
    let store = state.store.clone();
    let convs = blocking(move || store.list_conversations()).await?;
    Ok(Json(convs.into_iter().map(Into::into).collect()))
}

/// `POST /api/conversations` with the default character, if any.
pub async fn create_conversation(
    State(state): State<AppState>,
) -> Result<(StatusCode, Json<ConversationSummary>), ApiError> {
    let store = state.store.clone();
    let conv = blocking(move || {
        let character = store.get_setting(SettingKey::DefaultCharacter)?;
        store.create_conversation(character.as_deref())
    })
    .await?;
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
    usage: UsageView,
}

impl From<Metrics> for MetricsView {
    fn from(m: Metrics) -> MetricsView {
        MetricsView {
            ttft_ms: m.ttft_ms,
            total_ms: m.total_ms,
            tokens_per_sec: m.tokens_per_sec,
            usage: UsageView {
                cached_in: m.usage.cached_in,
                uncached_in: m.usage.uncached_in,
                reasoning: m.usage.reasoning,
                output: m.usage.output,
            },
        }
    }
}

/// A message node as the client sees it: no character data, no prices.
#[derive(Serialize)]
pub struct NodeView {
    id: String,
    parent_id: Option<String>,
    role: NodeRole,
    content: String,
    reasoning: Option<String>,
    model_label: Option<String>,
    effort: Option<String>,
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
            model_label: n.model_label,
            effort: n.effort,
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
        leaf_id: conv.leaf_id,
        nodes: nodes.into_iter().map(Into::into).collect(),
    }))
}

/// `DELETE /api/conversations/{id}`.
pub async fn delete_conversation(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let store = state.store.clone();
    blocking(move || store.delete_conversation(&id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/conversations/{id}/clear`.
pub async fn clear_conversation(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
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

/// `PUT /api/conversations/{id}/leaf`; `descend` follows the newest children down.
pub async fn set_leaf(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<LeafRequest>,
) -> Result<Json<LeafView>, ApiError> {
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
