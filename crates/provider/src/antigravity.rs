//! Antigravity executor — Google Cloud Code (`CLIProxyAPIPlus`) backend.
//!
//! Antigravity uses a Gemini-compatible request/response format wrapped in an
//! envelope with additional metadata fields. Streaming responses arrive as
//! JSON lines (not SSE), each containing a `response` field with a Gemini
//! stream chunk.

use crate::registry;
use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_translate::{GeminiToOpenAI, OpenAIToGemini};
use byokey_types::{
    ByokError, ProviderId, RequestTranslator, ResponseTranslator,
    traits::{ByteStream, ProviderExecutor, ProviderResponse, Result},
};
use bytes::Bytes;
use futures_util::StreamExt as _;
use rquest::Client;
use serde_json::{Value, json};
use std::sync::Arc;

/// Primary Antigravity API endpoint.
const PRIMARY_URL: &str = "https://daily-cloudcode-pa.googleapis.com";
/// Fallback Antigravity API endpoint.
const FALLBACK_URL: &str = "https://daily-cloudcode-pa.sandbox.googleapis.com";

/// Executor for the Antigravity (Cloud Code) API.
pub struct AntigravityExecutor {
    http: Client,
    api_key: Option<String>,
    auth: Arc<AuthManager>,
}

impl AntigravityExecutor {
    /// Creates a new Antigravity executor with an optional API key and auth manager.
    pub fn new(http: Client, api_key: Option<String>, auth: Arc<AuthManager>) -> Self {
        Self {
            http,
            api_key,
            auth,
        }
    }

    /// Returns the bearer token: API key if present, otherwise fetches an OAuth token.
    async fn bearer_token(&self) -> Result<String> {
        if let Some(key) = &self.api_key {
            return Ok(key.clone());
        }
        let token = self.auth.get_token(&ProviderId::Antigravity).await?;
        Ok(token.access_token)
    }

    /// Sends the request to the primary URL, falling back to the sandbox on failure or 429.
    async fn send_request(
        &self,
        path: &str,
        token: &str,
        body: &Value,
        stream: bool,
    ) -> Result<rquest::Response> {
        let accept = if stream {
            "text/event-stream"
        } else {
            "application/json"
        };

        let primary = format!("{PRIMARY_URL}{path}");
        let result = self
            .http
            .post(&primary)
            .header("authorization", format!("Bearer {token}"))
            .header("user-agent", "antigravity/1.104.0 darwin/arm64")
            .header("content-type", "application/json")
            .header("accept", accept)
            .json(body)
            .send()
            .await;

        match result {
            Ok(r) if r.status().as_u16() != 429 => Ok(r),
            _ => {
                let fallback = format!("{FALLBACK_URL}{path}");
                self.http
                    .post(&fallback)
                    .header("authorization", format!("Bearer {token}"))
                    .header("user-agent", "antigravity/1.104.0 darwin/arm64")
                    .header("content-type", "application/json")
                    .header("accept", accept)
                    .json(body)
                    .send()
                    .await
                    .map_err(ByokError::from)
            }
        }
    }
}

/// Generates a random UUID v4 string.
fn random_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Wraps a translated Gemini request body in the Antigravity envelope.
fn wrap_request(model: &str, gemini_body: &mut Value) -> Value {
    // Remove safety_settings — Antigravity does not support them
    gemini_body
        .as_object_mut()
        .map(|o| o.remove("safety_settings"));

    let uuid = random_uuid();
    let project_id = format!("useful-wave-{}", &uuid[..5]);

    json!({
        "model": model,
        "project": project_id,
        "requestId": format!("agent-{uuid}"),
        "userAgent": "antigravity",
        "requestType": "agent",
        "request": gemini_body,
    })
}

/// Extracts the actual model name from an `ag-` prefixed model identifier.
///
/// e.g. `ag-gemini-2.5-pro` -> `gemini-2.5-pro`, `ag-claude-sonnet-4-5` -> `claude-sonnet-4-5`
fn strip_ag_prefix(model: &str) -> &str {
    model.strip_prefix("ag-").unwrap_or(model)
}

