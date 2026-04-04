//! Claude executor — Anthropic Messages API.
//!
//! Auth: `x-api-key` for raw API keys, `Authorization: Bearer` for OAuth tokens.
//! Format: `OpenAI` -> Anthropic (translate), Anthropic -> `OpenAI` (translate).
use crate::cloak;
use crate::device_profile::DeviceProfileCache;
use crate::http_util::ProviderHttp;
use crate::registry;
use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_config::CloakConfig;
use byokey_translate::{ClaudeToOpenAI, OpenAIToClaude, inject_cache_control};
use byokey_types::{
    ChatRequest, ProviderId, RateLimitStore,
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

/// Default Anthropic API base URL.
const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
/// Messages API path (with beta flag required by the API).
const API_PATH: &str = "/v1/messages?beta=true";

// ── Shared Claude fingerprint constants ─────────────────────────────
// Re-exported via `byokey_provider::claude_headers` so the proxy crate's
// `/v1/messages` passthrough handler stays in sync.

/// Required Anthropic API version header value.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Beta features to enable.
///
/// `prompt-caching-2024-07-31` removed: prompt caching is GA since Dec 2024;
/// the beta header is now rejected by the API.
pub const ANTHROPIC_BETA: &str = "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14,context-management-2025-06-27,prompt-caching-scope-2026-01-05";

/// User-Agent matching the Claude CLI SDK version.
pub const USER_AGENT: &str = "claude-cli/2.1.63 (external, sdk-cli)";

/// Anthropic SDK package version (matches @anthropic-ai/sdk).
pub const SDK_PACKAGE_VERSION: &str = "0.74.0";

/// Node.js runtime version for X-Stainless-Runtime-Version.
pub const RUNTIME_VERSION: &str = "v22.13.1";

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
    api_url: String,
    auth: Arc<AuthManager>,
    profile_cache: Option<Arc<DeviceProfileCache>>,
    cloak_config: Option<CloakConfig>,
}

#[bon::bon]
impl ClaudeExecutor {
    /// Creates a new Claude executor.
    ///
    /// When `profile_cache` is `Some`, per-auth device fingerprints are
    /// stabilised across requests instead of using static constants.
    /// When `cloak_config` is `Some` and enabled, request cloaking is applied.
    #[builder]
    #[allow(clippy::needless_pass_by_value)]
    pub fn new(
        http: Client,
        auth: Arc<AuthManager>,
        api_key: Option<String>,
        base_url: Option<String>,
        ratelimit: Option<Arc<RateLimitStore>>,
        profile_cache: Option<Arc<DeviceProfileCache>>,
        cloak_config: Option<CloakConfig>,
    ) -> Self {
        let mut ph = ProviderHttp::new(http);
        if let Some(store) = ratelimit {
            ph = ph.with_ratelimit(store, ProviderId::Claude);
        }
        let api_url = format!(
            "{}{}",
            base_url
                .as_deref()
                .unwrap_or(DEFAULT_BASE_URL)
                .trim_end_matches('/'),
            API_PATH
        );
        Self {
            ph,
            api_key,
            api_url,
            auth,
            profile_cache,
            cloak_config,
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

        // Apply cloaking before setting stream flag (payload hash covers the pre-stream body).
        if let Some(ref cc) = self.cloak_config
            && cc.enabled
        {
            let serialized = serde_json::to_vec(&body).unwrap_or_default();
            cloak::apply_cloaking(&mut body, cc, &serialized);
        }

        body["stream"] = Value::Bool(stream);

        let auth = self.get_auth().await?;

        // Resolve fingerprint: use cached profile when available, else statics.
        let scope_key = match &auth {
            AuthMode::ApiKey(k) => format!("api_key:{k}"),
            AuthMode::Bearer(_) => "global".to_string(),
        };

        let (ua, pkg_ver, rt_ver, os, arch) = if let Some(cache) = &self.profile_cache {
            let p = cache.resolve(&scope_key);
            (
                p.user_agent,
                p.package_version,
                p.runtime_version,
                p.os,
                p.arch,
            )
        } else {
            (
                USER_AGENT.to_string(),
                SDK_PACKAGE_VERSION.to_string(),
                RUNTIME_VERSION.to_string(),
                "MacOS".to_string(),
                "arm64".to_string(),
            )
        };

        let builder = self
            .ph
            .client()
            .post(&self.api_url)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("anthropic-beta", ANTHROPIC_BETA)
            .header("anthropic-dangerous-direct-browser-access", "true")
            .header("x-app", "cli")
            .header("user-agent", &ua)
            .header("content-type", "application/json")
            // Stainless SDK fingerprint headers — mimic real Claude Code client
            .header("x-stainless-lang", "js")
            .header("x-stainless-runtime", "node")
            .header("x-stainless-runtime-version", &rt_ver)
            .header("x-stainless-package-version", &pkg_ver)
            .header("x-stainless-os", &os)
            .header("x-stainless-arch", &arch)
            .header("x-stainless-retry-count", "0")
            // Prevent compressed SSE streams from breaking line scanner
            .header("accept-encoding", "identity");

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
        registry::models_for_provider(&ProviderId::Claude)
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
pub(crate) fn translate_claude_sse(inner: ByteStream) -> ByteStream {
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
        /// Accumulated input token count from `message_start`.
        input_tokens: u64,
        /// Accumulated output token count from `message_delta`.
        output_tokens: u64,
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
            input_tokens: 0,
            output_tokens: 0,
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
                                if let Some(tokens) = ev
                                    .pointer("/message/usage/input_tokens")
                                    .and_then(Value::as_u64)
                                {
                                    s.input_tokens = tokens;
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
                                if let Some(tokens) =
                                    ev.pointer("/usage/output_tokens").and_then(Value::as_u64)
                                {
                                    s.output_tokens = tokens;
                                }
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
                                let usage_chunk = serde_json::json!({
                                    "id": &s.id,
                                    "object": "chat.completion.chunk",
                                    "model": &s.model,
                                    "choices": [],
                                    "usage": {
                                        "prompt_tokens": s.input_tokens,
                                        "completion_tokens": s.output_tokens,
                                        "total_tokens": s.input_tokens + s.output_tokens
                                    }
                                });
                                let out = format!("data: {usage_chunk}\n\ndata: [DONE]\n\n");
                                return Ok(Some((Bytes::from(out), s)));
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

    fn make_executor() -> ClaudeExecutor {
        let (client, auth) = crate::http_util::test_auth();
        ClaudeExecutor::builder().http(client).auth(auth).build()
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
        let (client, auth) = crate::http_util::test_auth();
        let ex = ClaudeExecutor::builder()
            .http(client)
            .auth(auth)
            .api_key("sk-ant-test".to_string())
            .build();
        assert!(!ex.supported_models().is_empty());
    }
}
