//! Translates Gemini API responses into OpenAI-compatible format.

use byok_types::{ResponseTranslator, traits::Result};
use serde_json::{Value, json};

/// Translator from Gemini response format to `OpenAI` chat completion format.
pub struct GeminiToOpenAI;

impl ResponseTranslator for GeminiToOpenAI {
    /// Translates a Gemini `generateContent` response into an `OpenAI` chat completion response.
    ///
    /// # Errors
    ///
    /// Returns an error if the response cannot be translated.
    fn translate_response(&self, res: Value) -> Result<Value> {
        let text = res
            .pointer("/candidates/0/content/parts/0/text")
            .and_then(Value::as_str)
            .unwrap_or("");

        let finish_reason = match res
            .pointer("/candidates/0/finishReason")
            .and_then(Value::as_str)
        {
            Some("MAX_TOKENS" | "max_tokens") => "length",
            _ => "stop",
        };

        let prompt_tokens = res
            .pointer("/usageMetadata/promptTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let completion_tokens = res
            .pointer("/usageMetadata/candidatesTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or(0);

        Ok(json!({
            "id": "chatcmpl-gemini",
            "object": "chat.completion",
            "model": "gemini",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": text},
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
            "candidates": [{
                "content": { "parts": [{"text": "Hi there!"}], "role": "model" },
                "finishReason": "STOP"
            }],
            "usageMetadata": { "promptTokenCount": 8, "candidatesTokenCount": 4 }
        })
    }

    #[test]
    fn test_content_extraction() {
        let out = GeminiToOpenAI.translate_response(sample()).unwrap();
        assert_eq!(out["choices"][0]["message"]["content"], "Hi there!");
    }

    #[test]
    fn test_finish_reason_stop() {
        let out = GeminiToOpenAI.translate_response(sample()).unwrap();
        assert_eq!(out["choices"][0]["finish_reason"], "stop");
    }

    #[test]
    fn test_finish_reason_length() {
        let mut r = sample();
        r["candidates"][0]["finishReason"] = json!("MAX_TOKENS");
        let out = GeminiToOpenAI.translate_response(r).unwrap();
        assert_eq!(out["choices"][0]["finish_reason"], "length");
    }

    #[test]
    fn test_usage_mapping() {
        let out = GeminiToOpenAI.translate_response(sample()).unwrap();
        assert_eq!(out["usage"]["prompt_tokens"], 8);
        assert_eq!(out["usage"]["completion_tokens"], 4);
        assert_eq!(out["usage"]["total_tokens"], 12);
    }
}
