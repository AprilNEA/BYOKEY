//! Translates `OpenAI` chat completion requests into Codex (`OpenAI` Responses API) format.
//!
//! The Codex CLI uses a private Responses API at `chatgpt.com/backend-api/codex/responses`
//! that differs from the public Chat Completions API:
//!
//! - `messages` → `input` (typed message objects with content parts)
//! - `system` role → top-level `instructions` field
//! - `max_tokens` → `max_output_tokens`

use byokey_types::{ByokError, RequestTranslator, traits::Result};
use serde_json::{Value, json};

/// Translator from `OpenAI` chat completion request format to Codex Responses API format.
pub struct OpenAIToCodex;

/// Convert a single message content value into Codex content parts array.
fn to_codex_content(content: &Value, role: &str) -> Value {
    if let Some(text) = content.as_str() {
        let part_type = if role == "assistant" {
            "output_text"
        } else {
            "input_text"
        };
        return json!([{"type": part_type, "text": text}]);
    }

    // Already an array of content blocks (vision, tool results, etc.)
    if let Some(arr) = content.as_array() {
        let parts: Vec<Value> = arr
            .iter()
            .map(|block| {
                let block_type = block.get("type").and_then(Value::as_str).unwrap_or("");
                match block_type {
                    "text" => {
                        let text = block.get("text").and_then(Value::as_str).unwrap_or("");
                        let part_type = if role == "assistant" {
                            "output_text"
                        } else {
                            "input_text"
                        };
                        json!({"type": part_type, "text": text})
                    }
                    "image_url" => {
                        // Convert vision content
                        let url = block
                            .pointer("/image_url/url")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        json!({"type": "input_image", "image_url": url})
                    }
                    _ => block.clone(),
                }
            })
            .collect();
        return json!(parts);
    }

    json!([])
}

/// Converts `OpenAI` messages array to Codex input items (skipping system messages).
fn build_codex_input(messages: &[Value]) -> Vec<Value> {
    let mut input: Vec<Value> = Vec::new();
    for m in messages {
        let role = m.get("role").and_then(Value::as_str).unwrap_or("user");
        match role {
            "system" => {}
            "tool" => {
                let call_id = m
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let output = m
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": output,
                }));
            }
            "assistant" if m.get("tool_calls").is_some() => {
                if let Some(content) = m.get("content").filter(|c| !c.is_null()) {
                    let content_parts = to_codex_content(content, role);
                    input.push(json!({"type": "message", "role": role, "content": content_parts}));
                }
                if let Some(tool_calls) = m.get("tool_calls").and_then(Value::as_array) {
                    for tc in tool_calls {
                        let call_id = tc
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let name = tc
                            .pointer("/function/name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let arguments = tc
                            .pointer("/function/arguments")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        input.push(json!({"type": "function_call", "call_id": call_id, "name": name, "arguments": arguments}));
                    }
                }
            }
            _ => {
                let content = m
                    .get("content")
                    .cloned()
                    .unwrap_or(Value::String(String::new()));
                let content_parts = to_codex_content(&content, role);
                input.push(json!({"type": "message", "role": role, "content": content_parts}));
            }
        }
    }
    input
}

impl RequestTranslator for OpenAIToCodex {
    /// Translates an `OpenAI` chat completion request into a Codex Responses API request.
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

