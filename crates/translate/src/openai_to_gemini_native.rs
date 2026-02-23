//! Translates `OpenAI` chat completion responses into Gemini native format.
//!
//! Supports both complete (non-streaming) and SSE chunk-by-chunk translation so
//! that an `OpenAI`-compatible backend response can be served as if it came from
//! the Google `generateContent` / `streamGenerateContent` endpoint.
//!
//! # Usage
//!
//! ```ignore
//! use byokey_translate::{OpenAIResponseToGemini, OpenAISseChunk};
//!
//! // Complete response
//! let gemini: Value = OpenAIResponseToGemini { body: &resp, model: "gemini-2.5-pro" }
//!     .try_into()?;
//!
//! // SSE chunk
//! let chunk: Option<Vec<u8>> = OpenAISseChunk { line: &bytes, model: "gemini-2.5-pro" }
//!     .into();
//! ```

use byokey_types::ByokError;
use serde_json::{Value, json};

/// An `OpenAI` chat completion response paired with the target model name.
///
/// Implements `TryFrom` → [`Value`] to produce a Gemini native
/// `generateContent` response.
pub struct OpenAIResponseToGemini<'a> {
    pub body: &'a Value,
    pub model: &'a str,
}

impl TryFrom<OpenAIResponseToGemini<'_>> for Value {
    type Error = ByokError;

    fn try_from(resp: OpenAIResponseToGemini<'_>) -> std::result::Result<Self, Self::Error> {
        let choice = resp
            .body
            .pointer("/choices/0")
            .ok_or_else(|| ByokError::Translation("missing choices[0]".into()))?;

        let message = choice
            .get("message")
            .ok_or_else(|| ByokError::Translation("missing choices[0].message".into()))?;

        let parts = build_parts_from_message(message);

        let finish_reason = choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .map_or("STOP", map_finish_reason);

        let mut candidate = json!({
            "content": {"parts": parts, "role": "model"},
            "finishReason": finish_reason,
            "index": 0,
        });

        // Preserve safety ratings if present
        if let Some(ratings) = choice.get("safety_ratings") {
            candidate["safetyRatings"] = ratings.clone();
        }

        let mut out = json!({
            "candidates": [candidate],
            "modelVersion": resp.model,
        });

        // Map usage
        if let Some(usage) = resp.body.get("usage") {
            let prompt = usage
                .get("prompt_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let completion = usage
                .get("completion_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            out["usageMetadata"] = json!({
                "promptTokenCount": prompt,
                "candidatesTokenCount": completion,
                "totalTokenCount": prompt + completion,
            });
        }

        Ok(out)
    }
}

/// A single `OpenAI` SSE chunk (raw line bytes) paired with the target model name.
///
/// Implements `From` → `Option<Vec<u8>>`: returns `None` for `data: [DONE]` or
/// unparseable lines; otherwise returns translated Gemini-native SSE bytes
/// (including `data: ` prefix and `\r\n\r\n` suffix).
pub struct OpenAISseChunk<'a> {
    pub line: &'a [u8],
    pub model: &'a str,
}

impl From<OpenAISseChunk<'_>> for Option<Vec<u8>> {
    fn from(chunk: OpenAISseChunk<'_>) -> Self {
        let s = std::str::from_utf8(chunk.line).ok()?;
        let s = s.trim();

        // Skip empty lines and non-data lines
        let data = s.strip_prefix("data: ")?;
        if data == "[DONE]" {
            return None;
        }

        let parsed: Value = serde_json::from_str(data).ok()?;
        let choice = parsed.pointer("/choices/0")?;
        let delta = choice.get("delta")?;

        let finish_reason = choice.get("finish_reason").and_then(Value::as_str);

        // Build Gemini candidate
        let parts = build_parts_from_delta(delta);

        let mut candidate = json!({
            "content": {"parts": parts, "role": "model"},
            "index": 0,
        });

        if let Some(reason) = finish_reason {
            candidate["finishReason"] = json!(map_finish_reason(reason));
        }

        let mut gemini_chunk = json!({
            "candidates": [candidate],
            "modelVersion": chunk.model,
        });

        // Include usage if present (typically on the final chunk)
        if let Some(usage) = parsed.get("usage") {
            let prompt = usage
                .get("prompt_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let completion = usage
                .get("completion_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            gemini_chunk["usageMetadata"] = json!({
                "promptTokenCount": prompt,
                "candidatesTokenCount": completion,
                "totalTokenCount": prompt + completion,
            });
        }

        let json_str = serde_json::to_string(&gemini_chunk).ok()?;
        Some(format!("data: {json_str}\r\n\r\n").into_bytes())
    }
}

/// Build Gemini `parts` array from an `OpenAI` complete message object.
fn build_parts_from_message(message: &Value) -> Vec<Value> {
    let mut parts = Vec::new();

    // Text content
    if let Some(text) = message.get("content").and_then(Value::as_str) {
        parts.push(json!({"text": text}));
    }

    // Tool calls → functionCall parts
    if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
        for tc in tool_calls {
            if let Some(func) = tc.get("function") {
                let name = func.get("name").and_then(Value::as_str).unwrap_or("");
                let args_str = func
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("{}");
                let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
                parts.push(json!({"functionCall": {"name": name, "args": args}}));
            }
        }
    }

    if parts.is_empty() {
        parts.push(json!({"text": ""}));
    }

    parts
}

