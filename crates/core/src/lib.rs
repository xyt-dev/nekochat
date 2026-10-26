//! Shared domain types for nekochat.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    ChatCompletions,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningStyle {
    None,
    Effort,
    #[serde(rename = "deepseek")]
    DeepSeek,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ReasoningSpec {
    pub style: ReasoningStyle,
    pub levels: Vec<String>,
    pub default: Option<String>,
}

/// Prices in USD per million tokens.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Price {
    pub cached_in: Option<f64>,
    pub uncached_in: Option<f64>,
    pub output: Option<f64>,
}

/// Token usage of one generation. `output` excludes reasoning tokens.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Usage {
    pub cached_in: u64,
    pub uncached_in: u64,
    pub reasoning: u64,
    pub output: u64,
    pub upstream_cost_usd: Option<f64>,
    pub allowance_usd: Option<f64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Metrics {
    pub ttft_ms: Option<u64>,
    pub total_ms: u64,
    pub tokens_per_sec: Option<f64>,
    pub usage: Usage,
    /// Thinking time: first text delta (or, without text, the last delta) minus the first
    /// reasoning delta. `None` when the model did not think.
    #[serde(default)]
    pub reasoning_ms: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasoning_style_wire_names() {
        let s = serde_json::to_string(&[
            ReasoningStyle::None,
            ReasoningStyle::Effort,
            ReasoningStyle::DeepSeek,
        ])
        .unwrap();
        assert_eq!(s, r#"["none","effort","deepseek"]"#);
        assert_eq!(
            serde_json::to_string(&Protocol::ChatCompletions).unwrap(),
            r#""chat_completions""#
        );
    }
}
