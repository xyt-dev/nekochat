//! Conservative defaults from official provider docs, verified 2026-10-27.
//! Sources: opencode.ai/v2/docs/console/{models,go}, api-docs.deepseek.com/{quick_start/pricing,guides/thinking_mode}.
use nekochat_core::{Price, Protocol, ReasoningSpec, ReasoningStyle};
use nekochat_gateway::Provider;
use serde::Serialize;

use crate::config::GoPlan;

pub fn official_host(base: &str, host: &str) -> bool {
    reqwest::Url::parse(base)
        .ok()
        .is_some_and(|url| url.host_str() == Some(host))
}

/// Protocol restrictions are provider-specific: a custom OpenAI-compatible proxy can
/// legitimately serve a model ID whose native vendor uses a different protocol.
pub fn supported(provider: &Provider, id: &str) -> bool {
    if !official_host(&provider.base_url, "opencode.ai") {
        return true;
    }
    !(id.starts_with("gpt-")
        || id.starts_with("claude-")
        || id.starts_with("gemini-")
        || id.starts_with("grok-")
        || id.starts_with("muse-")
        || id.starts_with("jev-")
        || (id.starts_with("qwen") && id != "qwen3.8-max"))
}

#[derive(Serialize)]
pub struct ModelDefaults {
    display_name: String,
    protocol: Protocol,
    reasoning: ReasoningSpec,
    vision: bool,
    context_window: Option<i64>,
    price: Price,
}

pub fn defaults(provider: &Provider, id: &str) -> Option<ModelDefaults> {
    let mut result = ModelDefaults {
        display_name: id.to_owned(),
        protocol: Protocol::ChatCompletions,
        reasoning: ReasoningSpec {
            style: ReasoningStyle::None,
            levels: vec![],
            default: None,
        },
        vision: false,
        context_window: None,
        price: Price {
            cached_in: None,
            uncached_in: None,
            output: None,
        },
    };
    if official_host(&provider.base_url, "api.deepseek.com") {
        let (name, vision) = match id {
            "deepseek-flash" => ("DeepSeek V4.1 Flash", true),
            "deepseek-v4-pro" => ("DeepSeek V4 Pro", false),
            _ => return None,
        };
        result.display_name = name.into();
        result.vision = vision;
        result.context_window = Some(1_000_000);
        result.reasoning = ReasoningSpec {
            style: ReasoningStyle::DeepSeek,
            levels: ["none", "low", "high", "max"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            default: Some("high".into()),
        };
        // Peak/off-peak prices cannot be represented by a single fixed price.
        return Some(result);
    }
    if !official_host(&provider.base_url, "opencode.ai") || !supported(provider, id) {
        return None;
    }
    let (name, price) = match id {
        "glm-5.3-flash" => ("GLM 5.3 Flash", Some((0.03, 0.15, 0.50))),
        "glm-5.3" => ("GLM 5.3", Some((0.26, 1.40, 4.40))),
        "glm-5.2" => ("GLM 5.2", Some((0.26, 1.40, 4.40))),
        "glm-5.1" => ("GLM 5.1", Some((0.26, 1.40, 4.40))),
        "glm-5" => ("GLM 5", Some((0.20, 1.00, 3.20))),
        "minimax-m3" => ("MiniMax M3", Some((0.06, 0.30, 1.20))),
        "minimax-m2.7" => ("MiniMax M2.7", Some((0.06, 0.30, 1.20))),
        "minimax-m2.5" => ("MiniMax M2.5", Some((0.06, 0.30, 1.20))),
        "kimi-k2.7-code" => ("Kimi K2.7 Code", Some((0.19, 0.95, 4.00))),
        "kimi-k2.6" => ("Kimi K2.6", Some((0.16, 0.95, 4.00))),
        "kimi-k2.5" => ("Kimi K2.5", Some((0.10, 0.60, 3.00))),
        "kimi-k3" => ("Kimi K3", Some((0.30, 3.00, 15.00))),
        "deepseek-v4.1-flash" => ("DeepSeek V4.1 Flash", None),
        "deepseek-v4-pro" => ("DeepSeek V4 Pro", None),
        // Cache writes have a distinct rate that the current Price schema cannot express.
        "qwen3.8-max" => ("Qwen3.8 Max", None),
        _ => return None,
    };
    result.display_name = name.into();
    if let Some((cached_in, uncached_in, output)) = price {
        result.price = Price {
            cached_in: Some(cached_in),
            uncached_in: Some(uncached_in),
            output: Some(output),
        };
    }
    Some(result)
}

/// Only IDs cross-checked against the public model catalog and Go pricing table.
/// Unlisted/new/retired or unlimited models stay unknown, never guessed from display names.
pub fn monthly_limit(id: &str, plan: GoPlan) -> Option<f64> {
    let (go, plus) = match id {
        "glm-5.3-flash" | "glm-5.2" | "kimi-k2.7-code" | "minimax-m3" => (60.0, 180.0),
        "glm-5.3" => (15.0, 120.0),
        "kimi-k3" | "deepseek-v4-pro" | "grok-4.7" | "gpt-6-luna" | "gpt-5.6-luna"
        | "claude-haiku-5-5" | "qwen3.8-max" => (15.0, 60.0),
        "qwen3.8-flash" => (30.0, 90.0),
        "deepseek-v4.1-flash" => (80.0, 160.0),
        "gemini-4-argon" => (30.0, 120.0),
        _ => return None,
    };
    Some(match plan {
        GoPlan::Go => go,
        GoPlan::GoPlus => plus,
    })
}
