//! Translates `OpenAI` chat completion requests into Claude Messages API format.

use byokey_types::{ByokError, RequestTranslator, traits::Result};
use serde_json::{Value, json};

/// Translator from `OpenAI` chat completion request format to Claude Messages API format.
pub struct OpenAIToClaude;

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

        // Extract system messages and merge into a top-level system field
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

        // Filter out system messages
        let claude_messages: Vec<Value> = messages
            .iter()
            .filter(|m| m.get("role").and_then(Value::as_str) != Some("system"))
            .map(|m| {
                let role = m.get("role").and_then(Value::as_str).unwrap_or("user");
                let content = m
                    .get("content")
                    .cloned()
                    .unwrap_or_else(|| Value::String(String::new()));
                json!({ "role": role, "content": content })
            })
            .collect();

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
