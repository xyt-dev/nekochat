//! Admin API: providers, listed models, characters and settings.

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use nekochat_core::{ReasoningSpec, ReasoningStyle};
use nekochat_store::{Character, CharacterSpec, ListedModel, ModelSpec, SettingKey};
use serde::{Deserialize, Serialize};

use crate::api::{ApiError, JsonBody, blocking};
use crate::model_defaults::{self, ModelDefaults};
use crate::state::AppState;

#[derive(Serialize)]
pub struct ProviderSummary {
    key: String,
    label: String,
}

pub async fn list_providers(State(state): State<AppState>) -> Json<Vec<ProviderSummary>> {
    Json(
        state
            .providers
            .iter()
            .map(|p| ProviderSummary {
                key: p.key.clone(),
                label: p.label.clone(),
            })
            .collect(),
    )
}

#[derive(Serialize)]
pub struct UpstreamModel {
    upstream_id: String,
    listed_model_id: Option<String>,
    supported: bool,
    defaults: Option<ModelDefaults>,
}

pub async fn provider_models(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<Json<Vec<UpstreamModel>>, ApiError> {
    let provider = state.provider(&key).ok_or_else(ApiError::not_found)?;
    let upstream = nekochat_gateway::list_models(&state.http, provider)
        .await
        .map_err(|e| {
            tracing::warn!(provider = %key, "listing upstream models failed: {}", match e {
                nekochat_gateway::GatewayError::Http { status, .. } => format!("HTTP {status}"),
                _ => "transport or protocol error".into(),
            });
            ApiError::new(StatusCode::BAD_GATEWAY, "获取上游模型列表失败")
        })?;
    let store = state.store.clone();
    let listed: HashMap<String, String> = blocking(move || store.list_models())
        .await?
        .into_iter()
        .filter(|m| m.spec.provider == key)
        .map(|m| (m.spec.upstream_id, m.id))
        .collect();
    Ok(Json(
        upstream
            .into_iter()
            .map(|upstream_id| UpstreamModel {
                listed_model_id: listed.get(&upstream_id).cloned(),
                supported: model_defaults::supported(provider, &upstream_id),
                defaults: model_defaults::defaults(provider, &upstream_id),
                upstream_id,
            })
            .collect(),
    ))
}

pub async fn list_models(
    State(state): State<AppState>,
) -> Result<Json<Vec<ListedModel>>, ApiError> {
    let store = state.store.clone();
    Ok(Json(blocking(move || store.list_models()).await?))
}

fn check_spec(state: &AppState, spec: &ModelSpec) -> Result<(), ApiError> {
    let provider = state
        .provider(&spec.provider)
        .ok_or_else(|| ApiError::bad_request("提供商不在配置中"))?;
    if !model_defaults::supported(provider, &spec.upstream_id) {
        return Err(ApiError::bad_request("此模型需要尚未接入的上游协议"));
    }
    if spec.upstream_id.trim().is_empty() || spec.display_name.trim().is_empty() {
        return Err(ApiError::bad_request("模型ID和显示名称不能为空"));
    }
    if spec.context_window.is_some_and(|v| v <= 0) {
        return Err(ApiError::bad_request("上下文长度必须是正整数"));
    }
    if [
        &spec.price.cached_in,
        &spec.price.uncached_in,
        &spec.price.output,
    ]
    .into_iter()
    .flatten()
    .any(|v| !v.is_finite() || *v < 0.0)
    {
        return Err(ApiError::bad_request("价格必须是有限非负数或空值"));
    }
    check_reasoning(&spec.reasoning)
}

/// A model without reasoning has no levels; one with reasoning has distinct levels and a default
/// that is one of them (or none).
fn check_reasoning(r: &ReasoningSpec) -> Result<(), ApiError> {
    if r.style == ReasoningStyle::None {
        if !r.levels.is_empty() || r.default.is_some() {
            return Err(ApiError::bad_request("不支持思考的模型不能设置思考档位"));
        }
        return Ok(());
    }
    if r.levels.is_empty() {
        return Err(ApiError::bad_request("思考档位不能为空"));
    }
    if r.levels
        .iter()
        .enumerate()
        .any(|(i, l)| r.levels[..i].contains(l))
    {
        return Err(ApiError::bad_request("思考档位不能重复"));
    }
    if r.default.as_ref().is_some_and(|d| !r.levels.contains(d)) {
        return Err(ApiError::bad_request("默认思考档位不在档位列表里"));
    }
    Ok(())
}

pub async fn create_model(
    State(state): State<AppState>,
    JsonBody(spec): JsonBody<ModelSpec>,
) -> Result<(StatusCode, Json<ListedModel>), ApiError> {
    check_spec(&state, &spec)?;
    let store = state.store.clone();
    let model = blocking(move || store.insert_model(&spec)).await?;
    Ok((StatusCode::CREATED, Json(model)))
}

pub async fn update_model(
    State(state): State<AppState>,
    Path(id): Path<String>,
    JsonBody(spec): JsonBody<ModelSpec>,
) -> Result<Json<ListedModel>, ApiError> {
    check_spec(&state, &spec)?;
    let store = state.store.clone();
    Ok(Json(
        blocking(move || store.update_model(&id, &spec)).await?,
    ))
}

pub async fn delete_model(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let store = state.store.clone();
    blocking(move || store.delete_model(&id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_characters(
    State(state): State<AppState>,
) -> Result<Json<Vec<Character>>, ApiError> {
    let store = state.store.clone();
    Ok(Json(blocking(move || store.list_characters()).await?))
}

pub async fn create_character(
    State(state): State<AppState>,
    JsonBody(body): JsonBody<CharacterSpec>,
) -> Result<(StatusCode, Json<Character>), ApiError> {
    check_character(&state, &body).await?;
    let store = state.store.clone();
    let c = blocking(move || store.create_character(&body)).await?;
    Ok((StatusCode::CREATED, Json(c)))
}

pub async fn update_character(
    State(state): State<AppState>,
    Path(id): Path<String>,
    JsonBody(body): JsonBody<CharacterSpec>,
) -> Result<Json<Character>, ApiError> {
    check_character(&state, &body).await?;
    let store = state.store.clone();
    Ok(Json(
        blocking(move || store.update_character(&id, &body)).await?,
    ))
}

pub async fn delete_character(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let store = state.store.clone();
    blocking(move || store.delete_character(&id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn check_character(state: &AppState, body: &CharacterSpec) -> Result<(), ApiError> {
    if body.name.trim().is_empty() || body.display_name.trim().is_empty() {
        return Err(ApiError::bad_request("内部名称和公开名称不能为空"));
    }
    body.parameters.validate().map_err(ApiError::bad_request)?;
    if !["self", "neutral"].contains(&body.sticker_identity.as_str())
        || !["rare", "normal", "often"].contains(&body.sticker_frequency.as_str())
    {
        return Err(ApiError::bad_request("资源身份或频率无效"));
    }
    if body
        .resource_packs
        .iter()
        .any(|id| !state.resources.contains_pack(id))
    {
        return Err(ApiError::bad_request("资源包不存在"));
    }
    if let Some(background) = &body.background {
        if !background.opacity.is_finite()
            || !(0.0..=1.0).contains(&background.opacity)
            || !crate::assets::background_url_allowed(state, &background.url).await
        {
            return Err(ApiError::bad_request(
                "背景必须是存在的本站图片，透明度为 0..1",
            ));
        }
    }
    if let Some(id) = &body.model_id {
        let store = state.store.clone();
        let id = id.clone();
        let model = blocking(move || store.get_model(&id))
            .await?
            .ok_or_else(|| ApiError::bad_request("角色绑定模型未上架"))?;
        if state.provider(&model.spec.provider).is_none() {
            return Err(ApiError::bad_request("模型的提供商不在配置中"));
        }
        crate::chat::resolve_effort(&model.spec.reasoning, body.parameters.effort.clone())?;
    } else if body.parameters.effort.is_some() {
        return Err(ApiError::bad_request("设置思考参数前请先绑定模型"));
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    default_model_id: Option<String>,
    #[serde(default)]
    default_character_id: Option<String>,
}

pub async fn get_settings(State(state): State<AppState>) -> Result<Json<Settings>, ApiError> {
    let store = state.store.clone();
    let settings = blocking(move || {
        Ok(Settings {
            default_model_id: store.get_setting(SettingKey::DefaultModel)?,
            default_character_id: store.get_setting(SettingKey::DefaultCharacter)?,
        })
    })
    .await?;
    Ok(Json(settings))
}

pub async fn put_settings(
    State(state): State<AppState>,
    JsonBody(body): JsonBody<Settings>,
) -> Result<Json<Settings>, ApiError> {
    let store = state.store.clone();
    let dangling = blocking(move || {
        let model_ok = match &body.default_model_id {
            Some(id) => store.get_model(id)?.is_some(),
            None => true,
        };
        let character_ok = match &body.default_character_id {
            Some(id) => store.get_character(id)?.is_some(),
            None => true,
        };
        if !(model_ok && character_ok) {
            return Ok(Err(body));
        }
        store.set_setting(SettingKey::DefaultModel, body.default_model_id.as_deref())?;
        store.set_setting(
            SettingKey::DefaultCharacter,
            body.default_character_id.as_deref(),
        )?;
        Ok(Ok(body))
    })
    .await?;
    match dangling {
        Ok(settings) => Ok(Json(settings)),
        Err(_) => Err(ApiError::bad_request("默认模型或默认角色不存在")),
    }
}
