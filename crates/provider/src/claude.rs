//! Claude executor — Anthropic Messages API.
//!
//! Auth: `x-api-key` for raw API keys, `Authorization: Bearer` for OAuth tokens.
//! Format: `OpenAI` -> Anthropic (translate), Anthropic -> `OpenAI` (translate).
use crate::http_util::ProviderHttp;
use crate::registry;
use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_translate::{ClaudeToOpenAI, OpenAIToClaude, inject_cache_control};
use byokey_types::{
    ChatRequest, ProviderId,
    traits::{
        ByteStream, ProviderExecutor, ProviderResponse, RequestTranslator, ResponseTranslator,
        Result,
    },
};
use bytes::Bytes;
use futures_util::{StreamExt as _, stream::try_unfold};
use rquest::Client;
use serde_json::Value;
use std::sync::Arc;

/// Anthropic Messages API endpoint (with beta flag required by the API).
const API_URL: &str = "https://api.anthropic.com/v1/messages?beta=true";

/// Required Anthropic API version header value.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Beta features to enable; `oauth-2025-04-20` is required for OAuth Bearer tokens.
const ANTHROPIC_BETA: &str = "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14,fine-grained-tool-streaming-2025-05-14,prompt-caching-2024-07-31";

/// User-Agent matching the Claude CLI SDK version.
const USER_AGENT: &str = "claude-cli/2.1.44 (external, sdk-cli)";

/// Authentication mode for the Claude API.
enum AuthMode {
    /// Raw API key sent via `x-api-key` header.
    ApiKey(String),
    /// OAuth token sent via `Authorization: Bearer` header.
    Bearer(String),
}

/// Executor for the Anthropic Claude API.
pub struct ClaudeExecutor {
    ph: ProviderHttp,
    api_key: Option<String>,
    auth: Arc<AuthManager>,
}

impl ClaudeExecutor {
    /// Creates a new Claude executor with an optional API key and auth manager.
    pub fn new(http: Client, api_key: Option<String>, auth: Arc<AuthManager>) -> Self {
        Self {
            ph: ProviderHttp::new(http),
            api_key,
            auth,
        }
    }

    /// Resolves the authentication mode: API key if present, otherwise OAuth token.
    async fn get_auth(&self) -> Result<AuthMode> {
        if let Some(key) = &self.api_key {
            return Ok(AuthMode::ApiKey(key.clone()));
        }
        let token = self.auth.get_token(&ProviderId::Claude).await?;
        Ok(AuthMode::Bearer(token.access_token))
    }
}

#[async_trait]
impl ProviderExecutor for ClaudeExecutor {
    async fn chat_completion(&self, request: ChatRequest) -> Result<ProviderResponse> {
        let stream = request.stream;
        let mut body = OpenAIToClaude.translate_request(request.into_body())?;
        body = inject_cache_control(body);
        body["stream"] = Value::Bool(stream);

        let auth = self.get_auth().await?;

        let builder = self
            .ph
            .client()
            .post(API_URL)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("anthropic-beta", ANTHROPIC_BETA)
            .header("anthropic-dangerous-direct-browser-access", "true")
            .header("x-app", "cli")
            .header("user-agent", USER_AGENT)
            .header("content-type", "application/json");

        let builder = match &auth {
            AuthMode::ApiKey(key) => builder.header("x-api-key", key.as_str()),
            AuthMode::Bearer(tok) => builder.header("authorization", format!("Bearer {tok}")),
        };

        let resp = self.ph.send(builder.json(&body)).await?;

        if stream {
            let byte_stream: ByteStream = ProviderHttp::byte_stream(resp);
            Ok(ProviderResponse::Stream(translate_claude_sse(byte_stream)))
        } else {
            let json: Value = resp.json().await?;
            let translated = ClaudeToOpenAI.translate_response(json)?;
            Ok(ProviderResponse::Complete(translated))
        }
    }

    fn supported_models(&self) -> Vec<String> {
        registry::claude_models()
    }
}

