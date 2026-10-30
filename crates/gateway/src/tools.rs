//! Bounded tool-call assembly and protocol-neutral continuation history.
use std::collections::{BTreeMap, HashSet};

use serde_json::{Value, json};

use crate::GatewayError;

pub const MAX_ARGUMENT_BYTES: usize = 8192;
const MAX_PARALLEL_CALLS: usize = 8;

#[derive(Clone, Debug, PartialEq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolCallDelta {
    pub index: usize,
    pub id: Option<String>,
    pub name: Option<String>,
    pub arguments: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolResult {
    pub call_id: String,
    pub output: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolExchange {
    pub content: String,
    pub reasoning: String,
    pub calls: Vec<ToolCall>,
    pub results: Vec<ToolResult>,
    /// Native Responses items include encrypted reasoning and must survive tool continuation.
    pub response_items: Vec<Value>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolContext {
    pub definitions: Vec<ToolDefinition>,
    pub history: Vec<ToolExchange>,
}

impl ToolContext {
    pub(crate) fn chat_definitions(&self) -> Value {
        Value::Array(self.definitions.iter().map(|tool| json!({
            "type": "function",
            "function": {
                "name": tool.name, "description": tool.description,
                "parameters": tool.parameters, "strict": true
            }
        })).collect())
    }

    pub(crate) fn response_definitions(&self, namespaced: bool) -> Value {
        let functions: Vec<_> = self.definitions.iter().map(|tool| json!({
            "type": "function", "name": tool.name, "description": tool.description,
            "parameters": tool.parameters, "strict": true
        })).collect();
        if namespaced {
            json!([{"type":"namespace", "name":"nekochat",
                "description":"Authorized public web research for this conversation.", "tools":functions}])
        } else {
            Value::Array(functions)
        }
    }

    pub(crate) fn extend_chat(&self, messages: &mut Vec<Value>, deepseek: bool) {
        for exchange in &self.history {
            let mut assistant = json!({"role":"assistant", "content":exchange.content});
            if !exchange.calls.is_empty() {
                assistant["tool_calls"] = json!(exchange.calls.iter().map(|call| json!({
                    "id":call.id, "type":"function",
                    "function":{"name":call.name, "arguments":call.arguments}
                })).collect::<Vec<_>>());
            }
            if deepseek && !exchange.reasoning.is_empty() {
                assistant["reasoning_content"] = json!(exchange.reasoning);
            }
            messages.push(assistant);
            messages.extend(exchange.results.iter().map(|result| json!({
                "role":"tool", "tool_call_id":result.call_id, "content":result.output
            })));
        }
    }

    pub(crate) fn extend_responses(&self, input: &mut Vec<Value>) {
        for exchange in &self.history {
            if exchange.response_items.is_empty() {
                if !exchange.content.is_empty() {
                    input.push(json!({"role":"assistant", "content":exchange.content}));
                }
                input.extend(exchange.calls.iter().map(|call| json!({
                    "type":"function_call", "call_id":call.id,
                    "name":call.name, "arguments":call.arguments
                })));
            } else {
                input.extend(exchange.response_items.iter().cloned());
            }
            input.extend(exchange.results.iter().map(|result| json!({
                "type":"function_call_output", "call_id":result.call_id, "output":result.output
            })));
        }
    }
}

#[derive(Default)]
pub struct ToolAccumulator {
    calls: BTreeMap<usize, ToolCall>,
}

impl ToolAccumulator {
    pub fn push(&mut self, delta: ToolCallDelta) -> Result<(), GatewayError> {
        if delta.index >= MAX_PARALLEL_CALLS {
            return Err(GatewayError::Protocol("工具调用数量超过上限".into()));
        }
        let call = self.calls.entry(delta.index).or_insert_with(|| ToolCall {
            id: String::new(), name: String::new(), arguments: String::new(),
        });
        if let Some(id) = delta.id.filter(|id| !id.is_empty()) {
            if id.len() > 256 || (!call.id.is_empty() && call.id != id) {
                return Err(GatewayError::Protocol("工具调用标识无效".into()));
            }
            call.id = id;
        }
        if let Some(name) = delta.name {
            if call.name.len() + name.len() > 128 {
                return Err(GatewayError::Protocol("工具名称超过上限".into()));
            }
            call.name.push_str(&name);
        }
        if call.arguments.len() + delta.arguments.len() > MAX_ARGUMENT_BYTES {
            return Err(GatewayError::Protocol("工具参数超过上限".into()));
        }
        call.arguments.push_str(&delta.arguments);
        Ok(())
    }

    pub fn finish(self) -> Result<Vec<ToolCall>, GatewayError> {
        let mut ids = HashSet::with_capacity(self.calls.len());
        for call in self.calls.values() {
            if call.id.is_empty() || call.name.is_empty() || !ids.insert(call.id.as_str())
                || !serde_json::from_str::<Value>(&call.arguments).is_ok_and(|value| value.is_object())
            {
                return Err(GatewayError::Protocol("工具调用不完整或标识重复".into()));
            }
        }
        Ok(self.calls.into_values().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interleaved_fragments_keep_each_calls_identity_and_unicode_arguments() {
        let mut calls = ToolAccumulator::default();
        calls.push(ToolCallDelta { index: 1, id: Some("second".into()), name: Some("web_search".into()), arguments: "{\"query\":\"SQLite".into() }).unwrap();
        calls.push(ToolCallDelta { index: 0, id: Some("first".into()), name: Some("web_search".into()), arguments: "{\"query\":\"并".into() }).unwrap();
        calls.push(ToolCallDelta { index: 1, id: None, name: None, arguments: " WAL\"}".into() }).unwrap();
        calls.push(ToolCallDelta { index: 0, id: None, name: None, arguments: "发\"}".into() }).unwrap();
        let calls = calls.finish().unwrap();
        assert_eq!(calls[0].id, "first");
        assert_eq!(serde_json::from_str::<serde_json::Value>(&calls[0].arguments).unwrap()["query"], "并发");
        assert_eq!(calls[1].id, "second");
        assert_eq!(serde_json::from_str::<serde_json::Value>(&calls[1].arguments).unwrap()["query"], "SQLite WAL");
    }

    #[test]
    fn changing_a_call_identity_and_excessive_argument_bytes_are_rejected() {
        let mut calls = ToolAccumulator::default();
        calls.push(ToolCallDelta { index: 0, id: Some("first".into()), name: Some("web_search".into()), arguments: String::new() }).unwrap();
        assert!(calls.push(ToolCallDelta { index: 0, id: Some("different".into()), name: None, arguments: String::new() }).is_err());
        let mut calls = ToolAccumulator::default();
        assert!(calls.push(ToolCallDelta { index: 0, id: Some("first".into()), name: Some("web_search".into()), arguments: "a".repeat(MAX_ARGUMENT_BYTES + 1) }).is_err());
    }

    #[test]
    fn incomplete_and_duplicate_call_ids_cannot_be_dispatched() {
        let mut incomplete = ToolAccumulator::default();
        incomplete.push(ToolCallDelta { index: 0, id: None, name: Some("web_search".into()), arguments: "{}".into() }).unwrap();
        assert!(incomplete.finish().is_err());
        let mut duplicated = ToolAccumulator::default();
        for index in 0..2 {
            duplicated.push(ToolCallDelta { index, id: Some("same".into()), name: Some("web_search".into()), arguments: "{}".into() }).unwrap();
        }
        assert!(duplicated.finish().is_err());
    }
}