/// Converts a single Gemini streaming chunk (from within the Antigravity envelope)
/// into an `OpenAI` SSE `chat.completion.chunk` line.
fn gemini_chunk_to_openai_sse(chunk: &Value, model: &str) -> Option<String> {
    let candidates = chunk.get("candidates")?.as_array()?;
    let candidate = candidates.first()?;

    let finish_reason = candidate
        .get("finishReason")
        .and_then(Value::as_str)
        .and_then(|r| match r {
            "STOP" => Some("stop"),
            "MAX_TOKENS" => Some("length"),
            _ => None,
        });

    // Extract tool calls from functionCall parts
    let parts = candidate
        .pointer("/content/parts")
        .and_then(Value::as_array);

    let mut delta = json!({});
    let mut has_content = false;

    if let Some(parts) = parts {
        for part in parts {
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                delta["content"] = json!(text);
                has_content = true;
            }
            if let Some(fc) = part.get("functionCall") {
                let name = fc.get("name").and_then(Value::as_str).unwrap_or("");
                let args = fc.get("args").cloned().unwrap_or_else(|| json!({}));
                let tool_call = json!({
                    "index": 0,
                    "id": format!("{name}-{}", &random_uuid()[..8]),
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": args.to_string(),
                    }
                });
                delta["tool_calls"] = json!([tool_call]);
                has_content = true;
            }
        }
    }

    if !has_content && finish_reason.is_none() {
        return None;
    }

    if finish_reason.is_some() && !has_content {
        delta = json!({});
    }

    let sse_chunk = json!({
        "id": "chatcmpl-antigravity",
        "object": "chat.completion.chunk",
        "model": model,
        "choices": [{
            "index": 0,
            "delta": delta,
            "finish_reason": finish_reason,
        }]
    });

    Some(format!(
        "data: {}\n\n",
        serde_json::to_string(&sse_chunk).ok()?
    ))
}

