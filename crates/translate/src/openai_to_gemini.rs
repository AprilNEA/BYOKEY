//! Translates `OpenAI` chat completion requests into Gemini `generateContent` format.

use byok_types::{ByokError, RequestTranslator, traits::Result};
use serde_json::{Value, json};

/// Translator from `OpenAI` chat completion request format to Gemini `generateContent` format.
pub struct OpenAIToGemini;

impl RequestTranslator for OpenAIToGemini {
    /// Translates an `OpenAI` chat completion request into a Gemini `generateContent` request.
    ///
    /// System messages are extracted into the `systemInstruction` field.
    /// The `assistant` role is mapped to `model`.
    ///
    /// # Errors
    ///
    /// Returns [`ByokError::Translation`] if `messages` is missing.
    fn translate_request(&self, req: Value) -> Result<Value> {
        let messages = req
            .get("messages")
            .and_then(Value::as_array)
            .ok_or_else(|| ByokError::Translation("missing 'messages'".into()))?;

        let system_parts: Vec<&str> = messages
            .iter()
            .filter(|m| m.get("role").and_then(Value::as_str) == Some("system"))
            .filter_map(|m| m.get("content").and_then(Value::as_str))
            .collect();
        let system_text = system_parts.join("\n");

        let contents: Vec<Value> = messages
            .iter()
            .filter(|m| m.get("role").and_then(Value::as_str) != Some("system"))
            .map(|m| {
                let role = match m.get("role").and_then(Value::as_str) {
                    Some("assistant") => "model",
                    _ => "user",
                };
                let text = m.get("content").and_then(Value::as_str).unwrap_or("");
                json!({ "role": role, "parts": [{"text": text}] })
            })
            .collect();

        let mut generation_config = json!({});
        if let Some(max_tokens) = req.get("max_tokens").and_then(Value::as_u64) {
            generation_config["maxOutputTokens"] = json!(max_tokens);
        }
        if let Some(temp) = req.get("temperature") {
            generation_config["temperature"] = temp.clone();
        }

        let mut out = json!({
            "contents": contents,
            "generationConfig": generation_config,
        });

        if !system_text.is_empty() {
            out["systemInstruction"] = json!({ "parts": [{"text": system_text}] });
        }

        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_basic_contents() {
        let req = json!({
            "messages": [{"role": "user", "content": "Hello"}]
        });
        let out = OpenAIToGemini.translate_request(req).unwrap();
        assert_eq!(out["contents"][0]["role"], "user");
        assert_eq!(out["contents"][0]["parts"][0]["text"], "Hello");
    }

    #[test]
    fn test_assistant_becomes_model() {
        let req = json!({
            "messages": [
                {"role": "user", "content": "Hi"},
                {"role": "assistant", "content": "Hello!"}
            ]
        });
        let out = OpenAIToGemini.translate_request(req).unwrap();
        assert_eq!(out["contents"][1]["role"], "model");
    }

    #[test]
    fn test_system_to_instruction() {
        let req = json!({
            "messages": [
                {"role": "system", "content": "Be concise."},
                {"role": "user", "content": "Hi"}
            ]
        });
        let out = OpenAIToGemini.translate_request(req).unwrap();
        assert_eq!(out["systemInstruction"]["parts"][0]["text"], "Be concise.");
        assert_eq!(out["contents"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_max_tokens_mapping() {
        let req = json!({
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 512
        });
        let out = OpenAIToGemini.translate_request(req).unwrap();
        assert_eq!(out["generationConfig"]["maxOutputTokens"], 512);
    }

    #[test]
    fn test_no_system_no_instruction_field() {
        let req = json!({ "messages": [{"role": "user", "content": "hi"}] });
        let out = OpenAIToGemini.translate_request(req).unwrap();
        assert!(out.get("systemInstruction").is_none());
    }
}
