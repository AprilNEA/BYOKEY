//! Codex (`OpenAI`) executor.
//!
//! Two authentication / API modes:
//!
//! * **API key** (`sk-…`) — standard `OpenAI` Chat Completions API at
//!   `api.openai.com/v1/chat/completions`.  No translation needed.
//!
//! * **OAuth token** (Codex CLI PKCE flow) — private Codex Responses API at
//!   `chatgpt.com/backend-api/codex/responses`.  Request and response translated
//!   via [`aigw_openai`]'s Responses API helpers
//!   ([`build_responses_create_request`], [`ResponsesResponseTranslator`],
//!   [`ResponsesStreamParser`]) with the Codex preset config.
use crate::http_util::ProviderHttp;
use crate::registry;
use aigw_core::translate::{ResponseTranslator as _, StreamParser as _};
use aigw_openai::{
    ResponsesRequestConfig, ResponsesResponseTranslator, ResponsesStreamParser,
    build_responses_create_request,
};
use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_types::{
    ByokError, ChatRequest, ProviderId, RateLimitStore,
    traits::{ByteStream, ProviderExecutor, ProviderResponse, Result},
};
use bytes::Bytes;
use futures_util::{StreamExt as _, TryStreamExt as _, stream::try_unfold};
use rquest::Client;
use serde_json::Value;
use std::sync::Arc;

/// Default `OpenAI` API base URL.
const DEFAULT_OPENAI_BASE_URL: &str = "https://api.openai.com";
/// Chat completions API path.
const OPENAI_API_PATH: &str = "/v1/chat/completions";

/// Codex CLI Responses endpoint (used with OAuth tokens).
const CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api/codex";

/// Default User-Agent (compile-time fallback).
const DEFAULT_USER_AGENT: &str = "codex-tui/0.120.0 (Mac OS 26.0.1; arm64) Apple_Terminal/464";

/// Executor for the `OpenAI` (Codex) API.
pub struct CodexExecutor {
    ph: ProviderHttp,
    api_key: Option<String>,
    openai_api_url: String,
    auth: Arc<AuthManager>,
    user_agent: String,
}

#[bon::bon]
impl CodexExecutor {
    /// Creates a new Codex executor.
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
            ph = ph.with_ratelimit(store, ProviderId::Codex);
        }
        let openai_api_url = format!(
            "{}{}",
            base_url
                .as_deref()
                .unwrap_or(DEFAULT_OPENAI_BASE_URL)
                .trim_end_matches('/'),
            OPENAI_API_PATH
        );
        Self {
            ph,
            api_key,
            openai_api_url,
            auth,
            user_agent: user_agent.unwrap_or_else(|| DEFAULT_USER_AGENT.to_string()),
        }
    }

    /// Returns `(token, is_oauth)`.  `is_oauth = true` when the token came
    /// from the device/PKCE flow rather than a raw API key.
    async fn token(&self) -> Result<(String, bool)> {
        if let Some(key) = &self.api_key {
            return Ok((key.clone(), false));
        }
        let tok = self.auth.get_token(&ProviderId::Codex).await?;
        Ok((tok.access_token, true))
    }

    // ── OAuth / Codex Responses API path ─────────────────────────────────────

    /// Issues a Codex Responses API request and returns raw bytes + HTTP status.
    async fn codex_request(&self, body: &Value, token: &str) -> Result<rquest::Response> {
        let url = format!("{CODEX_BASE_URL}/responses");
        let session_id = random_uuid();
        let builder = self
            .ph
            .client()
            .post(&url)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .header("Session_id", session_id)
            .header("User-Agent", self.user_agent.as_str())
            .header("Originator", "codex_cli_rs")
            .header("Accept", "text/event-stream")
            .header("Connection", "Keep-Alive")
            .json(body);
        self.ph.send(builder).await
    }

    /// Translate a `ChatRequest` body `Value` to the Codex Responses API JSON
    /// body using [`aigw_openai::build_responses_create_request`] with the
    /// Codex preset config.
    fn translate_body(body: Value) -> Result<Value> {
        let aigw_request: aigw_core::model::ChatRequest = serde_json::from_value(body)
            .map_err(|e: serde_json::Error| ByokError::Translation(e.to_string()))?;
        let responses_req =
            build_responses_create_request(&aigw_request, &ResponsesRequestConfig::codex())
                .map_err(|e| ByokError::Translation(e.to_string()))?;
        serde_json::to_value(&responses_req)
            .map_err(|e: serde_json::Error| ByokError::Translation(e.to_string()))
    }

    /// Translates an `OpenAI` Chat request, sends it to the Codex Responses
    /// API, and returns a streaming `ByteStream` of `OpenAI`-format SSE events.
    async fn codex_stream(&self, body: Value, token: &str) -> Result<ProviderResponse> {
        let mut codex_body = Self::translate_body(body)?;
        codex_body["stream"] = Value::Bool(true);

        let resp = self.codex_request(&codex_body, token).await?;

        let raw: ByteStream = ProviderHttp::byte_stream(resp);

        Ok(ProviderResponse::Stream(translate_codex_responses_sse(raw)))
    }

    /// Like [`codex_stream`] but collects the full SSE response and extracts
    /// the completed OpenAI-format `Value`.
    async fn codex_complete(&self, body: Value, token: &str) -> Result<ProviderResponse> {
        let mut codex_body = Self::translate_body(body)?;
        codex_body["stream"] = Value::Bool(true); // Codex always streams

        let resp = self.codex_request(&codex_body, token).await?;

        let mut all = Vec::new();
        let mut stream = resp.bytes_stream().map_err(ByokError::from);
        while let Some(chunk) = stream.try_next().await? {
            all.extend_from_slice(&chunk);
        }

        // Find the `response.completed` SSE event and translate the inner
        // `response` object via aigw_openai's ResponsesResponseTranslator.
        for line in String::from_utf8_lossy(&all).lines() {
            if let Some(data) = line.strip_prefix("data: ")
                && let Ok(ev) = serde_json::from_str::<Value>(data)
                && ev["type"].as_str() == Some("response.completed")
            {
                let response = ev["response"].clone();
                let resp_bytes = serde_json::to_vec(&response)
                    .map_err(|e: serde_json::Error| ByokError::Translation(e.to_string()))?;
                let chat_resp = ResponsesResponseTranslator
                    .translate_response(http::StatusCode::OK, &resp_bytes)
                    .map_err(|e| ByokError::Translation(e.to_string()))?;
                let mut value = serde_json::to_value(&chat_resp)
                    .map_err(|e: serde_json::Error| ByokError::Translation(e.to_string()))?;
                // Prefix response id with `chatcmpl-` to match BYOKEY's
                // legacy CodexToOpenAI behaviour.
                if let Some(id) = value.get("id").and_then(Value::as_str) {
                    value["id"] = Value::String(format!("chatcmpl-{id}"));
                }
                return Ok(ProviderResponse::Complete(value));
            }
        }

        Err(ByokError::Http(
            "Codex: response.completed event not found in stream".into(),
        ))
    }
}