/// Wraps a raw Claude SSE `ByteStream` and translates its events to
/// `OpenAI` chat completion chunk SSE format line-by-line.
///
/// Claude SSE events handled:
/// - `message_start`         → extract `id` and `model`; emit first chunk with `role`
/// - `content_block_start`   → for `tool_use` blocks: emit `tool_calls` opening chunk
/// - `content_block_delta`   → `text_delta` → content; `input_json_delta` → tool arguments
/// - `message_delta`         → emit finish chunk (`stop_reason` mapped to `finish_reason`)
/// - `message_stop`          → emit `data: [DONE]`
#[allow(clippy::too_many_lines)]
fn translate_claude_sse(inner: ByteStream) -> ByteStream {
    struct State {
        inner: ByteStream,
        buf: Vec<u8>,
        id: String,
        model: String,
        done: bool,
        /// Index of the most recently started `tool_call`, or -1 if not in a tool block.
        tool_call_index: i64,
        /// Whether the current content block is a `tool_use` block.
        in_tool_use: bool,
    }

    Box::pin(try_unfold(
        State {
            inner,
            buf: Vec::new(),
            id: "chatcmpl-claude".to_string(),
            model: "claude".to_string(),
            done: false,
            tool_call_index: -1,
            in_tool_use: false,
        },
        |mut s| async move {
            loop {
                if let Some(nl) = s.buf.iter().position(|&b| b == b'\n') {
                    let raw: Vec<u8> = s.buf.drain(..=nl).collect();
                    let line = String::from_utf8_lossy(&raw);
                    let line = line.trim_end_matches(['\r', '\n']);

                    if let Some(data) = line.strip_prefix("data: ")
                        && let Ok(ev) = serde_json::from_str::<Value>(data)
                    {
                        match ev["type"].as_str().unwrap_or("") {
                            "message_start" => {
                                if let Some(id) = ev.pointer("/message/id").and_then(Value::as_str)
                                {
                                    s.id = format!("chatcmpl-{id}");
                                }
                                if let Some(model) =
                                    ev.pointer("/message/model").and_then(Value::as_str)
                                {
                                    s.model = model.to_string();
                                }
                                let chunk = serde_json::json!({
                                    "id": &s.id,
                                    "object": "chat.completion.chunk",
                                    "model": &s.model,
                                    "choices": [{
                                        "index": 0,
                                        "delta": {"role": "assistant", "content": ""},
                                        "finish_reason": null
                                    }]
                                });
                                return Ok(Some((Bytes::from(format!("data: {chunk}\n\n")), s)));
                            }
                            "content_block_start" => {
                                let block_type = ev
                                    .pointer("/content_block/type")
                                    .and_then(Value::as_str)
                                    .unwrap_or("");
                                if block_type == "tool_use" {
                                    s.in_tool_use = true;
                                    s.tool_call_index += 1;
                                    let id = ev
                                        .pointer("/content_block/id")
                                        .and_then(Value::as_str)
                                        .unwrap_or("")
                                        .to_string();
                                    let name = ev
                                        .pointer("/content_block/name")
                                        .and_then(Value::as_str)
                                        .unwrap_or("")
                                        .to_string();
                                    let idx = s.tool_call_index;
                                    let chunk = serde_json::json!({
                                        "id": &s.id,
                                        "object": "chat.completion.chunk",
                                        "model": &s.model,
                                        "choices": [{"index": 0, "delta": {
                                            "tool_calls": [{"index": idx, "id": id, "type": "function", "function": {"name": name, "arguments": ""}}]
                                        }, "finish_reason": null}]
                                    });
                                    return Ok(Some((
                                        Bytes::from(format!("data: {chunk}\n\n")),
                                        s,
                                    )));
                                }
                                s.in_tool_use = false;
                            }
                            "content_block_delta" => {
                                let delta_type = ev
                                    .pointer("/delta/type")
                                    .and_then(Value::as_str)
                                    .unwrap_or("");
                                if delta_type == "text_delta" {
                                    let text = ev
                                        .pointer("/delta/text")
                                        .and_then(Value::as_str)
                                        .unwrap_or("");
                                    let chunk = serde_json::json!({
                                        "id": &s.id,
                                        "object": "chat.completion.chunk",
                                        "model": &s.model,
                                        "choices": [{
                                            "index": 0,
                                            "delta": {"content": text},
                                            "finish_reason": null
                                        }]
                                    });
                                    return Ok(Some((
                                        Bytes::from(format!("data: {chunk}\n\n")),
                                        s,
                                    )));
                                } else if delta_type == "input_json_delta" && s.in_tool_use {
                                    let partial = ev
                                        .pointer("/delta/partial_json")
                                        .and_then(Value::as_str)
                                        .unwrap_or("");
                                    let idx = s.tool_call_index;
                                    let chunk = serde_json::json!({
                                        "id": &s.id,
                                        "object": "chat.completion.chunk",
                                        "model": &s.model,
                                        "choices": [{"index": 0, "delta": {
                                            "tool_calls": [{"index": idx, "function": {"arguments": partial}}]
                                        }, "finish_reason": null}]
                                    });
                                    return Ok(Some((
                                        Bytes::from(format!("data: {chunk}\n\n")),
                                        s,
                                    )));
                                }
                            }
                            "message_delta" => {
                                let finish_reason = match ev
                                    .pointer("/delta/stop_reason")
                                    .and_then(Value::as_str)
                                {
                                    Some("max_tokens") => "length",
                                    Some("tool_use") => "tool_calls",
                                    _ => "stop",
                                };
                                let chunk = serde_json::json!({
                                    "id": &s.id,
                                    "object": "chat.completion.chunk",
                                    "model": &s.model,
                                    "choices": [{
                                        "index": 0,
                                        "delta": {},
                                        "finish_reason": finish_reason
                                    }]
                                });
                                return Ok(Some((Bytes::from(format!("data: {chunk}\n\n")), s)));
                            }
                            "message_stop" => {
                                s.done = true;
                                return Ok(Some((Bytes::from("data: [DONE]\n\n"), s)));
                            }
                            _ => {} // ping, content_block_stop
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

#[cfg(test)]
mod tests {
    use super::*;
    use byokey_store::InMemoryTokenStore;

    fn make_executor() -> ClaudeExecutor {
        let store = Arc::new(InMemoryTokenStore::new());
        let auth = Arc::new(AuthManager::new(store));
        ClaudeExecutor::new(Client::new(), None, auth)
    }

    #[test]
    fn test_supported_models_non_empty() {
        let ex = make_executor();
        let models = ex.supported_models();
        assert!(!models.is_empty());
        assert!(models.iter().all(|m| m.starts_with("claude-")));
    }

    #[test]
    fn test_supported_models_with_api_key() {
        let store = Arc::new(InMemoryTokenStore::new());
        let auth = Arc::new(AuthManager::new(store));
        let ex = ClaudeExecutor::new(Client::new(), Some("sk-ant-test".into()), auth);
        assert!(!ex.supported_models().is_empty());
    }
}