        let instructions: String = messages
            .iter()
            .filter(|m| m.get("role").and_then(Value::as_str) == Some("system"))
            .filter_map(|m| m.get("content").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");

        let input = build_codex_input(messages);

        let mut out = json!({
            "model": model,
            "input": input,
            "instructions": instructions,
            // Request reasoning content so reasoning models return their thinking.
            "include": ["reasoning.encrypted_content"],
        });

        // Translate tools definitions
        if let Some(tools) = req.get("tools").and_then(Value::as_array) {
            let codex_tools: Vec<Value> = tools
                .iter()
                .filter_map(|t| {
                    let func = t.get("function")?;
                    let mut name = func
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    if name.len() > 64 {
                        name.truncate(64);
                    }
                    let mut tool = json!({
                        "type": "function",
                        "name": name,
                    });
                    if let Some(desc) = func.get("description") {
                        tool["description"] = desc.clone();
                    }
                    if let Some(params) = func.get("parameters") {
                        tool["parameters"] = params.clone();
                    }
                    Some(tool)
                })
                .collect();
            if !codex_tools.is_empty() {
                out["tools"] = json!(codex_tools);
            }
        }

        if let Some(tokens) = req.get("max_tokens").and_then(Value::as_u64) {
            out["max_output_tokens"] = json!(tokens);
        }
        if let Some(t) = req.get("temperature") {
            out["temperature"] = t.clone();
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
            "model": "o4-mini",
            "messages": [{"role": "user", "content": "Hello"}]
        });
        let out = OpenAIToCodex.translate_request(req).unwrap();
        assert_eq!(out["model"], "o4-mini");
        assert_eq!(out["input"][0]["type"], "message");
        assert_eq!(out["input"][0]["role"], "user");
        assert_eq!(out["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(out["input"][0]["content"][0]["text"], "Hello");
    }

    #[test]
    fn test_system_to_instructions() {
        let req = json!({
            "model": "o4-mini",
            "messages": [
                {"role": "system", "content": "You are helpful."},
                {"role": "user", "content": "Hi"}
            ]
        });
        let out = OpenAIToCodex.translate_request(req).unwrap();
        assert_eq!(out["instructions"], "You are helpful.");
        assert_eq!(out["input"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_max_tokens_renamed() {
        let req = json!({
            "model": "o4-mini",
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 500
        });
        let out = OpenAIToCodex.translate_request(req).unwrap();
        assert_eq!(out["max_output_tokens"], 500);
        assert!(out.get("max_tokens").is_none());
    }

    #[test]
    fn test_assistant_content_type() {
        let req = json!({
            "model": "o4-mini",
            "messages": [
                {"role": "user", "content": "Hi"},
                {"role": "assistant", "content": "Hello!"}
            ]
        });
        let out = OpenAIToCodex.translate_request(req).unwrap();
        assert_eq!(out["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(out["input"][1]["content"][0]["type"], "output_text");
    }

    #[test]
    fn test_missing_model_error() {
        let req = json!({"messages": [{"role": "user", "content": "hi"}]});
        assert!(OpenAIToCodex.translate_request(req).is_err());
    }

    #[test]
    fn test_tools_translated() {
        let req = json!({
            "model": "o4-mini",
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "get_weather",
                    "description": "Get weather info",
                    "parameters": {"type": "object", "properties": {"city": {"type": "string"}}}
                }
            }]
        });
        let out = OpenAIToCodex.translate_request(req).unwrap();
        let tools = out["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["name"], "get_weather");
        assert_eq!(tools[0]["description"], "Get weather info");
        assert!(tools[0].get("parameters").is_some());
        // No nested "function" wrapper
        assert!(tools[0].get("function").is_none());
    }

    #[test]
    fn test_tool_calls_message() {
        let req = json!({
            "model": "o4-mini",
            "messages": [
                {"role": "user", "content": "What's the weather?"},
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "get_weather", "arguments": "{\"city\":\"Tokyo\"}"}
                    }]
                }
            ]
        });
        let out = OpenAIToCodex.translate_request(req).unwrap();
        let input = out["input"].as_array().unwrap();
        // user message + function_call
        assert_eq!(input.len(), 2);
        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[1]["call_id"], "call_1");
        assert_eq!(input[1]["name"], "get_weather");
        assert_eq!(input[1]["arguments"], "{\"city\":\"Tokyo\"}");
    }

    #[test]
    fn test_tool_result_message() {
        let req = json!({
            "model": "o4-mini",
            "messages": [
                {"role": "user", "content": "What's the weather?"},
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "get_weather", "arguments": "{\"city\":\"Tokyo\"}"}
                    }]
                },
                {"role": "tool", "tool_call_id": "call_1", "content": "Sunny, 25C"}
            ]
        });
        let out = OpenAIToCodex.translate_request(req).unwrap();
        let input = out["input"].as_array().unwrap();
        // user + function_call + function_call_output
        assert_eq!(input.len(), 3);
        assert_eq!(input[2]["type"], "function_call_output");
        assert_eq!(input[2]["call_id"], "call_1");
        assert_eq!(input[2]["output"], "Sunny, 25C");
    }
}
