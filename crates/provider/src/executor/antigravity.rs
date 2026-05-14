//! Antigravity executor — Google Cloud Code (`CLIProxyAPIPlus`) backend.
//!
//! Antigravity uses a Gemini-compatible request/response format wrapped in an
//! envelope with additional metadata fields. Streaming responses arrive as
//! JSON lines (not SSE), each containing a `response` field with a Gemini
//! stream chunk.

use crate::http_util::ProviderHttp;
use crate::registry;
use crate::stream_bridge::{SseContext, stream_events_to_sse};
use aigw_core::translate::ResponseTranslator as _;
use aigw_core::translate::StreamParser as _;
use aigw_gemini::translate::{
    GeminiResponseTranslator, GeminiStreamParser, build_generate_content_request,
};
use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_types::{
    ByokError, ChatRequest, ProviderId, RateLimitStore,
    traits::{ByteStream, ProviderExecutor, ProviderResponse, Result},
};
use bytes::Bytes;
use futures_util::{StreamExt as _, stream::try_unfold};
use http::StatusCode;
use rquest::Client;
use serde_json::{Value, json};
use std::sync::Arc;

/// Default primary Antigravity API base URL.
const DEFAULT_PRIMARY_URL: &str = "https://daily-cloudcode-pa.googleapis.com";
/// Fallback Antigravity API base URL.
const FALLBACK_URL: &str = "https://daily-cloudcode-pa.sandbox.googleapis.com";

/// Default user-agent (compile-time fallback).
const DEFAULT_USER_AGENT: &str = "antigravity/1.20.5 darwin/arm64";

/// Executor for the Antigravity (Cloud Code) API.
pub struct AntigravityExecutor {
    ph: ProviderHttp,
    api_key: Option<String>,
    primary_url: String,
    auth: Arc<AuthManager>,
    user_agent: String,
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
        user_agent: Option<String>,
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
            user_agent: user_agent.unwrap_or_else(|| DEFAULT_USER_AGENT.to_string()),
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

