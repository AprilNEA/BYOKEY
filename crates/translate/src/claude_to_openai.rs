//! Translates Claude API responses into OpenAI-compatible format.

use byok_types::{ResponseTranslator, traits::Result};
use serde_json::{Value, json};

/// Translator from Claude response format to `OpenAI` chat completion format.
pub struct ClaudeToOpenAI;

/// Maps a Claude `stop_reason` to an `OpenAI` `finish_reason`.
fn map_finish_reason(stop_reason: Option<&str>) -> &'static str {
    match stop_reason {
        Some("max_tokens") => "length",
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
        let content = res
            .get("content")
            .and_then(Value::as_array)
            .and_then(|arr| arr.first())
            .and_then(|c| c.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("");

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

        Ok(json!({
            "id": id,
            "object": "chat.completion",
            "model": model,
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": content },
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