/// Generates a deterministic prompt cache key from an API key using UUID v5.
fn prompt_cache_key(api_key: &str) -> String {
    let seed = format!("cli-proxy-api:codex:prompt-cache:{api_key}");
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, seed.as_bytes()).to_string()
}

/// Generates a random UUID v4 string.
fn random_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Wraps a raw Codex Responses API SSE `ByteStream` and translates each event
/// to an `OpenAI` chat-completion-chunk SSE line.
///
/// Delegates semantic parsing to [`aigw_openai::ResponsesStreamParser`], then
/// converts the canonical `StreamEvent`s to `OpenAI` SSE bytes via the shared
/// [`stream_bridge`](crate::stream_bridge) helpers.
pub(crate) fn translate_codex_responses_sse(inner: ByteStream) -> ByteStream {
    use crate::stream_bridge::{SseContext, stream_events_to_sse};

    struct State {
        inner: ByteStream,
        buf: Vec<u8>,
        parser: ResponsesStreamParser,
        ctx: SseContext,
        done: bool,
    }

    Box::pin(try_unfold(
        State {
            inner,
            buf: Vec::new(),
            parser: ResponsesStreamParser::new(),
            ctx: SseContext::default(),
            done: false,
        },
        |mut s| async move {
            loop {
                if let Some(nl) = s.buf.iter().position(|&b| b == b'\n') {
                    let raw: Vec<u8> = s.buf.drain(..=nl).collect();
                    let line = String::from_utf8_lossy(&raw);
                    let line = line.trim_end_matches(['\r', '\n']);

                    if let Some(data) = line.strip_prefix("data: ") {
                        match s.parser.parse_event("", data) {
                            Ok(events) if !events.is_empty() => {
                                let sse_bytes = stream_events_to_sse(&events, &mut s.ctx);
                                if !sse_bytes.is_empty() {
                                    if events
                                        .iter()
                                        .any(|e| matches!(e, aigw_core::model::StreamEvent::Done))
                                    {
                                        s.done = true;
                                    }
                                    return Ok(Some((Bytes::from(sse_bytes), s)));
                                }
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "codex responses stream parse error");
                            }
                            _ => {}
                        }
                    }
                    continue;
                }

                if s.done {
                    return Ok(None);
                }

                match s.inner.next().await {
                    Some(Ok(b)) => s.buf.extend_from_slice(&b),
                    Some(Err(e)) => return Err(e),
                    None => return Ok(None),
                }
            }
        },
    ))
}

#[async_trait]
impl ProviderExecutor for CodexExecutor {
    async fn chat_completion(&self, request: ChatRequest) -> Result<ProviderResponse> {
        let (token, is_oauth) = self.token().await?;
        let stream = request.stream;

        if is_oauth {
            let body = request.into_body();
            if stream {
                return self.codex_stream(body, &token).await;
            }
            return self.codex_complete(body, &token).await;
        }

        // API key → standard OpenAI Chat Completions
        let mut body = request.into_body();
        let cache_key = prompt_cache_key(&token);
        body["prompt_cache_key"] = Value::String(cache_key.clone());
        let builder = self
            .ph
            .client()
            .post(&self.openai_api_url)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .header("Conversation_id", &cache_key)
            .header("Session_id", &cache_key)
            .json(&body);

        self.ph.send_passthrough(builder, stream).await
    }

    fn supported_models(&self) -> Vec<String> {
        registry::models_for_provider(&ProviderId::Codex)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_executor() -> CodexExecutor {
        let (client, auth) = crate::http_util::test_auth();
        CodexExecutor::builder().http(client).auth(auth).build()
    }

    #[test]
    fn test_supported_models_non_empty() {
        let ex = make_executor();
        assert!(!ex.supported_models().is_empty());
    }

    #[test]
    fn test_supported_models_contains_o4_mini() {
        let ex = make_executor();
        assert!(ex.supported_models().iter().any(|m| m == "o4-mini"));
    }
}
