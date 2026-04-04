//! Antigravity executor — Google Cloud Code (`CLIProxyAPIPlus`) backend.
//!
//! Antigravity uses a Gemini-compatible request/response format wrapped in an
//! envelope with additional metadata fields. Streaming responses arrive as
//! JSON lines (not SSE), each containing a `response` field with a Gemini
//! stream chunk.

use std::fmt::Write as _;

use crate::http_util::ProviderHttp;
use crate::registry;
use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_translate::{GeminiToOpenAI, OpenAIToGemini};
use byokey_types::{
    ByokError, ChatRequest, ProviderId, RateLimitStore, RequestTranslator, ResponseTranslator,
    traits::{ByteStream, ProviderExecutor, ProviderResponse, Result},
};
use bytes::Bytes;
use futures_util::StreamExt as _;
use rquest::Client;
use serde_json::{Value, json};
use std::sync::Arc;

/// Default primary Antigravity API base URL.
const DEFAULT_PRIMARY_URL: &str = "https://daily-cloudcode-pa.googleapis.com";
/// Fallback Antigravity API base URL.
const FALLBACK_URL: &str = "https://daily-cloudcode-pa.sandbox.googleapis.com";

/// Executor for the Antigravity (Cloud Code) API.
pub struct AntigravityExecutor {
    ph: ProviderHttp,
    api_key: Option<String>,
    primary_url: String,
    auth: Arc<AuthManager>,
}

#[bon::bon]
impl AntigravityExecutor {
    /// Creates a new Antigravity executor.
    #[builder]
    #[allow(clippy::needless_pass_by_value)]
    pub fn new(
        http: Client,
        auth: Arc<AuthManager>,
        api_key: Option<String>,
        base_url: Option<String>,
        ratelimit: Option<Arc<RateLimitStore>>,
    ) -> Self {
        let mut ph = ProviderHttp::new(http);
        if let Some(store) = ratelimit {
            ph = ph.with_ratelimit(store, ProviderId::Antigravity);
        }
        let primary_url = base_url
            .as_deref()
            .unwrap_or(DEFAULT_PRIMARY_URL)
            .trim_end_matches('/')
            .to_string();
        Self {
            ph,
            api_key,
            primary_url,
            auth,
        }
    }

    /// Returns the bearer token: API key if present, otherwise fetches an OAuth token.
    async fn bearer_token(&self) -> Result<String> {
        crate::http_util::resolve_bearer_token(
            self.api_key.as_deref(),
            &self.auth,
            &ProviderId::Antigravity,
        )
        .await
    }

    /// Sends the request to the primary URL, falling back to the sandbox on failure or 429.
    async fn send_request(
        &self,
        path: &str,
        token: &str,
        body: &Value,
        stream: bool,
    ) -> Result<rquest::Response> {
        let accept = crate::http_util::accept_for_stream(stream);
        let auth_value = format!("Bearer {token}");

        let build_request = |base_url: &str| {
            let url = format!("{base_url}{path}");
            self.ph
                .client()
                .post(url)
                .header("authorization", &auth_value)
                .header("user-agent", "antigravity/1.104.0 darwin/arm64")
                .header("content-type", "application/json")
                .header("accept", accept)
                .json(body)
        };

        let result = build_request(&self.primary_url).send().await;

        match result {
            Ok(r) if r.status().as_u16() != 429 => Ok(r),
            _ => build_request(FALLBACK_URL)
                .send()
                .await
                .map_err(ByokError::from),
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
    let mut output = String::new();

    // Build main content chunk from candidates
    if let Some(candidates) = chunk.get("candidates").and_then(Value::as_array)
        && let Some(candidate) = candidates.first()
    {
        let finish_reason = candidate
            .get("finishReason")
            .and_then(Value::as_str)
            .and_then(|r| match r {
                "STOP" => Some("stop"),
                "MAX_TOKENS" => Some("length"),
                _ => None,
            });

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

        if has_content || finish_reason.is_some() {
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
            if let Ok(s) = serde_json::to_string(&sse_chunk) {
                let _ = write!(output, "data: {s}\n\n");
            }
        }
    }

    // Emit usage chunk if usageMetadata is present (typically on the last chunk)
    if let Some(usage_meta) = chunk.get("usageMetadata") {
        let prompt = usage_meta
            .get("promptTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let completion = usage_meta
            .get("candidatesTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let usage_chunk = json!({
            "id": "chatcmpl-antigravity",
            "object": "chat.completion.chunk",
            "model": model,
            "choices": [],
            "usage": {
                "prompt_tokens": prompt,
                "completion_tokens": completion,
                "total_tokens": prompt + completion
            }
        });
        if let Ok(s) = serde_json::to_string(&usage_chunk) {
            let _ = write!(output, "data: {s}\n\n");
        }
    }

    if output.is_empty() {
        None
    } else {
        Some(output)
    }
}

#[async_trait]
impl ProviderExecutor for AntigravityExecutor {
    async fn chat_completion(&self, request: ChatRequest) -> Result<ProviderResponse> {
        let stream = request.stream;
        let body = request.into_body();

        // Extract model from request, strip ag- prefix for the actual API call
        let model = body.get("model").and_then(Value::as_str).map_or_else(
            || "gemini-2.5-pro".to_string(),
            |m| strip_ag_prefix(m).to_string(),
        );

        // Translate OpenAI -> Gemini format
        let mut gemini_body = OpenAIToGemini.translate_request(body)?;

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
                retry_after: None,
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
        registry::models_for_provider(&ProviderId::Antigravity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_executor() -> AntigravityExecutor {
        let (client, auth) = crate::http_util::test_auth();
        AntigravityExecutor::builder()
            .http(client)
            .auth(auth)
            .build()
    }

    #[test]
    fn test_supported_models_non_empty() {
        let ex = make_executor();
        assert!(!ex.supported_models().is_empty());
    }

    #[test]
    fn test_supported_models_start_with_ag() {
        let ex = make_executor();
        // Most Antigravity models are prefixed with "ag-", but shared models
        // like "claude-sonnet-4-5" also appear via REGISTRY.
        let ag_only: Vec<_> = ex
            .supported_models()
            .into_iter()
            .filter(|m| m.starts_with("ag-"))
            .collect();
        assert!(!ag_only.is_empty());
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