#[async_trait]
impl ProviderExecutor for AntigravityExecutor {
    async fn chat_completion(&self, request: Value, stream: bool) -> Result<ProviderResponse> {
        // Extract model from request, strip ag- prefix for the actual API call
        let model = request.get("model").and_then(Value::as_str).map_or_else(
            || "gemini-2.5-pro".to_string(),
            |m| strip_ag_prefix(m).to_string(),
        );

        // Translate OpenAI -> Gemini format
        let mut gemini_body = OpenAIToGemini.translate_request(request)?;

        // Wrap in Antigravity envelope
        let body = wrap_request(&model, &mut gemini_body);

        let token = self.bearer_token().await?;

        let path = if stream {
            "/v1internal:streamGenerateContent?alt=sse"
        } else {
            "/v1internal:generateContent"
        };

        let resp = self.send_request(path, &token, &body, stream).await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ByokError::Upstream {
                status: status.as_u16(),
                body: text,
            });
        }

        if stream {
            let model_owned = model;
            let byte_stream: ByteStream = Box::pin(resp.bytes_stream().map(move |chunk_result| {
                let chunk_bytes = chunk_result.map_err(ByokError::from)?;
                let text = String::from_utf8_lossy(&chunk_bytes);
                let mut output = String::new();

                for line in text.lines() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    // Strip SSE "data: " prefix if present
                    let json_str = line.strip_prefix("data: ").unwrap_or(line);
                    if json_str == "[DONE]" {
                        output.push_str("data: [DONE]\n\n");
                        continue;
                    }
                    if let Ok(envelope) = serde_json::from_str::<Value>(json_str)
                        && let Some(gemini_chunk) = envelope.get("response")
                        && let Some(sse) = gemini_chunk_to_openai_sse(gemini_chunk, &model_owned)
                    {
                        output.push_str(&sse);
                    }
                }

                Ok(Bytes::from(output))
            }));
            Ok(ProviderResponse::Stream(byte_stream))
        } else {
            let json: Value = resp.json().await?;

            // Extract the `response` field from the Antigravity envelope
            let gemini_response = json.get("response").cloned().unwrap_or(json);

            let translated = GeminiToOpenAI.translate_response(gemini_response)?;
            Ok(ProviderResponse::Complete(translated))
        }
    }

    fn supported_models(&self) -> Vec<String> {
        registry::antigravity_models()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byokey_store::InMemoryTokenStore;

    fn make_executor() -> AntigravityExecutor {
        let store = Arc::new(InMemoryTokenStore::new());
        let auth = Arc::new(AuthManager::new(store));
        AntigravityExecutor::new(Client::new(), None, auth)
    }

    #[test]
    fn test_supported_models_non_empty() {
        let ex = make_executor();
        assert!(!ex.supported_models().is_empty());
    }

    #[test]
    fn test_supported_models_start_with_ag() {
        let ex = make_executor();
        assert!(ex.supported_models().iter().all(|m| m.starts_with("ag-")));
    }

    #[test]
    fn test_strip_ag_prefix() {
        assert_eq!(strip_ag_prefix("ag-gemini-2.5-pro"), "gemini-2.5-pro");
        assert_eq!(strip_ag_prefix("ag-claude-sonnet-4-5"), "claude-sonnet-4-5");
        assert_eq!(strip_ag_prefix("gemini-2.5-pro"), "gemini-2.5-pro");
    }

    #[test]
    fn test_random_uuid_format() {
        let uuid = random_uuid();
        assert_eq!(uuid.len(), 36);
        assert_eq!(uuid.chars().filter(|&c| c == '-').count(), 4);
    }

    #[test]
    fn test_wrap_request_structure() {
        let mut gemini = json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}],
            "generationConfig": {},
            "safety_settings": [{"category": "HARM_CATEGORY_DANGEROUS_CONTENT"}]
        });
        let wrapped = wrap_request("gemini-2.5-pro", &mut gemini);

        assert_eq!(wrapped["model"], "gemini-2.5-pro");
        assert_eq!(wrapped["userAgent"], "antigravity");
        assert_eq!(wrapped["requestType"], "agent");
        assert!(wrapped["requestId"].as_str().unwrap().starts_with("agent-"));
        // safety_settings should be removed
        assert!(wrapped["request"].get("safety_settings").is_none());
        // contents should be present
        assert!(wrapped["request"].get("contents").is_some());
    }

    #[test]
    fn test_gemini_chunk_to_openai_sse_text() {
        let chunk = json!({
            "candidates": [{
                "content": {"parts": [{"text": "Hello"}], "role": "model"},
                "index": 0,
            }]
        });
        let sse = gemini_chunk_to_openai_sse(&chunk, "gemini-2.5-pro").unwrap();
        assert!(sse.starts_with("data: "));
        let data: Value = serde_json::from_str(sse.trim_start_matches("data: ").trim()).unwrap();
        assert_eq!(data["choices"][0]["delta"]["content"], "Hello");
        assert_eq!(data["object"], "chat.completion.chunk");
    }

    #[test]
    fn test_gemini_chunk_to_openai_sse_finish() {
        let chunk = json!({
            "candidates": [{
                "content": {"parts": [], "role": "model"},
                "finishReason": "STOP",
                "index": 0,
            }]
        });
        let sse = gemini_chunk_to_openai_sse(&chunk, "gemini-2.5-pro").unwrap();
        let data: Value = serde_json::from_str(sse.trim_start_matches("data: ").trim()).unwrap();
        assert_eq!(data["choices"][0]["finish_reason"], "stop");
    }

    #[test]
    fn test_gemini_chunk_to_openai_sse_function_call() {
        let chunk = json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "functionCall": {
                            "name": "get_weather",
                            "args": {"location": "NYC"}
                        }
                    }],
                    "role": "model"
                },
                "index": 0,
            }]
        });
        let sse = gemini_chunk_to_openai_sse(&chunk, "gemini-2.5-pro").unwrap();
        let data: Value = serde_json::from_str(sse.trim_start_matches("data: ").trim()).unwrap();
        let tool_call = &data["choices"][0]["delta"]["tool_calls"][0];
        assert_eq!(tool_call["function"]["name"], "get_weather");
    }
}
