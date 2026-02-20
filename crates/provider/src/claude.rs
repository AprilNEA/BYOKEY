//! Claude executor — Anthropic Messages API.
//!
//! Auth: `x-api-key` for raw API keys, `Authorization: Bearer` for OAuth tokens.
//! Format: `OpenAI` -> Anthropic (translate), Anthropic -> `OpenAI` (translate).
use crate::registry;
use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_translate::{ClaudeToOpenAI, OpenAIToClaude};
use byokey_types::{
    ByokError, ProviderId,
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
    http: Client,
    api_key: Option<String>,
    auth: Arc<AuthManager>,
}

impl ClaudeExecutor {
    /// Creates a new Claude executor with an optional API key and auth manager.
    pub fn new(api_key: Option<String>, auth: Arc<AuthManager>) -> Self {
        Self {
            http: Client::new(),
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
    async fn chat_completion(&self, request: Value, stream: bool) -> Result<ProviderResponse> {
        let mut body = OpenAIToClaude.translate_request(request)?;
        body["stream"] = Value::Bool(stream);

        let auth = self.get_auth().await?;

        let builder = self
            .http
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

        let resp = builder
            .json(&body)
            .send()
            .await
            .map_err(|e| ByokError::Http(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ByokError::Http(format!("Claude API {status}: {text}")));
        }

        if stream {
            let byte_stream: ByteStream = Box::pin(
                resp.bytes_stream()
                    .map(|r| r.map_err(|e| ByokError::Http(e.to_string()))),
            );
            Ok(ProviderResponse::Stream(translate_claude_sse(byte_stream)))
        } else {
            let json: Value = resp
                .json()
                .await
                .map_err(|e| ByokError::Http(e.to_string()))?;
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
/// Claude SSE events used:
/// - `message_start`       → extract `id` and `model`; emit empty first chunk with `role`
/// - `content_block_delta` → emit content chunk for `text_delta` events
/// - `message_delta`       → emit finish chunk with mapped `finish_reason`
/// - `message_stop`        → emit `data: [DONE]`
fn translate_claude_sse(inner: ByteStream) -> ByteStream {
    struct State {
        inner: ByteStream,
        buf: Vec<u8>,
        id: String,
        model: String,
        done: bool,
    }

    Box::pin(try_unfold(
        State {
            inner,
            buf: Vec::new(),
            id: "chatcmpl-claude".to_string(),
            model: "claude".to_string(),
            done: false,
        },
        |mut s| async move {
            loop {
                if let Some(nl) = s.buf.iter().position(|&b| b == b'\n') {
                    let raw: Vec<u8> = s.buf.drain(..=nl).collect();
                    let line = String::from_utf8_lossy(&raw);
                    let line = line.trim_end_matches(['\r', '\n']);

                    if let Some(data) = line.strip_prefix("data: ") {
                        if let Ok(ev) = serde_json::from_str::<Value>(data) {
                            match ev["type"].as_str().unwrap_or("") {
                                "message_start" => {
                                    if let Some(id) =
                                        ev.pointer("/message/id").and_then(Value::as_str)
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
                                    return Ok(Some((
                                        Bytes::from(format!("data: {chunk}\n\n")),
                                        s,
                                    )));
                                }
                                "content_block_delta" => {
                                    if ev.pointer("/delta/type").and_then(Value::as_str)
                                        == Some("text_delta")
                                    {
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
                                    }
                                }
                                "message_delta" => {
                                    let finish_reason = match ev
                                        .pointer("/delta/stop_reason")
                                        .and_then(Value::as_str)
                                    {
                                        Some("max_tokens") => "length",
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
                                    return Ok(Some((
                                        Bytes::from(format!("data: {chunk}\n\n")),
                                        s,
                                    )));
                                }
                                "message_stop" => {
                                    s.done = true;
                                    return Ok(Some((Bytes::from("data: [DONE]\n\n"), s)));
                                }
                                _ => {} // ping, content_block_start, content_block_stop
                            }
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
        ClaudeExecutor::new(None, auth)
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
        let ex = ClaudeExecutor::new(Some("sk-ant-test".into()), auth);
        assert!(!ex.supported_models().is_empty());
    }
}
