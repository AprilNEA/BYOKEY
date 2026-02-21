//! Translates `OpenAI` chat completion requests into Claude Messages API format.

use byokey_types::{ByokError, RequestTranslator, traits::Result};
use serde_json::{Value, json};

/// Translator from `OpenAI` chat completion request format to Claude Messages API format.
pub struct OpenAIToClaude;

/// Builds Claude `messages` array from non-system `OpenAI` messages.
///
/// Tool result messages (`role == "tool"`) are buffered and flushed as a single
/// `user` message with `tool_result` content blocks before the next non-tool message.
fn build_claude_messages(non_system: &[&Value]) -> Vec<Value> {
    let mut claude_messages: Vec<Value> = Vec::new();
    let mut tool_buffer: Vec<Value> = Vec::new();

    for m in non_system {
        let role = m.get("role").and_then(Value::as_str).unwrap_or("user");

        if role == "tool" {
            let tool_call_id = m.get("tool_call_id").and_then(Value::as_str).unwrap_or("");
            let content = m
                .get("content")
                .cloned()
                .unwrap_or_else(|| Value::String(String::new()));
            tool_buffer.push(json!({
                "type": "tool_result",
                "tool_use_id": tool_call_id,
                "content": content,
            }));
            continue;
        }

        if !tool_buffer.is_empty() {
            claude_messages.push(json!({
                "role": "user",
                "content": std::mem::take(&mut tool_buffer),
            }));
        }

        if role == "assistant" {
            if let Some(tool_calls) = m.get("tool_calls").and_then(Value::as_array) {
                let mut content_blocks: Vec<Value> = Vec::new();
                if let Some(text) = m.get("content").and_then(Value::as_str)
                    && !text.is_empty()
                {
                    content_blocks.push(json!({"type": "text", "text": text}));
                }
                for tc in tool_calls {
                    let id = tc.get("id").and_then(Value::as_str).unwrap_or("");
                    let func = tc.get("function").unwrap_or(&Value::Null);
                    let name = func.get("name").and_then(Value::as_str).unwrap_or("");
                    let args_str = func
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or("{}");
                    let input: Value = serde_json::from_str(args_str).unwrap_or_else(|_| json!({}));
                    content_blocks.push(json!({
                        "type": "tool_use",
                        "id": id,
                        "name": name,
                        "input": input,
                    }));
                }
                claude_messages.push(json!({
                    "role": "assistant",
                    "content": content_blocks,
                }));
            } else {
                let content = m
                    .get("content")
                    .cloned()
                    .unwrap_or_else(|| Value::String(String::new()));
                claude_messages.push(json!({ "role": "assistant", "content": content }));
            }
        } else {
            let content = m
                .get("content")
                .cloned()
                .unwrap_or_else(|| Value::String(String::new()));
            claude_messages.push(json!({ "role": role, "content": content }));
        }
    }

    if !tool_buffer.is_empty() {
        claude_messages.push(json!({
            "role": "user",
            "content": tool_buffer,
        }));
    }

    claude_messages
}

