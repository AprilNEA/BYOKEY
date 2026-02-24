//! Strongly-typed OpenAI-compatible chat completion request.
//!
//! Replaces raw `serde_json::Value` usage at the API boundary, providing
//! compile-time guarantees for common fields (`model`, `stream`, `messages`)
//! while preserving forward-compatibility through a catch-all extra map.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// A strongly-typed OpenAI-compatible chat completion request body.
///
/// Common fields are deserialized into typed fields; all remaining fields
/// (e.g., `temperature`, `tools`, `max_tokens`) are captured in [`extra`].
///
/// Use [`ChatRequest::into_body`] to reconstruct a full `serde_json::Value`
/// for translator consumption.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    /// The model identifier (e.g., `"claude-opus-4-5"`, `"gpt-4o"`).
    pub model: String,
    /// Whether to use streaming SSE mode.
    #[serde(default)]
    pub stream: bool,
    /// The conversation messages.
    pub messages: Vec<Value>,
    /// All remaining fields not captured above.
    #[serde(flatten)]
    pub extra: HashMap<String, Value>,
}

impl ChatRequest {
    /// Reconstructs the full request body as a `serde_json::Value`.
    ///
    /// The returned `Value` is an object containing `model`, `stream`,
    /// `messages`, plus all extra fields — suitable for passing to translators.
    #[must_use]
    pub fn into_body(self) -> Value {
        // Start with the extra fields, then overlay the typed ones.
        let mut map = serde_json::Map::with_capacity(self.extra.len() + 3);
        for (k, v) in self.extra {
            map.insert(k, v);
        }
        map.insert("model".into(), Value::String(self.model));
        map.insert("stream".into(), Value::Bool(self.stream));
        map.insert("messages".into(), Value::Array(self.messages));
        Value::Object(map)
    }

    /// Returns a `serde_json::Value` view of the full body without consuming self.
    #[must_use]
    pub fn to_body(&self) -> Value {
        self.clone().into_body()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_deserialize_minimal() {
        let v = json!({
            "model": "claude-opus-4-5",
            "messages": [{"role": "user", "content": "hi"}]
        });
        let req: ChatRequest = serde_json::from_value(v).unwrap();
        assert_eq!(req.model, "claude-opus-4-5");
        assert!(!req.stream);
        assert_eq!(req.messages.len(), 1);
        assert!(req.extra.is_empty());
    }

    #[test]
    fn test_deserialize_with_stream() {
        let v = json!({
            "model": "gpt-4o",
            "stream": true,
            "messages": []
        });
        let req: ChatRequest = serde_json::from_value(v).unwrap();
        assert!(req.stream);
    }

    #[test]
    fn test_extra_fields_preserved() {
        let v = json!({
            "model": "m",
            "messages": [],
            "temperature": 0.7,
            "max_tokens": 1024,
            "tools": [{"type": "function"}]
        });
        let req: ChatRequest = serde_json::from_value(v).unwrap();
        assert_eq!(req.extra.len(), 3);
        assert_eq!(req.extra["temperature"], json!(0.7));
        assert_eq!(req.extra["max_tokens"], json!(1024));
    }

    #[test]
    fn test_into_body_roundtrip() {
        let original = json!({
            "model": "claude-opus-4-5",
            "stream": true,
            "messages": [{"role": "user", "content": "test"}],
            "temperature": 0.5,
        });
        let req: ChatRequest = serde_json::from_value(original.clone()).unwrap();
        let body = req.into_body();
        assert_eq!(body["model"], "claude-opus-4-5");
        assert_eq!(body["stream"], true);
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["temperature"], 0.5);
    }

    #[test]
    fn test_to_body_does_not_consume() {
        let v = json!({
            "model": "m",
            "messages": [{"role": "user", "content": "hi"}]
        });
        let req: ChatRequest = serde_json::from_value(v).unwrap();
        let _ = req.to_body();
        // req is still usable
        assert_eq!(req.model, "m");
    }

    #[test]
    fn test_stream_defaults_to_false() {
        let v = json!({"model": "m", "messages": []});
        let req: ChatRequest = serde_json::from_value(v).unwrap();
        assert!(!req.stream);
    }
}