/// Build Gemini `parts` array from an `OpenAI` streaming delta object.
fn build_parts_from_delta(delta: &Value) -> Vec<Value> {
    let mut parts = Vec::new();

    if let Some(content) = delta.get("content").and_then(Value::as_str) {
        parts.push(json!({"text": content}));
    }

    // Tool calls in delta
    if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
        for tc in tool_calls {
            if let Some(func) = tc.get("function") {
                let name = func.get("name").and_then(Value::as_str).unwrap_or("");
                let args_str = func.get("arguments").and_then(Value::as_str).unwrap_or("");
                if !name.is_empty() || !args_str.is_empty() {
                    let args: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
                    parts.push(json!({"functionCall": {"name": name, "args": args}}));
                }
            }
        }
    }

    if parts.is_empty() {
        parts.push(json!({"text": ""}));
    }

    parts
}

/// Map `OpenAI` `finish_reason` string to Gemini `finishReason`.
fn map_finish_reason(reason: &str) -> &'static str {
    match reason {
        "length" => "MAX_TOKENS",
        _ => "STOP",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_openai_response() -> Value {
        json!({
            "id": "chatcmpl-xxx",
            "object": "chat.completion",
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "Hi there!"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 8, "completion_tokens": 4, "total_tokens": 12}
        })
    }

    #[test]
    fn test_complete_response_content() {
        let resp = sample_openai_response();
        let out: Value = OpenAIResponseToGemini {
            body: &resp,
            model: "gemini-2.5-pro",
        }
        .try_into()
        .unwrap();
        assert_eq!(
            out["candidates"][0]["content"]["parts"][0]["text"],
            "Hi there!"
        );
        assert_eq!(out["candidates"][0]["content"]["role"], "model");
    }

    #[test]
    fn test_complete_response_finish_reason() {
        let resp = sample_openai_response();
        let out: Value = OpenAIResponseToGemini {
            body: &resp,
            model: "gemini-2.5-pro",
        }
        .try_into()
        .unwrap();
        assert_eq!(out["candidates"][0]["finishReason"], "STOP");
    }

    #[test]
    fn test_complete_response_finish_reason_length() {
        let mut resp = sample_openai_response();
        resp["choices"][0]["finish_reason"] = json!("length");
        let out: Value = OpenAIResponseToGemini {
            body: &resp,
            model: "gemini-2.5-pro",
        }
        .try_into()
        .unwrap();
        assert_eq!(out["candidates"][0]["finishReason"], "MAX_TOKENS");
    }

    #[test]
    fn test_complete_response_usage() {
        let resp = sample_openai_response();
        let out: Value = OpenAIResponseToGemini {
            body: &resp,
            model: "gemini-2.5-pro",
        }
        .try_into()
        .unwrap();
        assert_eq!(out["usageMetadata"]["promptTokenCount"], 8);
        assert_eq!(out["usageMetadata"]["candidatesTokenCount"], 4);
        assert_eq!(out["usageMetadata"]["totalTokenCount"], 12);
    }

    #[test]
    fn test_complete_response_model_version() {
        let resp = sample_openai_response();
        let out: Value = OpenAIResponseToGemini {
            body: &resp,
            model: "gemini-2.5-pro",
        }
        .try_into()
        .unwrap();
        assert_eq!(out["modelVersion"], "gemini-2.5-pro");
    }

    #[test]
    fn test_sse_chunk_text() {
        let line = b"data: {\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}";
        let result: Option<Vec<u8>> = OpenAISseChunk {
            line,
            model: "gemini-2.5-pro",
        }
        .into();
        assert!(result.is_some());
        let bytes = result.unwrap();
        let s = std::str::from_utf8(&bytes).unwrap();
        assert!(s.starts_with("data: "));
        let json_str = s.trim().strip_prefix("data: ").unwrap();
        let v: Value = serde_json::from_str(json_str).unwrap();
        assert_eq!(v["candidates"][0]["content"]["parts"][0]["text"], "Hello");
        assert_eq!(v["modelVersion"], "gemini-2.5-pro");
    }

    #[test]
    fn test_sse_chunk_done() {
        let line = b"data: [DONE]";
        let result: Option<Vec<u8>> = OpenAISseChunk {
            line,
            model: "gemini-2.5-pro",
        }
        .into();
        assert!(result.is_none());
    }

    #[test]
    fn test_sse_chunk_finish_reason() {
        let line = b"data: {\"id\":\"x\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}";
        let result: Option<Vec<u8>> = OpenAISseChunk {
            line,
            model: "gemini-2.5-pro",
        }
        .into();
        let bytes = result.unwrap();
        let s = std::str::from_utf8(&bytes).unwrap();
        let json_str = s.trim().strip_prefix("data: ").unwrap();
        let v: Value = serde_json::from_str(json_str).unwrap();
        assert_eq!(v["candidates"][0]["finishReason"], "STOP");
    }

    #[test]
    fn test_sse_empty_line() {
        let result: Option<Vec<u8>> = OpenAISseChunk {
            line: b"",
            model: "gemini-2.5-pro",
        }
        .into();
        assert!(result.is_none());
    }

    #[test]
    fn test_sse_non_data_line() {
        let result: Option<Vec<u8>> = OpenAISseChunk {
            line: b": keepalive",
            model: "gemini-2.5-pro",
        }
        .into();
        assert!(result.is_none());
    }

    #[test]
    fn test_complete_response_with_tool_calls() {
        let resp = json!({
            "id": "chatcmpl-xxx",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_0",
                        "type": "function",
                        "function": {"name": "get_weather", "arguments": "{\"location\":\"NYC\"}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        });
        let out: Value = OpenAIResponseToGemini {
            body: &resp,
            model: "gemini-2.5-pro",
        }
        .try_into()
        .unwrap();
        let fc = &out["candidates"][0]["content"]["parts"][0]["functionCall"];
        assert_eq!(fc["name"], "get_weather");
        assert_eq!(fc["args"]["location"], "NYC");
    }
}
