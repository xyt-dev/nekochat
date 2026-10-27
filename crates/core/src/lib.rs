//! Shared domain types for nekochat.

use serde::{Deserialize, Serialize};

/// Administrator-controlled, typed generation settings. Never a raw upstream-body override.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct GenerationParameters {
    pub effort: Option<String>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_output_tokens: Option<u32>,
    pub seed: Option<i64>,
    pub frequency_penalty: Option<f64>,
    pub presence_penalty: Option<f64>,
    #[serde(default)]
    pub stop: Vec<String>,
}

impl GenerationParameters {
    pub fn validate(&self) -> Result<(), &'static str> {
        for (value, min, max) in [
            (self.temperature, 0.0, 2.0),
            (self.top_p, 0.0, 1.0),
            (self.frequency_penalty, -2.0, 2.0),
            (self.presence_penalty, -2.0, 2.0),
        ] {
            if value.is_some_and(|v| !v.is_finite() || v < min || v > max) {
                return Err("生成参数超出有效范围");
            }
        }
        if self.max_output_tokens == Some(0) {
            return Err("最大输出 token 必须大于零");
        }
        if self.stop.len() > 4 || self.stop.iter().any(|s| s.is_empty()) {
            return Err("停止序列必须为至多四个非空字符串");
        }
        Ok(())
    }

    pub fn validate_for_protocol(&self, protocol: &Protocol) -> Result<(), &'static str> {
        self.validate()?;
        if protocol == &Protocol::Responses
            && (self.seed.is_some()
                || self.frequency_penalty.is_some()
                || self.presence_penalty.is_some()
                || !self.stop.is_empty())
        {
            return Err("Responses 不支持 seed、penalty 或 stop 参数，请清空这些字段");
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Background {
    pub url: String,
    #[serde(default = "default_background_opacity")]
    pub opacity: f64,
}

fn default_background_opacity() -> f64 {
    0.15
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    ChatCompletions,
    Responses,
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