impl RequestTranslator for OpenAIToClaude {
    /// Translates an `OpenAI` chat completion request into a Claude Messages API request.
    ///
    /// System messages are extracted and merged into the top-level `system` field.
    /// Non-system messages are forwarded as Claude `messages`.
    ///
    /// # Errors
    ///
    /// Returns [`ByokError::Translation`] if `model` or `messages` is missing.
    fn translate_request(&self, req: Value) -> Result<Value> {
        let model = req
            .get("model")
            .and_then(Value::as_str)
            .ok_or_else(|| ByokError::Translation("missing 'model'".into()))?
            .to_string();

        let messages = req
            .get("messages")
            .and_then(Value::as_array)
            .ok_or_else(|| ByokError::Translation("missing 'messages'".into()))?;

        let system_parts: Vec<&str> = messages
            .iter()
            .filter(|m| m.get("role").and_then(Value::as_str) == Some("system"))
            .filter_map(|m| m.get("content").and_then(Value::as_str))
            .collect();
        let system = if system_parts.is_empty() {
            None
        } else {
            Some(system_parts.join("\n"))
        };

        let non_system: Vec<&Value> = messages
            .iter()
            .filter(|m| m.get("role").and_then(Value::as_str) != Some("system"))
            .collect();

        let claude_messages = build_claude_messages(&non_system);

        let max_tokens = req
            .get("max_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(4096);

        let mut out = json!({
            "model": model,
            "messages": claude_messages,
            "max_tokens": max_tokens,
        });

        if let Some(sys) = system {
            out["system"] = Value::String(sys);
        }
        if let Some(t) = req.get("temperature") {
            out["temperature"] = t.clone();
        }
        if let Some(s) = req.get("stream") {
            out["stream"] = s.clone();
        }

        if let Some(tools) = req.get("tools").and_then(Value::as_array) {
            let claude_tools: Vec<Value> = tools
                .iter()
                .filter_map(|t| {
                    let func = t.get("function")?;
                    let name = func.get("name")?.clone();
                    let description = func.get("description").cloned().unwrap_or(Value::Null);
                    let input_schema = func
                        .get("parameters")
                        .cloned()
                        .unwrap_or_else(|| json!({"type": "object"}));
                    let mut tool = json!({ "name": name, "input_schema": input_schema });
                    if !description.is_null() {
                        tool["description"] = description;
                    }
                    Some(tool)
                })
                .collect();
            if !claude_tools.is_empty() {
                out["tools"] = Value::Array(claude_tools);
            }
        }

        if let Some(tc) = req.get("tool_choice") {
            if let Some(s) = tc.as_str() {
                match s {
                    "auto" => out["tool_choice"] = json!({"type": "auto"}),
                    "required" => out["tool_choice"] = json!({"type": "any"}),
                    _ => {}
                }
            } else if let Some(name) = tc
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
            {
                out["tool_choice"] = json!({"type": "tool", "name": name});
            }
        }

        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_basic_translation() {
        let req = json!({
            "model": "claude-opus-4-5",
            "messages": [{"role": "user", "content": "Hello"}],
            "max_tokens": 100
        });
        let out = OpenAIToClaude.translate_request(req).unwrap();
        assert_eq!(out["model"], "claude-opus-4-5");
        assert_eq!(out["max_tokens"], 100);
        assert_eq!(out["messages"][0]["role"], "user");
        assert_eq!(out["messages"][0]["content"], "Hello");
    }

    #[test]
    fn test_system_message_extracted() {
        let req = json!({
            "model": "claude-opus-4-5",
            "messages": [
                {"role": "system", "content": "You are helpful."},
                {"role": "user", "content": "Hi"}
            ]
        });
        let out = OpenAIToClaude.translate_request(req).unwrap();
        assert_eq!(out["system"], "You are helpful.");
        assert_eq!(out["messages"].as_array().unwrap().len(), 1);
        assert_eq!(out["messages"][0]["role"], "user");
    }

    #[test]
    fn test_default_max_tokens() {
        let req = json!({
            "model": "claude-opus-4-5",
            "messages": [{"role": "user", "content": "hi"}]
        });
        let out = OpenAIToClaude.translate_request(req).unwrap();
        assert_eq!(out["max_tokens"], 4096);
    }

    #[test]
    fn test_temperature_forwarded() {
        let req = json!({
            "model": "m",
            "messages": [{"role": "user", "content": "hi"}],
            "temperature": 0.7
        });
        let out = OpenAIToClaude.translate_request(req).unwrap();
        assert_eq!(out["temperature"], 0.7);
    }

    #[test]
    fn test_stream_forwarded() {
        let req = json!({
            "model": "m",
            "messages": [{"role": "user", "content": "hi"}],
            "stream": true
        });
        let out = OpenAIToClaude.translate_request(req).unwrap();
        assert_eq!(out["stream"], true);
    }

    #[test]
    fn test_tools_translated() {
        let req = json!({
            "model": "m",
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "get_weather",
                    "description": "Get the weather",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "city": {"type": "string"}
                        },
                        "required": ["city"]
                    }
                }
            }]
        });
        let out = OpenAIToClaude.translate_request(req).unwrap();
        let tools = out["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "get_weather");
        assert_eq!(tools[0]["description"], "Get the weather");
        assert_eq!(tools[0]["input_schema"]["type"], "object");
        assert_eq!(
            tools[0]["input_schema"]["properties"]["city"]["type"],
            "string"
        );
        // Ensure OpenAI-style fields are not present
        assert!(tools[0].get("type").is_none());
        assert!(tools[0].get("function").is_none());
    }

    #[test]
    fn test_tool_choice_auto() {
        let req = json!({
            "model": "m",
            "messages": [{"role": "user", "content": "hi"}],
            "tool_choice": "auto"
        });
        let out = OpenAIToClaude.translate_request(req).unwrap();
        assert_eq!(out["tool_choice"]["type"], "auto");
    }

    #[test]
    fn test_tool_choice_specific() {
        let req = json!({
            "model": "m",
            "messages": [{"role": "user", "content": "hi"}],
            "tool_choice": {"type": "function", "function": {"name": "get_weather"}}
        });
        let out = OpenAIToClaude.translate_request(req).unwrap();
        assert_eq!(out["tool_choice"]["type"], "tool");
        assert_eq!(out["tool_choice"]["name"], "get_weather");
    }

    #[test]
    fn test_tool_calls_in_assistant_message() {
        let req = json!({
            "model": "m",
            "messages": [
                {"role": "user", "content": "What's the weather?"},
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{\"city\":\"Beijing\"}"
                        }
                    }]
                }
            ]
        });
        let out = OpenAIToClaude.translate_request(req).unwrap();
        let assistant_msg = &out["messages"][1];
        assert_eq!(assistant_msg["role"], "assistant");
        let content = assistant_msg["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "tool_use");
        assert_eq!(content[0]["id"], "call_1");
        assert_eq!(content[0]["name"], "get_weather");
        assert_eq!(content[0]["input"]["city"], "Beijing");
    }

    #[test]
    fn test_tool_result_messages() {
        let req = json!({
            "model": "m",
            "messages": [
                {"role": "user", "content": "What's the weather?"},
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{\"city\":\"Beijing\"}"
                        }
                    }]
                },
                {"role": "tool", "tool_call_id": "call_1", "content": "Sunny, 25C"}
            ]
        });
        let out = OpenAIToClaude.translate_request(req).unwrap();
        // tool message becomes a user message with tool_result content
        let tool_result_msg = &out["messages"][2];
        assert_eq!(tool_result_msg["role"], "user");
        let content = tool_result_msg["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "tool_result");
        assert_eq!(content[0]["tool_use_id"], "call_1");
        assert_eq!(content[0]["content"], "Sunny, 25C");
    }

    #[test]
    fn test_multiple_tool_results_merged() {
        let req = json!({
            "model": "m",
            "messages": [
                {"role": "user", "content": "hi"},
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {"id": "call_1", "type": "function", "function": {"name": "a", "arguments": "{}"}},
                        {"id": "call_2", "type": "function", "function": {"name": "b", "arguments": "{}"}}
                    ]
                },
                {"role": "tool", "tool_call_id": "call_1", "content": "result1"},
                {"role": "tool", "tool_call_id": "call_2", "content": "result2"}
            ]
        });
        let out = OpenAIToClaude.translate_request(req).unwrap();
        // Two consecutive tool messages merged into one user message
        assert_eq!(out["messages"].as_array().unwrap().len(), 3);
        let tool_msg = &out["messages"][2];
        assert_eq!(tool_msg["role"], "user");
        let content = tool_msg["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["tool_use_id"], "call_1");
        assert_eq!(content[1]["tool_use_id"], "call_2");
    }

    #[test]
    fn test_missing_model_error() {
        let req = json!({"messages": [{"role": "user", "content": "hi"}]});
        assert!(OpenAIToClaude.translate_request(req).is_err());
    }

    #[test]
    fn test_missing_messages_error() {
        let req = json!({"model": "m"});
        assert!(OpenAIToClaude.translate_request(req).is_err());
    }
}