    /// Sends the request to the primary URL, falling back to the sandbox on 429 or error.
    ///
    /// Routes through [`ProviderHttp::send`] so that rate-limit headers and
    /// retry-after body parsing (Google `RetryInfo` / `ErrorInfo`) are handled.
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
                .header("user-agent", self.user_agent.as_str())
                .header("content-type", "application/json")
                .header("accept", accept)
                .json(body)
        };

        match self.ph.send(build_request(&self.primary_url)).await {
            Ok(r) => Ok(r),
            Err(e) if e.is_retryable() => {
                // Fallback to sandbox URL on transient errors.
                self.ph.send(build_request(FALLBACK_URL)).await
            }
            Err(e) => Err(e),
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

/// Translate Antigravity's enveloped Gemini stream into `OpenAI` Chat SSE.
fn translate_antigravity_stream(inner: ByteStream, model: String) -> ByteStream {
    struct State {
        inner: ByteStream,
        buf: Vec<u8>,
        parser: GeminiStreamParser,
        ctx: SseContext,
        done: bool,
    }

    Box::pin(try_unfold(
        State {
            inner,
            buf: Vec::new(),
            parser: GeminiStreamParser::new(),
            ctx: SseContext {
                id: "chatcmpl-antigravity".to_owned(),
                model,
            },
            done: false,
        },
        |mut s| async move {
            loop {
                if let Some(nl) = s.buf.iter().position(|&b| b == b'\n') {
                    let raw: Vec<u8> = s.buf.drain(..=nl).collect();
                    let line = String::from_utf8_lossy(&raw);
                    let line = line.trim_end_matches(['\r', '\n']).trim();
                    if line.is_empty() {
                        continue;
                    }

                    let data = line.strip_prefix("data: ").unwrap_or(line);
                    let events = if data == "[DONE]" {
                        s.parser.parse_event("", data)
                    } else {
                        match serde_json::from_str::<Value>(data) {
                            Ok(envelope) => {
                                let gemini_chunk = envelope.get("response").unwrap_or(&envelope);
                                s.parser.parse_event("", &gemini_chunk.to_string())
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "antigravity stream envelope parse error");
                                continue;
                            }
                        }
                    };

                    match events {
                        Ok(events) if !events.is_empty() => {
                            if events
                                .iter()
                                .any(|e| matches!(e, aigw_core::model::StreamEvent::Done))
                            {
                                s.done = true;
                            }
                            let bytes = stream_events_to_sse(&events, &mut s.ctx);
                            if !bytes.is_empty() {
                                return Ok(Some((Bytes::from(bytes), s)));
                            }
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "antigravity stream parse error");
                        }
                        _ => {}
                    }
                    continue;
                }

                if s.done {
                    return Ok(None);
                }

                match s.inner.next().await {
                    Some(Ok(b)) => s.buf.extend_from_slice(&b),
                    Some(Err(e)) => return Err(e),
                    None => {
                        let events = s
                            .parser
                            .finish()
                            .map_err(|e| ByokError::Translation(e.to_string()))?;
                        let bytes = stream_events_to_sse(&events, &mut s.ctx);
                        if bytes.is_empty() {
                            return Ok(None);
                        }
                        s.done = true;
                        return Ok(Some((Bytes::from(bytes), s)));
                    }
                }
            }
        },
    ))
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

        // Translate OpenAI/canonical → Gemini-native via aigw-gemini, then
        // serialise back to a Value so the Antigravity envelope can wrap it.
        let mut canonical: aigw_core::model::ChatRequest =
            serde_json::from_value(body).map_err(|e| ByokError::Translation(e.to_string()))?;
        // Use the bare model in the canonical body — aigw will write it back
        // into the URL, but the envelope path doesn't use the URL anyway.
        canonical.model = model.clone();
        let native = build_generate_content_request(&canonical)
            .map_err(|e| ByokError::Translation(e.to_string()))?;
        let mut gemini_body =
            serde_json::to_value(&native).map_err(|e| ByokError::Translation(e.to_string()))?;

        // Wrap in Antigravity envelope
        let body = wrap_request(&model, &mut gemini_body);

        let token = self.bearer_token().await?;

        let path = if stream {
            "/v1internal:streamGenerateContent?alt=sse"
        } else {
            "/v1internal:generateContent"
        };

        let resp = self.send_request(path, &token, &body, stream).await?;

        if stream {
            let byte_stream: ByteStream =
                Box::pin(resp.bytes_stream().map(|r| r.map_err(ByokError::from)));
            Ok(ProviderResponse::Stream(translate_antigravity_stream(
                byte_stream,
                model,
            )))
        } else {
            let json: Value = resp.json().await?;

            // Extract the `response` field from the Antigravity envelope
            let gemini_response = json.get("response").cloned().unwrap_or(json);

            // Hand the inner Gemini-format response to aigw-gemini for
            // canonical translation, then serialise the canonical
            // ChatResponse back to a Value (BYOKEY's executor trait).
            let bytes = serde_json::to_vec(&gemini_response)
                .map_err(|e| ByokError::Translation(e.to_string()))?;
            let canonical = GeminiResponseTranslator
                .translate_response(StatusCode::OK, &bytes)
                .map_err(|e| ByokError::Translation(e.to_string()))?;
            let translated = serde_json::to_value(&canonical)
                .map_err(|e| ByokError::Translation(e.to_string()))?;
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

    async fn collect_stream_text(stream: ByteStream) -> String {
        let chunks: Vec<Bytes> = stream
            .map(|r| r.expect("stream chunk should be ok"))
            .collect()
            .await;
        String::from_utf8(
            chunks
                .into_iter()
                .flat_map(|b| b.to_vec())
                .collect::<Vec<_>>(),
        )
        .expect("stream should be utf8")
    }

    #[tokio::test]
    async fn antigravity_stream_text_uses_gemini_parser() {
        let line = json!({
            "response": {
                "candidates": [{
                    "content": {"parts": [{"text": "Hello"}], "role": "model"},
                    "index": 0
                }],
                "responseId": "ag-1",
                "modelVersion": "gemini-2.5-pro"
            }
        });
        let input: ByteStream = Box::pin(futures_util::stream::iter(vec![Ok(Bytes::from(
            format!("data: {line}\n"),
        ))]));
        let out =
            collect_stream_text(translate_antigravity_stream(input, "gemini-2.5-pro".into())).await;

        assert!(out.contains(r#""content":"Hello""#));
        assert!(out.contains("data: [DONE]"));
    }

    #[tokio::test]
    async fn antigravity_stream_thought_becomes_reasoning_delta() {
        let line = json!({
            "response": {
                "candidates": [{
                    "content": {
                        "parts": [
                            {"text": "thinking", "thought": true, "thoughtSignature": "sig"},
                            {"text": "answer"}
                        ],
                        "role": "model"
                    },
                    "index": 0
                }],
                "responseId": "ag-1"
            }
        });
        let input: ByteStream = Box::pin(futures_util::stream::iter(vec![Ok(Bytes::from(
            format!("{line}\n"),
        ))]));
        let out =
            collect_stream_text(translate_antigravity_stream(input, "gemini-2.5-pro".into())).await;

        assert!(out.contains(r#""reasoning_content":"thinking""#));
        assert!(out.contains(r#""reasoning_signature":"sig""#));
        assert!(out.contains(r#""content":"answer""#));
    }

    #[tokio::test]
    async fn antigravity_stream_finish_usage_and_tool_calls() {
        let tool = json!({
            "response": {
                "candidates": [{
                    "content": {
                        "parts": [{
                            "functionCall": {
                                "name": "get_weather",
                                "args": {"location": "NYC"},
                                "id": "fc1"
                            }
                        }],
                        "role": "model"
                    },
                    "finishReason": "STOP",
                    "index": 0
                }],
                "usageMetadata": {
                    "promptTokenCount": 3,
                    "candidatesTokenCount": 2,
                    "totalTokenCount": 5
                }
            }
        });
        let input: ByteStream = Box::pin(futures_util::stream::iter(vec![Ok(Bytes::from(
            format!("data: {tool}\n"),
        ))]));
        let out =
            collect_stream_text(translate_antigravity_stream(input, "gemini-2.5-pro".into())).await;

        assert!(out.contains(r#""name":"get_weather""#));
        assert!(out.contains(r#""arguments":"{\"location\":\"NYC\"}""#));
        assert!(out.contains(r#""finish_reason":"stop""#));
        assert!(out.contains(r#""prompt_tokens":3"#));
        assert!(out.contains("data: [DONE]"));
    }
}
