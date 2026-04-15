//! Codex (`OpenAI`) executor.
//!
//! Two authentication / API modes:
//!
//! * **API key** (`sk-…`) — standard `OpenAI` Chat Completions API at
//!   `api.openai.com/v1/chat/completions`.  No translation needed.
//!
//! * **OAuth token** (Codex CLI PKCE flow) — private Codex Responses API at
//!   `chatgpt.com/backend-api/codex/responses`.  Request translated with
//!   [`OpenAIToCodex`]; response parsed from SSE and translated with
//!   [`CodexToOpenAI`].
use crate::http_util::ProviderHttp;
use crate::registry;
use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_translate::{CodexToOpenAI, OpenAIToCodex};
use byokey_types::{
    ByokError, ChatRequest, ProviderId, RateLimitStore,
    traits::{
        ByteStream, ProviderExecutor, ProviderResponse, RequestTranslator, ResponseTranslator,
        Result,
    },
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

    /// Translates an `OpenAI` Chat request, sends it to the Codex Responses
    /// API, and returns a streaming `ByteStream` of `OpenAI`-format SSE events.
    async fn codex_stream(&self, body: Value, token: &str) -> Result<ProviderResponse> {
        let mut codex_body = OpenAIToCodex.translate_request(body)?;
        codex_body["stream"] = Value::Bool(true);

        let resp = self.codex_request(&codex_body, token).await?;

        let model = codex_body["model"].as_str().unwrap_or("codex").to_string();

        let raw: ByteStream = ProviderHttp::byte_stream(resp);

        Ok(ProviderResponse::Stream(translate_codex_sse(raw, model)))
    }

    /// Like [`codex_stream`] but collects the full SSE response and extracts
    /// the completed OpenAI-format `Value`.
    async fn codex_complete(&self, body: Value, token: &str) -> Result<ProviderResponse> {
        let mut codex_body = OpenAIToCodex.translate_request(body)?;
        codex_body["stream"] = Value::Bool(true); // Codex always streams

        let resp = self.codex_request(&codex_body, token).await?;

        let mut all = Vec::new();
        let mut stream = resp.bytes_stream().map_err(ByokError::from);
        while let Some(chunk) = stream.try_next().await? {
            all.extend_from_slice(&chunk);
        }

        // Find the response.completed SSE event
        for line in String::from_utf8_lossy(&all).lines() {
            if let Some(data) = line.strip_prefix("data: ")
                && let Ok(ev) = serde_json::from_str::<Value>(data)
                && ev["type"].as_str() == Some("response.completed")
            {
                let response = ev["response"].clone();
                let translated = CodexToOpenAI.translate_response(response)?;
                return Ok(ProviderResponse::Complete(translated));
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

/// Wraps a raw Codex SSE `ByteStream` and translates its events to
/// `OpenAI` chat completion chunk SSE format line-by-line.
#[allow(clippy::too_many_lines)]
pub(crate) fn translate_codex_sse(inner: ByteStream, model: String) -> ByteStream {
    struct State {
        inner: ByteStream,
        buf: Vec<u8>,
        model: String,
        done: bool,
        tool_call_index: i64,
        /// Buffered `encrypted_content` from `response.output_item.added`
        /// (reasoning type). Emitted when the thinking block finalizes.
        thinking_signature: Option<String>,
        /// True while a reasoning output item is open (between added and done).
        thinking_block_open: bool,
    }

    Box::pin(try_unfold(
        State {
            inner,
            buf: Vec::new(),
            model,
            done: false,
            tool_call_index: 0,
            thinking_signature: None,
            thinking_block_open: false,
        },
        |mut s| async move {
            loop {
                // Attempt to consume one complete line from the buffer.
                if let Some(nl) = s.buf.iter().position(|&b| b == b'\n') {
                    let raw: Vec<u8> = s.buf.drain(..=nl).collect();
                    let line = String::from_utf8_lossy(&raw);
                    let line = line.trim_end_matches(['\r', '\n']);

                    if let Some(data) = line.strip_prefix("data: ")
                        && let Ok(ev) = serde_json::from_str::<Value>(data)
                    {
                        match ev["type"].as_str().unwrap_or("") {
                            // Capture encrypted_content when a reasoning item starts.
                            "response.output_item.added"
                                if ev.pointer("/item/type").and_then(Value::as_str)
                                    == Some("reasoning") =>
                            {
                                s.thinking_block_open = true;
                                s.thinking_signature = ev
                                    .pointer("/item/encrypted_content")
                                    .and_then(Value::as_str)
                                    .map(String::from);
                                continue;
                            }
                            // Finalize reasoning block — emit buffered signature.
                            "response.output_item.done"
                                if ev.pointer("/item/type").and_then(Value::as_str)
                                    == Some("reasoning") =>
                            {
                                s.thinking_block_open = false;
                                if let Some(sig) = s.thinking_signature.take() {
                                    let chunk = serde_json::json!({
                                        "object": "chat.completion.chunk",
                                        "model": &s.model,
                                        "choices": [{
                                            "index": 0,
                                            "delta": {"reasoning_signature": sig},
                                            "finish_reason": null
                                        }]
                                    });
                                    let line = format!("data: {chunk}\n\n");
                                    return Ok(Some((Bytes::from(line), s)));
                                }
                                continue;
                            }
                            "response.reasoning_summary_text.delta" => {
                                let delta = ev["delta"].as_str().unwrap_or("").to_string();
                                let chunk = serde_json::json!({
                                    "object": "chat.completion.chunk",
                                    "model": &s.model,
                                    "choices": [{
                                        "index": 0,
                                        "delta": {"reasoning_content": delta},
                                        "finish_reason": null
                                    }]
                                });
                                let line = format!("data: {chunk}\n\n");
                                return Ok(Some((Bytes::from(line), s)));
                            }
                            "response.output_text.delta" => {
                                let delta = ev["delta"].as_str().unwrap_or("").to_string();
                                let chunk = serde_json::json!({
                                    "object": "chat.completion.chunk",
                                    "model": &s.model,
                                    "choices": [{
                                        "index": 0,
                                        "delta": {"content": delta},
                                        "finish_reason": null
                                    }]
                                });
                                let line = format!("data: {chunk}\n\n");
                                return Ok(Some((Bytes::from(line), s)));
                            }
                            "response.output_item.added"
                                if ev.pointer("/item/type").and_then(Value::as_str)
                                    == Some("function_call") =>
                            {
                                // Flush pending thinking signature before starting tool calls.
                                let mut prefix = String::new();
                                if let Some(sig) = s.thinking_signature.take() {
                                    s.thinking_block_open = false;
                                    let sig_chunk = serde_json::json!({
                                        "object": "chat.completion.chunk",
                                        "model": &s.model,
                                        "choices": [{
                                            "index": 0,
                                            "delta": {"reasoning_signature": sig},
                                            "finish_reason": null
                                        }]
                                    });
                                    prefix = format!("data: {sig_chunk}\n\n");
                                }
                                let id = ev
                                    .pointer("/item/call_id")
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                                    .to_string();
                                let name = ev
                                    .pointer("/item/name")
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                                    .to_string();
                                let idx = s.tool_call_index;
                                s.tool_call_index += 1;
                                let chunk = serde_json::json!({
                                    "object": "chat.completion.chunk",
                                    "model": &s.model,
                                    "choices": [{"index": 0, "delta": {
                                        "tool_calls": [{"index": idx, "id": id, "type": "function", "function": {"name": name, "arguments": ""}}]
                                    }, "finish_reason": null}]
                                });
                                let line = format!("{prefix}data: {chunk}\n\n");
                                return Ok(Some((Bytes::from(line), s)));
                            }
                            "response.function_call_arguments.delta" => {
                                let delta = ev["delta"].as_str().unwrap_or("").to_string();
                                let idx = (s.tool_call_index - 1).max(0);
                                let chunk = serde_json::json!({
                                    "object": "chat.completion.chunk",
                                    "model": &s.model,
                                    "choices": [{"index": 0, "delta": {
                                        "tool_calls": [{"index": idx, "function": {"arguments": delta}}]
                                    }, "finish_reason": null}]
                                });
                                let line = format!("data: {chunk}\n\n");
                                return Ok(Some((Bytes::from(line), s)));
                            }
                            "response.completed" => {
                                let mut chunks = Vec::new();
                                // If tool calls were emitted, send a finish chunk
                                if s.tool_call_index > 0 {
                                    let finish = serde_json::json!({
                                        "object": "chat.completion.chunk",
                                        "model": &s.model,
                                        "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]
                                    });
                                    chunks.extend_from_slice(
                                        format!("data: {finish}\n\n").as_bytes(),
                                    );
                                }
                                // Emit usage chunk from the completed response
                                let input_tokens = ev
                                    .pointer("/response/usage/input_tokens")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(0);
                                let output_tokens = ev
                                    .pointer("/response/usage/output_tokens")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(0);
                                let usage_chunk = serde_json::json!({
                                    "object": "chat.completion.chunk",
                                    "model": &s.model,
                                    "choices": [],
                                    "usage": {
                                        "prompt_tokens": input_tokens,
                                        "completion_tokens": output_tokens,
                                        "total_tokens": input_tokens + output_tokens
                                    }
                                });
                                chunks.extend_from_slice(
                                    format!("data: {usage_chunk}\n\n").as_bytes(),
                                );
                                chunks.extend_from_slice(b"data: [DONE]\n\n");
                                s.done = true;
                                return Ok(Some((Bytes::from(chunks), s)));
                            }
                            _ => {}
                        }
                    }
                    continue;
                }

                if s.done {
                    return Ok(None);
                }

                // Buffer more data from the upstream stream.
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
