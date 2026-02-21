//! Translates Claude API responses into OpenAI-compatible format.

use byokey_types::{ResponseTranslator, traits::Result};
use serde_json::{Value, json};

/// Translator from Claude response format to `OpenAI` chat completion format.
pub struct ClaudeToOpenAI;

/// Maps a Claude `stop_reason` to an `OpenAI` `finish_reason`.
fn map_finish_reason(stop_reason: Option<&str>) -> &'static str {
    match stop_reason {
        Some("max_tokens") => "length",
        Some("tool_use") => "tool_calls",
        _ => "stop",
    }
}

impl ResponseTranslator for ClaudeToOpenAI {
    /// Translates a Claude Messages API response into an `OpenAI` chat completion response.
    ///
    /// # Errors
    ///
    /// Returns an error if the response cannot be translated.
    fn translate_response(&self, res: Value) -> Result<Value> {
        let content_blocks = res.get("content").and_then(Value::as_array);

        // Extract text content and tool_use blocks
        let mut text_parts: Vec<&str> = Vec::new();
        let mut tool_calls: Vec<Value> = Vec::new();

        if let Some(blocks) = content_blocks {
            for block in blocks {
                let block_type = block.get("type").and_then(Value::as_str).unwrap_or("");
                match block_type {
                    "text" => {
                        if let Some(text) = block.get("text").and_then(Value::as_str) {
                            text_parts.push(text);
                        }
                    }
                    "tool_use" => {
                        let id = block.get("id").and_then(Value::as_str).unwrap_or("");
                        let name = block.get("name").and_then(Value::as_str).unwrap_or("");
                        let input = block.get("input").unwrap_or(&Value::Null);
                        let arguments = serde_json::to_string(input).unwrap_or_default();
                        tool_calls.push(json!({
                            "id": id,
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": arguments,
                            }
                        }));
                    }
                    _ => {}
                }
            }
        }

        let finish_reason = map_finish_reason(res.get("stop_reason").and_then(Value::as_str));

        let model = res
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();

        let id = res
            .get("id")
            .and_then(Value::as_str)
            .map_or_else(|| "chatcmpl-unknown".to_string(), |s| format!("chatcmpl-{s}"));

        let prompt_tokens = res
            .pointer("/usage/input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let completion_tokens = res
            .pointer("/usage/output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);

        let content_value: Value = if text_parts.is_empty() {
            Value::Null
        } else {
            Value::String(text_parts.join(""))
        };

        let mut message = json!({
            "role": "assistant",
            "content": content_value,
        });

        if !tool_calls.is_empty() {
            message["tool_calls"] = Value::Array(tool_calls);
        }

        Ok(json!({
            "id": id,
            "object": "chat.completion",
            "model": model,
            "choices": [{
                "index": 0,
                "message": message,
                "finish_reason": finish_reason
            }],
            "usage": {
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens,
                "total_tokens": prompt_tokens + completion_tokens
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "id": "msg_abc123",
            "type": "message",
            "role": "assistant",
            "model": "claude-opus-4-5",
            "content": [{"type": "text", "text": "Hello there!"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 10, "output_tokens": 5}
        })
    }

    #[test]
    fn test_basic() {
        let out = ClaudeToOpenAI.translate_response(sample()).unwrap();
        assert_eq!(out["choices"][0]["message"]["content"], "Hello there!");
        assert_eq!(out["choices"][0]["message"]["role"], "assistant");
        assert_eq!(out["choices"][0]["finish_reason"], "stop");
    }

    #[test]
    fn test_model_forwarded() {
        let out = ClaudeToOpenAI.translate_response(sample()).unwrap();
        assert_eq!(out["model"], "claude-opus-4-5");
    }

    #[test]
    fn test_usage_mapping() {
        let out = ClaudeToOpenAI.translate_response(sample()).unwrap();
        assert_eq!(out["usage"]["prompt_tokens"], 10);
        assert_eq!(out["usage"]["completion_tokens"], 5);
        assert_eq!(out["usage"]["total_tokens"], 15);
    }

    #[test]
    fn test_id_prefixed() {
        let out = ClaudeToOpenAI.translate_response(sample()).unwrap();
        assert!(out["id"].as_str().unwrap().starts_with("chatcmpl-"));
    }

    #[test]
    fn test_tool_use_response() {
        let res = json!({
            "id": "msg_tool",
            "type": "message",
            "role": "assistant",
            "model": "claude-opus-4-5",
            "content": [
                {"type": "tool_use", "id": "toolu_abc", "name": "get_weather", "input": {"city": "Beijing"}}
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 10, "output_tokens": 20}
        });
        let out = ClaudeToOpenAI.translate_response(res).unwrap();
        let msg = &out["choices"][0]["message"];
        assert_eq!(msg["content"], Value::Null);
        let tc = msg["tool_calls"].as_array().unwrap();
        assert_eq!(tc.len(), 1);
        assert_eq!(tc[0]["id"], "toolu_abc");
        assert_eq!(tc[0]["type"], "function");
        assert_eq!(tc[0]["function"]["name"], "get_weather");
        let args: Value = serde_json::from_str(tc[0]["function"]["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(args["city"], "Beijing");
    }

    #[test]
    fn test_tool_use_with_text() {
        let res = json!({
            "id": "msg_mixed",
            "type": "message",
            "role": "assistant",
            "model": "claude-opus-4-5",
            "content": [
                {"type": "text", "text": "Let me check the weather."},
                {"type": "tool_use", "id": "toolu_123", "name": "get_weather", "input": {"city": "Tokyo"}}
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 10, "output_tokens": 20}
        });
        let out = ClaudeToOpenAI.translate_response(res).unwrap();
        let msg = &out["choices"][0]["message"];
        assert_eq!(msg["content"], "Let me check the weather.");
        assert!(msg["tool_calls"].as_array().unwrap().len() == 1);
    }

    #[test]
    fn test_tool_use_finish_reason() {
        let res = json!({
            "id": "msg_tr",
            "type": "message",
            "role": "assistant",
            "model": "claude-opus-4-5",
            "content": [
                {"type": "tool_use", "id": "toolu_1", "name": "fn1", "input": {}}
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 1, "output_tokens": 1}
        });
        let out = ClaudeToOpenAI.translate_response(res).unwrap();
        assert_eq!(out["choices"][0]["finish_reason"], "tool_calls");
    }

    #[test]
    fn test_finish_reason_length() {
        let mut r = sample();
        r["stop_reason"] = json!("max_tokens");
        let out = ClaudeToOpenAI.translate_response(r).unwrap();
        assert_eq!(out["choices"][0]["finish_reason"], "length");
    }

    #[test]
    fn test_object_field() {
        let out = ClaudeToOpenAI.translate_response(sample()).unwrap();
        assert_eq!(out["object"], "chat.completion");
    }
}
