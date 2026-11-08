//! Conservative official capabilities plus metadata disclosed by the configured provider.
//! Prices remain unknown unless the provider's catalog supplies recognized USD rates.
use nekochat_core::{Price, Protocol, ReasoningSpec, ReasoningStyle};
use nekochat_gateway::{ModelCatalogEntry, Provider};
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
    !(id.starts_with("claude-")
        || id.starts_with("gemini-")
        || id.starts_with("grok-")
        || id.starts_with("muse-")
        || id.starts_with("jev-")
        || (id.starts_with("qwen") && id != "qwen3.8-max"))
}

#[derive(Serialize)]
pub struct ModelDefaults {
    pub(crate) display_name: String,
    pub(crate) protocol: Protocol,
    pub(crate) reasoning: ReasoningSpec,
    pub(crate) vision: bool,
    pub(crate) context_window: Option<i64>,
    pub(crate) price: Price,
}

fn conservative(id: &str) -> ModelDefaults {
    ModelDefaults {
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
    }
}

/// Merge only disclosed catalog fields; absent relay metadata stays conservative.
/// Callers must still enforce `supported` before offering official protocol routes.
pub fn from_catalog(provider: &Provider, entry: &ModelCatalogEntry) -> ModelDefaults {
    let mut result = defaults(provider, &entry.id).unwrap_or_else(|| conservative(&entry.id));
    if let Some(name) = &entry.name {
        result.display_name = name.clone();
    }
    if let Some(context_window) = entry.context_window {
        result.context_window = Some(context_window);
    }
    if let Some(vision) = entry.vision {
        result.vision = vision;
    }
    if let Some(reasoning) = &entry.reasoning {
        let native_deepseek = result.reasoning.style == ReasoningStyle::DeepSeek;
        result.reasoning = reasoning.clone();
        if native_deepseek && reasoning.style == ReasoningStyle::Effort {
            result.reasoning.style = ReasoningStyle::DeepSeek;
        }
    }
    result.price = entry.price.clone();
    result
}

pub fn defaults(provider: &Provider, id: &str) -> Option<ModelDefaults> {
    let mut result = conservative(id);
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
    if id.starts_with("gpt-") {
        result.protocol = Protocol::Responses;
        if id == "gpt-6-luna" {
            result.display_name = "GPT 6 Luna".into();
            result.vision = true;
            result.reasoning = ReasoningSpec {
                style: ReasoningStyle::Effort,
                levels: ["low", "medium", "high"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                default: Some("high".into()),
            };
        }
        return Some(result);
    }
    result.display_name = match id {
        "glm-5.3-flash" => "GLM 5.3 Flash",
        "glm-5.3" => "GLM 5.3",
        "glm-5.2" => "GLM 5.2",
        "glm-5.1" => "GLM 5.1",
        "glm-5" => "GLM 5",
        "minimax-m3" => "MiniMax M3",
        "minimax-m2.7" => "MiniMax M2.7",
        "minimax-m2.5" => "MiniMax M2.5",
        "kimi-k2.7-code" => "Kimi K2.7 Code",
        "kimi-k2.6" => "Kimi K2.6",
        "kimi-k2.5" => "Kimi K2.5",
        "kimi-k3" => "Kimi K3",
        "deepseek-v4.1-flash" => "DeepSeek V4.1 Flash",
        "deepseek-v4-pro" => "DeepSeek V4 Pro",
        "qwen3.8-max" => "Qwen3.8 Max",
        _ => return None,
    }.into();
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

#[cfg(test)]
mod tests {
    use super::*;
    use nekochat_gateway::ModelCatalogEntry;

    fn provider(base: &str) -> Provider {
        Provider {
            key: "custom".into(), label: "Custom".into(), base_url: base.into(),
            models_url: None, api_key: String::new(), session_header: None,
        }
    }

    fn entry(id: &str) -> ModelCatalogEntry {
        ModelCatalogEntry {
            id: id.into(), name: None, context_window: None, vision: None, reasoning: None,
            price: Price { cached_in: None, uncached_in: None, output: None },
            price_source: None,
        }
    }

    #[test]
    fn relay_names_do_not_inherit_official_capabilities_or_prices() {
        let relay = provider("https://relay.example/v1");
        assert!(defaults(&relay, "gpt-6-luna").is_none());
        let result = from_catalog(&relay, &entry("gpt-6-luna"));
        assert_eq!(result.protocol, Protocol::ChatCompletions);
        assert_eq!(result.reasoning.style, ReasoningStyle::None);
        assert!(!result.vision);
        assert!(result.context_window.is_none());
        assert!(result.price.uncached_in.is_none());
        assert!(result.price.output.is_none());
    }

    #[test]
    fn disclosed_values_override_defaults_including_false_and_zero() {
        let provider = provider("https://api.deepseek.com/v1");
        let mut entry = entry("deepseek-flash");
        entry.name = Some("Disclosed name".into());
        entry.context_window = Some(8192);
        entry.vision = Some(false);
        entry.reasoning = Some(ReasoningSpec {
            style: ReasoningStyle::None, levels: vec![], default: None,
        });
        entry.price.uncached_in = Some(0.0);
        let result = from_catalog(&provider, &entry);
        assert_eq!(result.display_name, "Disclosed name");
        assert_eq!(result.context_window, Some(8192));
        assert!(!result.vision);
        assert_eq!(result.reasoning.style, ReasoningStyle::None);
        assert_eq!(result.price.uncached_in, Some(0.0));
        assert!(result.price.cached_in.is_none());
        assert!(result.price.output.is_none());
    }

    #[test]
    fn official_protocol_rules_and_unknown_prices_are_preserved() {
        let official = provider("https://opencode.ai/zen/v1");
        assert!(!supported(&official, "claude-haiku-5-5"));
        assert!(defaults(&official, "claude-haiku-5-5").is_none());
        let gpt = from_catalog(&official, &entry("gpt-6-luna"));
        assert_eq!(gpt.protocol, Protocol::Responses);
        assert!(gpt.vision);
        for id in ["glm-5.3", "kimi-k3", "minimax-m3", "deepseek-v4-pro"] {
            let result = defaults(&official, id).unwrap();
            assert!(result.price.cached_in.is_none());
            assert!(result.price.uncached_in.is_none());
            assert!(result.price.output.is_none());
        }
        assert!(supported(&provider("https://relay.example/v1"), "claude-haiku-5-5"));
    }
}
