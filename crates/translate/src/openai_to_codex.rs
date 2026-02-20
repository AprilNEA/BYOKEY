//! Translates `OpenAI` chat completion requests into Codex (OpenAI Responses API) format.
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

        // Extract system messages → instructions
        let instructions: String = messages
            .iter()
            .filter(|m| m.get("role").and_then(Value::as_str) == Some("system"))
            .filter_map(|m| m.get("content").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");

        // Convert non-system messages to Codex input items
        let input: Vec<Value> = messages
            .iter()
            .filter(|m| m.get("role").and_then(Value::as_str) != Some("system"))
            .map(|m| {
                let role = m.get("role").and_then(Value::as_str).unwrap_or("user");
                let content = m
                    .get("content")
                    .cloned()
                    .unwrap_or(Value::String(String::new()));
                let content_parts = to_codex_content(&content, role);
                json!({
                    "type": "message",
                    "role": role,
                    "content": content_parts,
                })
            })
            .collect();

        let mut out = json!({
            "model": model,
            "input": input,
            "instructions": instructions,
        });

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
}
