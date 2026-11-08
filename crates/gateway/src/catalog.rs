//! Bounded discovery of the metadata a provider actually discloses.

use std::{collections::HashSet, sync::LazyLock, time::Duration};

use nekochat_core::{Price, ReasoningSpec, ReasoningStyle};
use serde_json::Value;

use crate::{GatewayError, Provider, USER_AGENT, http::endpoint};

const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

/// Candidate metadata, with absent fields left unknown. Prices are USD per million tokens.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelCatalogEntry {
    pub id: String,
    pub name: Option<String>,
    pub context_window: Option<i64>,
    pub vision: Option<bool>,
    pub reasoning: Option<ReasoningSpec>,
    pub price: Price,
    pub price_source: Option<String>,
}

/// GETs `models_url`, or `{base_url}/models`, with the provider's bearer credential.
/// Discovery uses an isolated client because a caller's redirect policy cannot be
/// overridden per request. Even same-origin redirects must not receive credentials.
/// The client retains reqwest's default environment proxy configuration.
pub async fn list_models(
    _http: &reqwest::Client,
    provider: &Provider,
) -> Result<Vec<ModelCatalogEntry>, GatewayError> {
    static CLIENT: LazyLock<Result<reqwest::Client, reqwest::Error>> = LazyLock::new(|| reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build());
    let client = CLIENT.as_ref().map_err(|_| transport_error())?;
    let raw_url = provider.models_url.as_deref().map(str::to_owned)
        .unwrap_or_else(|| endpoint(&provider.base_url, "/models"));
    let mut url = reqwest::Url::parse(&raw_url)
        .map_err(|_| GatewayError::Protocol("invalid model catalog URL".into()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(GatewayError::Protocol("model catalog URL must use HTTP or HTTPS".into()));
    }
    // Userinfo is neither a metadata source nor an alternate authentication channel.
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_fragment(None);
    let mut source = url.clone();
    source.set_query(None);
    let mut request = client.get(url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header(reqwest::header::ACCEPT, "application/json");
    if !provider.api_key.is_empty() {
        request = request.bearer_auth(&provider.api_key);
    }
    let mut response = request.send().await.map_err(|_| transport_error())?;
    if !response.status().is_success() {
        return Err(GatewayError::Http {
            status: response.status().as_u16(),
            message: "model catalog request failed".into(),
        });
    }
    if response.content_length().is_some_and(|length| length > MAX_RESPONSE_BYTES as u64) {
        return Err(oversized());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| transport_error())? {
        if chunk.len() > MAX_RESPONSE_BYTES - body.len() {
            return Err(oversized());
        }
        body.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&body)
        .map_err(|_| GatewayError::Protocol("invalid model catalog JSON".into()))?;
    parse_listing(&value, source.as_str())
}

fn transport_error() -> GatewayError {
    // reqwest errors may contain query credentials; never expose the raw error or body.
    GatewayError::Transport("model catalog request failed".into())
}

fn oversized() -> GatewayError {
    GatewayError::Protocol("model catalog response too large".into())
}

fn label(value: &Value) -> Option<&str> {
    value.as_str().filter(|text| !text.trim().is_empty())
}

fn capacity(value: &Value) -> Option<i64> {
    value.as_i64().filter(|number| *number > 0).or_else(|| {
        let number = value.as_f64()?;
        // i64::MAX rounds up in f64, hence the exclusive upper bound.
        (number.is_finite() && number > 0.0 && number.fract() == 0.0 && number < i64::MAX as f64)
            .then_some(number as i64)
    })
}

fn parse_listing<'a>(value: &'a Value, source: &str) -> Result<Vec<ModelCatalogEntry>, GatewayError> {
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut adopt = |key: Option<&'a str>, raw: &'a Value| {
        if !raw.is_object() { return; }
        let Some(id) = key.filter(|key| !key.trim().is_empty()).or_else(|| label(&raw["id"])) else { return; };
        if !seen.insert(id) { return; }
        let price = pricing(raw);
        let has_price = price.cached_in.is_some() || price.uncached_in.is_some() || price.output.is_some();
        entries.push(ModelCatalogEntry {
            id: id.to_owned(),
            name: ["name", "display_name", "displayName"].into_iter()
                .find_map(|field| label(&raw[field])).map(str::to_owned),
            context_window: ["/contextWindow", "/context_window", "/context_length", "/max_input_tokens", "/limit/context"]
                .into_iter().find_map(|path| raw.pointer(path).and_then(capacity)),
            vision: vision(raw),
            reasoning: reasoning(raw),
            price,
            price_source: has_price.then(|| source.to_owned()),
        });
    };
    if let Some(data) = value.get("data").and_then(Value::as_array) {
        for raw in data { adopt(None, raw); }
    } else if let Some(models) = value.get("models").and_then(Value::as_object) {
        for (key, raw) in models { adopt(Some(key), raw); }
    } else {
        return Err(GatewayError::Protocol("models response lacks data[] or models object".into()));
    }
    Ok(entries)
}

fn vision(raw: &Value) -> Option<bool> {
    for path in ["/vision", "/supports_vision", "/capabilities/vision"] {
        if let Some(disclosed) = raw.pointer(path).and_then(Value::as_bool) { return Some(disclosed); }
    }
    for path in ["/input_modalities", "/inputModalities", "/architecture/input_modalities", "/modalities/input", "/input"] {
        let Some(modalities) = raw.pointer(path).and_then(Value::as_array) else { continue; };
        if modalities.is_empty() || modalities.iter().any(|value| label(value).is_none()) { continue; }
        return Some(modalities.iter().filter_map(Value::as_str).any(|modality| modality == "image" || modality.starts_with("image/")));
    }
    None
}

fn no_reasoning() -> ReasoningSpec {
    ReasoningSpec { style: ReasoningStyle::None, levels: vec![], default: None }
}

fn reasoning(raw: &Value) -> Option<ReasoningSpec> {
    if raw.get("reasoning").and_then(Value::as_bool) == Some(false)
        || raw.get("supports_reasoning").and_then(Value::as_bool) == Some(false) {
        return Some(no_reasoning());
    }
    let disclosed = raw.get("reasoning").filter(|value| value.is_object());
    let style = disclosed.and_then(|value| value.get("style")).and_then(Value::as_str)
        .or_else(|| raw.get("reasoning_style").and_then(Value::as_str));
    if style == Some("none") { return Some(no_reasoning()); }
    let style = match style {
        Some("effort") => ReasoningStyle::Effort,
        Some("deepseek") => ReasoningStyle::DeepSeek,
        Some(_) => return None,
        None => {
            // Effort levels identify the wire parameter; a generic reasoning flag does not.
            if raw.get("reasoning_effort").and_then(Value::as_array).is_none()
                && raw.get("reasoning_effort_levels").and_then(Value::as_array).is_none()
                && raw.pointer("/capabilities/reasoning/effort").and_then(Value::as_array).is_none()
                && raw.pointer("/effort/supported_levels").and_then(Value::as_array).is_none() {
                return None;
            }
            ReasoningStyle::Effort
        }
    };
    let values = disclosed.and_then(|value| value.get("levels")).and_then(Value::as_array)
        .or_else(|| raw.get("reasoning_effort_levels").and_then(Value::as_array))
        .or_else(|| raw.get("reasoning_effort").and_then(Value::as_array))
        .or_else(|| raw.pointer("/capabilities/reasoning/effort").and_then(Value::as_array))
        .or_else(|| raw.pointer("/effort/supported_levels").and_then(Value::as_array))?;
    let mut levels: Vec<String> = Vec::new();
    for level in values.iter().filter_map(label) {
        if !levels.iter().any(|existing| existing == level) { levels.push(level.to_owned()); }
    }
    if levels.is_empty() { return None; }
    let default = disclosed.and_then(|value| value.get("default")).and_then(label)
        .or_else(|| raw.get("default_reasoning_effort").and_then(label))
        .or_else(|| raw.pointer("/effort/default_level").and_then(label))
        .filter(|default| levels.iter().any(|level| level.as_str() == *default)).map(str::to_owned);
    Some(ReasoningSpec { style, levels, default })
}

fn unknown_price() -> Price {
    Price { cached_in: None, uncached_in: None, output: None }
}

fn usd_currency(value: &Value) -> bool {
    ["currency", "pricing_currency"].into_iter().all(|field| match value.get(field) {
        None => true,
        Some(currency) => currency.as_str().is_some_and(|currency| currency.eq_ignore_ascii_case("USD")),
    })
}

fn known_units(value: &Value, per_token: bool) -> bool {
    ["unit", "units", "pricing_unit"].into_iter().all(|field| match value.get(field) {
        None => true,
        Some(unit) => unit.as_str().is_some_and(|unit| {
            if per_token {
                matches!(unit, "token" | "per_token" | "USD/token" | "usd/token")
            } else {
                matches!(unit, "million_tokens" | "per_million_tokens" | "USD/1M_tokens" | "usd/1m_tokens")
            }
        }),
    })
}

fn rate(value: &Value, multiplier: f64) -> Option<f64> {
    let number = value.as_f64().or_else(|| value.as_str()?.parse::<f64>().ok())?;
    let converted = number * multiplier;
    (number.is_finite() && number >= 0.0 && converted.is_finite()).then_some(converted)
}

fn prices(value: &Value, fields: [&str; 3], multiplier: f64) -> Price {
    Price {
        cached_in: rate(&value[fields[0]], multiplier),
        uncached_in: rate(&value[fields[1]], multiplier),
        output: rate(&value[fields[2]], multiplier),
    }
}

fn pricing(raw: &Value) -> Price {
    if !usd_currency(raw) { return unknown_price(); }
    // OpenRouter's pricing fields are contractually USD per token. An explicit
    // currency/unit still takes precedence over that schema's implicit units.
    if let Some(pricing) = raw.get("pricing").filter(|value| value.is_object()) {
        if ["prompt", "completion", "input_cache_read"].into_iter().any(|field| pricing.get(field).is_some()) {
            return if usd_currency(pricing) && known_units(pricing, true) && known_units(raw, true) {
                prices(pricing, ["input_cache_read", "prompt", "completion"], 1_000_000.0)
            } else { unknown_price() };
        }
    }
    // LiteLLM exposes these USD/token fields directly or under model_info.
    for value in [Some(raw), raw.get("model_info")].into_iter().flatten() {
        if ["input_cost_per_token", "output_cost_per_token", "cache_read_input_token_cost"]
            .into_iter().any(|field| value.get(field).is_some()) {
            return if usd_currency(value) && known_units(value, true) && known_units(raw, true) {
                prices(value, ["cache_read_input_token_cost", "input_cost_per_token", "output_cost_per_token"], 1_000_000.0)
            } else { unknown_price() };
        }
    }
    // models.dev/OpenCode use cost + limit together; arbitrary cost.input fields
    // alone carry no recognized currency/unit contract and must stay unknown.
    let directory = raw.pointer("/limit/context").and_then(capacity).is_some()
        || raw.pointer("/limit/output").and_then(capacity).is_some();
    if let Some(cost) = raw.get("cost").filter(|value| value.is_object() && directory) {
        if usd_currency(cost) && known_units(cost, false) && known_units(raw, false) {
            if let Some(text) = cost.get("text").filter(|value| value.is_object()) {
                let input = &text["input"];
                let output = &text["output"];
                // Tiered rates and cache writes cannot be represented by the three stored rates.
                let variable = [input, output].into_iter().any(|value| value.get("overrides").is_some_and(|overrides| overrides.as_array().is_none_or(|rows| !rows.is_empty())));
                let writes = input.pointer("/cache/write").and_then(|value| rate(value, 1.0));
                if variable || writes.is_some_and(|write| Some(write) != rate(&input["price"], 1.0)) {
                    return unknown_price();
                }
                if !usd_currency(text) || !usd_currency(input) || !usd_currency(output)
                    || !known_units(text, false) || !known_units(input, false) || !known_units(output, false) {
                    return unknown_price();
                }
                return Price {
                    cached_in: input.pointer("/cache/read").and_then(|value| rate(value, 1.0)),
                    uncached_in: rate(&input["price"], 1.0),
                    output: rate(&output["price"], 1.0),
                };
            }
            return prices(cost, ["cache_read", "input", "output"], 1.0);
        }
    }
    unknown_price()
}
