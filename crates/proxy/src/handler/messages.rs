//! Anthropic Messages API passthrough handler.
//!
//! Accepts requests in native Anthropic format and forwards them to
//! either `api.anthropic.com/v1/messages` (default),
//! `api.githubcopilot.com/v1/messages` when `claude.backend: copilot`
//! is configured, or Cursor (see [`super::cursor_messages`]) for
//! `claude.backend: cursor` and `cursor/<model>` model names.
//!
//! The response (streaming SSE or complete JSON) is returned as-is.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use byokey_provider::claude::{ANTHROPIC_BETA, ANTHROPIC_VERSION, fingerprint_headers};
use byokey_provider::cloak::{derive_cc_entrypoint, inject_billing_header};
use byokey_provider::{Conversation, CopilotCredentials, CopilotIdentity, CopilotUpstream};
use byokey_types::{ByokError, ProviderId, ThinkingCapability, Usage, traits::ByteStream};
use bytes::Bytes;
use futures_util::{Future, StreamExt as _, TryStreamExt as _};
use serde_json::Value;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use crate::usage::{AnthropicUsage as _, Attribution};
use crate::util::stream::{
    deferred_stream, keep_alive, response_to_stream, tap_usage_stream, terminate_anthropic_stream,
};
use crate::util::{sse_response, strip_gateway_headers};
use crate::{AppState, error::ApiError};

/// How long a streaming request waits for the upstream's headers before the
/// client gets a response of its own, with keepalives, so that Claude Code
/// (which shows a retry banner after 20 s without a byte) keeps waiting.
/// Errors the upstream returns within this window keep their HTTP status.
const FIRST_BYTE_GRACE: Duration = Duration::from_secs(15);
/// A keepalive comment is written after this much upstream silence.
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(10);
/// This much upstream silence in a row ends the stream with an error. A
/// live upstream sends `ping` events every few seconds even while the model
/// thinks, so a longer silence means the connection is gone.
const SILENCE_LIMIT: Duration = Duration::from_secs(120);

/// Default thinking budget (tokens) for `Auto` mode on legacy Claude models
/// that require an explicit `budget_tokens` value with `thinking.type: "enabled"`.
const DEFAULT_AUTO_BUDGET: u32 = 10_000;

/// Handles `POST /v1/messages` — Anthropic native format passthrough.
///
/// Authenticates with the Claude provider (API key or OAuth), then forwards
/// the request body verbatim to the Anthropic API and streams the response
/// back without translation.
/// Claude Code's billing header carries a `cch=<hash>;` segment that changes
/// between requests. Every change invalidates the upstream prompt cache for
/// the whole system prompt, so it is pinned to one value.
const STABLE_CCH: &str = "cch=00000;";

/// Pin the `cch=` segment of a Claude Code billing header, if `text` is one.
fn stabilize_billing_header(text: &str) -> Option<String> {
    if !text.starts_with("x-anthropic-billing-header:") {
        return None;
    }
    let start = text.find("cch=")?;
    let end = start + text[start..].find(';')? + 1;
    if &text[start..end] == STABLE_CCH {
        return None;
    }
    Some(format!("{}{STABLE_CCH}{}", &text[..start], &text[end..]))
}

/// Strip empty system content to prevent "text content blocks must be non-empty" API error.
///
/// Handles both string (`"system": ""`) and array forms
/// (`"system": [{"type": "text", "text": ""}]`). Also pins the `cch=`
/// segment of Claude Code's billing header so it stops busting the prompt
/// cache.
pub(super) fn sanitize_system(body: &mut Value) {
    match body.get_mut("system") {
        Some(Value::String(s)) => {
            if let Some(fixed) = stabilize_billing_header(s) {
                *s = fixed;
            }
        }
        Some(Value::Array(arr)) => {
            for text in arr.iter_mut().filter_map(|b| b.get_mut("text")) {
                if let Some(fixed) = text.as_str().and_then(stabilize_billing_header) {
                    *text = Value::String(fixed);
                }
            }
        }
        _ => {}
    }
    let dominated_by_empty = match body.get("system") {
        Some(Value::String(s)) => s.is_empty(),
        Some(Value::Array(arr)) => arr.iter().all(|block| {
            block
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(str::is_empty)
        }),
        _ => false,
    };

    if dominated_by_empty {
        if let Some(obj) = body.as_object_mut() {
            obj.remove("system");
        }
        return;
    }

    // Filter individual empty text blocks from an array that has some non-empty blocks.
    if let Some(arr) = body.get_mut("system").and_then(Value::as_array_mut) {
        arr.retain(|block| {
            !block
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(str::is_empty)
        });
    }
}

/// Sanitize thinking configuration before sending to the Anthropic API.
///
/// Two cases require intervention:
///
/// 1. **`tool_choice` conflict** — the API rejects `thinking` when `tool_choice.type`
///    is `"any"` or `"tool"`. Strip all thinking-related fields.
///    Aligned with upstream `disableThinkingIfToolChoiceForced`.
///
/// 2. **`thinking.type: "auto"`** — not a valid Anthropic API value (returns 400).
///    Instead of stripping (which silently disables thinking), translate based on
///    model capability:
///    - Hybrid (4.6): `"auto"` → `"adaptive"` — let Claude decide thinking depth.
///    - `BudgetOnly` (legacy): `"auto"` → `"enabled"` + default budget.
///    - No thinking support: strip entirely.
fn sanitize_thinking(body: &mut Value) {
    let forced_tool = body
        .get("tool_choice")
        .and_then(|tc| tc.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|t| t == "any" || t == "tool");

    if forced_tool {
        strip_thinking_fields(body);
        return;
    }

    let is_auto = body
        .get("thinking")
        .and_then(|th| th.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|t| t == "auto");

    if is_auto {
        let model = body.get("model").and_then(Value::as_str).unwrap_or("");
        match byokey_provider::thinking_capability(model) {
            Some(ThinkingCapability::Hybrid) => {
                // 4.6 models: "auto" semantically means "let the model decide".
                body["thinking"] = serde_json::json!({"type": "adaptive"});
                if let Some(obj) = body.as_object_mut() {
                    obj.remove("output_config");
                }
            }
            Some(_) => {
                // Legacy models: "enabled" requires budget_tokens; use default.
                body["thinking"] = serde_json::json!({
                    "type": "enabled",
                    "budget_tokens": DEFAULT_AUTO_BUDGET
                });
            }
            None => {
                // Model has no thinking support — strip to avoid API error.
                strip_thinking_fields(body);
            }
        }
    }

    // Anthropic rejects temperature != 1 when thinking is active.
    normalize_temperature_for_thinking(body);
}

/// Force `temperature` to `1` when thinking is enabled/adaptive/auto.
///
/// Anthropic API returns 400 if temperature is set to anything other than 1
/// while a thinking mode is active. When thinking was stripped (e.g. by
/// `tool_choice` conflict), we leave temperature as-is so non-thinking requests
/// keep their original sampling behaviour.
fn normalize_temperature_for_thinking(body: &mut Value) {
    let thinking_active = body
        .get("thinking")
        .and_then(|th| th.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|t| matches!(t, "enabled" | "adaptive" | "auto"));

    if !thinking_active {
        return;
    }

    match body.get("temperature") {
        // temperature == 1 is already valid; no temperature field is fine too.
        None => {}
        Some(v) if v.as_f64() == Some(1.0) => {}
        Some(_) => {
            body["temperature"] = serde_json::json!(1);
        }
    }
}

/// Returns `true` if a Claude thinking block signature looks valid.
///
/// Valid Anthropic-generated signatures start with `E` or `R` (after
/// stripping an optional `<prefix>#` cache key). Thinking blocks a client
/// carried over from another vendor's model use a different format and
/// would be rejected by the Claude API if forwarded.
fn has_valid_claude_signature(sig: &str) -> bool {
    let sig = sig.trim();
    if sig.is_empty() {
        return false;
    }
    let core = if let Some(idx) = sig.find('#') {
        sig[idx + 1..].trim()
    } else {
        sig
    };
    if core.is_empty() {
        return false;
    }
    matches!(core.as_bytes()[0], b'E' | b'R')
}

/// Strip thinking blocks with non-Anthropic signatures from
/// `messages[].content[]` so they don't trip the Claude API on the way out.
fn strip_invalid_thinking_signatures(body: &mut Value) {
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    for msg in messages {
        let Some(content) = msg.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        content.retain(|block| {
            if block.get("type").and_then(Value::as_str) != Some("thinking") {
                return true;
            }
            let sig = block.get("signature").and_then(Value::as_str).unwrap_or("");
            has_valid_claude_signature(sig)
        });
    }
}

/// Remove thinking-related fields and associated adaptive controls.
fn strip_thinking_fields(body: &mut Value) {
    if let Some(obj) = body.as_object_mut() {
        obj.remove("thinking");
        if let Some(oc) = obj.get_mut("output_config").and_then(Value::as_object_mut) {
            oc.remove("effort");
            if oc.is_empty() {
                obj.remove("output_config");
            }
        }
    }
}

/// Beta that unlocks the 1M-token context window on Anthropic's API.
pub(super) const CONTEXT_1M_BETA: &str = "context-1m-2025-08-07";

/// Claude Code and Claude Desktop pick a model's 1M-context variant by
/// appending `[1m]` to its id, and Claude Desktop sends that spelling to a
/// gateway as-is. Upstreams reject it, so it is taken off `body.model`.
/// Returns whether it was there, so the caller can ask for the long context
/// in the upstream's own terms. Runs before anything that looks the model up.
pub(super) fn take_long_context_suffix(body: &mut Value) -> bool {
    let Some(bare) = body
        .get("model")
        .and_then(Value::as_str)
        .and_then(|m| m.strip_suffix("[1m]"))
    else {
        return false;
    };
    body["model"] = Value::String(bare.to_owned());
    true
}

/// Merge betas from the request body's `betas` array, the client's
/// `anthropic-beta` HTTP header and `extra` into the base beta string, then
/// strip the body field so the upstream API doesn't reject it as unknown.
pub(super) fn build_beta_header(
    body: &mut Value,
    client_headers: &HeaderMap,
    extra: Option<&str>,
) -> String {
    let mut betas = ANTHROPIC_BETA.to_string();
    if let Some(extra) = extra
        && !betas.contains(extra)
    {
        betas.push(',');
        betas.push_str(extra);
    }

    // Merge from client's `anthropic-beta` HTTP header (comma-separated).
    if let Some(hv) = client_headers
        .get("anthropic-beta")
        .and_then(|v| v.to_str().ok())
    {
        for token in hv.split(',') {
            let token = token.trim();
            if !token.is_empty() && !betas.contains(token) {
                betas.push(',');
                betas.push_str(token);
            }
        }
    }

    // Merge from body's `betas` array (BYOKEY client-to-proxy convention).
    if let Some(arr) = body.get("betas").and_then(Value::as_array) {
        for b in arr {
            if let Some(s) = b.as_str()
                && !betas.contains(s)
            {
                betas.push(',');
                betas.push_str(s);
            }
        }
    }
    // Strip `betas` — it's a client-to-proxy field, not a valid API field.
    if let Some(obj) = body.as_object_mut() {
        obj.remove("betas");
    }
    betas
}

#[tracing::instrument(skip_all, fields(
    model = %body.0.get("model").and_then(serde_json::Value::as_str).unwrap_or("-"),
    stream = body.0.get("stream").and_then(serde_json::Value::as_bool).unwrap_or(false),
))]
pub async fn anthropic_messages(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::extract::Json<Value>,
) -> Result<Response, ApiError> {
    serve_messages(state, headers, body.0)
        .await
        .map_err(ApiError::anthropic)
}

#[allow(clippy::too_many_lines)] // Single-pass handler — keeping one function boundary is clearer than splitting.
async fn serve_messages(
    state: Arc<AppState>,
    headers: HeaderMap,
    mut body: Value,
) -> Result<Response, ApiError> {
    let long_context = take_long_context_suffix(&mut body);
    sanitize_system(&mut body);
    sanitize_thinking(&mut body);
    strip_invalid_thinking_signatures(&mut body);
    let stream = body.get("stream").and_then(Value::as_bool).unwrap_or(false);
    let beta = build_beta_header(&mut body, &headers, long_context.then_some(CONTEXT_1M_BETA));

    let config = state.config.load();
    match Backend::route(&config, &mut body) {
        Backend::Cursor => {
            return super::cursor_messages::cursor_messages(&state, body, stream).await;
        }
        Backend::Copilot => return copilot_messages(&state, body, stream, &beta).await,
        Backend::Anthropic => {}
    }

    // Default: passthrough to Anthropic API.
    let provider_cfg = config.providers.get(&ProviderId::Claude);
    let api_key = provider_cfg.and_then(|pc| pc.api_key.clone());
    let is_oauth = api_key.is_none();

    // Resolve stable device fingerprint from the profile cache.
    let profile = state.device_profiles.resolve("global");

    // OAuth tokens require the billing header and tool name remapping.
    if is_oauth {
        let account_uuid = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, b"global").to_string();
        let ua = headers
            .get(axum::http::header::USER_AGENT)
            .and_then(|v| v.to_str().ok());
        let entrypoint = derive_cc_entrypoint(ua);
        let workload = headers
            .get("x-byokey-claude-workload")
            .and_then(|v| v.to_str().ok());
        inject_billing_header(
            &mut body,
            &profile.device_id,
            &account_uuid,
            &profile.session_id,
            entrypoint,
            workload,
        );
        byokey_provider::cloak::remap_tool_names_request(&mut body);
    }

    let upstream = AnthropicUpstream::resolve(&state, &config, &profile, &beta).await?;

    let accept = if stream {
        "text/event-stream"
    } else {
        "application/json"
    };

    let builder = upstream
        .request(&state.http, "/v1/messages")
        .header("accept", accept)
        .header("connection", "keep-alive")
        .header("accept-encoding", "identity");

    // Log request details for debugging upstream errors.
    let model = body.get("model").and_then(Value::as_str).unwrap_or("?");
    let keys: Vec<&str> = body
        .as_object()
        .map(|o| o.keys().map(String::as_str).collect())
        .unwrap_or_default();
    tracing::info!(
        %model, ?keys, auth = if is_oauth { "oauth" } else { "api_key" },
        beta = %beta, "anthropic passthrough"
    );

    let model_name = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();

    let attribution = Attribution::new(
        state.usage.clone(),
        model_name,
        ProviderId::Claude,
        upstream.account_id,
    );
    forward(builder.json(&body).send(), stream, attribution, is_oauth).await
}

/// Default Anthropic API base URL.
const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";

/// How a request authenticates with Anthropic.
enum Credential {
    /// A raw API key, sent as `x-api-key`.
    ApiKey(String),
    /// An OAuth access token, sent as a bearer token.
    OAuth(String),
}

/// Credentials and headers for the Anthropic API.
pub(super) struct AnthropicUpstream {
    base_url: String,
    credential: Credential,
    headers: http::HeaderMap,
    /// Account the request is attributed to in usage records.
    pub(super) account_id: String,
}

impl AnthropicUpstream {
    /// A configured API key, else the active Claude OAuth account.
    pub(super) async fn resolve(
        state: &AppState,
        config: &byokey_config::Config,
        profile: &byokey_provider::device_profile::DeviceProfile,
        beta: &str,
    ) -> Result<Self, ApiError> {
        let provider_cfg = config.providers.get(&ProviderId::Claude);
        let (credential, account_id) =
            if let Some(key) = provider_cfg.and_then(|pc| pc.api_key.clone()) {
                (
                    Credential::ApiKey(key),
                    byokey_types::DEFAULT_ACCOUNT.to_string(),
                )
            } else {
                let (account_id, token) = state
                    .auth
                    .get_token_with_account(ProviderId::Claude)
                    .await?;
                (Credential::OAuth(token.access_token), account_id)
            };
        let mut headers = fingerprint_headers(profile, matches!(credential, Credential::ApiKey(_)));
        headers.insert(
            "anthropic-version",
            http::HeaderValue::from_static(ANTHROPIC_VERSION),
        );
        headers.insert(
            "anthropic-beta",
            http::HeaderValue::from_str(beta)
                .map_err(|e| ApiError::from(ByokError::Config(e.to_string())))?,
        );
        Ok(Self {
            base_url: provider_cfg
                .and_then(|pc| pc.base_url.as_deref())
                .unwrap_or(ANTHROPIC_BASE_URL)
                .trim_end_matches('/')
                .to_owned(),
            credential,
            headers,
            account_id,
        })
    }

    /// A POST to `path` on the Anthropic API carrying the auth, version, beta
    /// and fingerprint headers.
    pub(super) fn request(&self, http: &reqwest::Client, path: &str) -> reqwest::RequestBuilder {
        let mut builder = http
            .post(format!("{}{path}?beta=true", self.base_url))
            .header("content-type", "application/json");
        builder = match &self.credential {
            Credential::ApiKey(key) => builder.header("x-api-key", key),
            Credential::OAuth(token) => builder.bearer_auth(token),
        };
        for (name, value) in &self.headers {
            builder = builder.header(name.as_str(), value.as_bytes());
        }
        builder
    }
}

/// Which upstream serves a Messages request.
pub(super) enum Backend {
    Anthropic,
    Copilot,
    Cursor,
}

impl Backend {
    /// An explicit `copilot/` or `cursor/` model prefix wins over any global
    /// backend; otherwise `claude.backend` picks Copilot or Cursor for every
    /// request. The prefix is stripped from `body.model`.
    pub(super) fn route(config: &byokey_config::Config, body: &mut Value) -> Self {
        let model = body
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let (hint, bare) = byokey_provider::parse_qualified_model(model);
        let explicit = hint.filter(|p| matches!(p, ProviderId::Copilot | ProviderId::Cursor));
        if explicit.is_some() {
            body["model"] = Value::String(bare.to_owned());
        }
        let backend = explicit.or_else(|| {
            config
                .providers
                .get(&ProviderId::Claude)
                .and_then(|c| c.backend)
        });
        match backend {
            Some(ProviderId::Cursor) => Self::Cursor,
            Some(ProviderId::Copilot) => Self::Copilot,
            _ => Self::Anthropic,
        }
    }
}

/// The Copilot accounts configured for this server.
pub(super) fn copilot_upstream(state: &AppState) -> CopilotUpstream {
    let config = state
        .config
        .load()
        .providers
        .get(&ProviderId::Copilot)
        .cloned()
        .unwrap_or_default();
    CopilotUpstream::builder()
        .http(state.http.clone())
        .auth(state.auth.clone())
        .maybe_api_key(config.api_key)
        .maybe_base_url(config.base_url)
        .identity(state.copilot_identity.clone())
        .build()
}

/// A POST of `body` to Copilot's Anthropic-format `path` as `creds`' account.
pub(super) fn copilot_request(
    http: &reqwest::Client,
    path: &str,
    creds: &CopilotCredentials,
    beta: &str,
    identity: &CopilotIdentity,
    conversation: &Conversation,
    body: &Value,
) -> reqwest::RequestBuilder {
    let mut builder = http
        .post(format!("{}{path}", creds.endpoint))
        .header("authorization", format!("Bearer {}", creds.token))
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("anthropic-beta", beta)
        .header("content-type", "application/json");
    for (name, value) in identity.request_headers(creds, conversation) {
        builder = builder.header(name, value);
    }
    builder.json(body)
}

/// Anthropic server tools Copilot's `/v1/messages` rejects with 400 ("The
/// use of the web search tool is not supported", "rejected tool(s):
/// `web_fetch`"). Its other built-in tools (`bash`, `text_editor`,
/// `code_execution`) are accepted.
const COPILOT_REJECTED_TOOLS: &[&str] = &["web_search", "web_fetch"];

/// Drop what Copilot's `/v1/messages` rejects: the fields it answers with
/// "Extra inputs are not permitted" (the per-message `output_config` of the
/// `per-turn-control` beta, the top-level `safeguards`, and the `scope` of
/// `cache_control` markers), and the server tools in
/// [`COPILOT_REJECTED_TOOLS`]. Claude Code sends all of these by default;
/// without the tools its `WebSearch` and `WebFetch` are unavailable, with
/// them the whole turn would fail.
pub(super) fn strip_copilot_unsupported(body: &mut Value) {
    strip_cache_scope(body);
    let Some(body) = body.as_object_mut() else {
        return;
    };
    body.remove("safeguards");
    if let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
        for message in messages.iter_mut().filter_map(Value::as_object_mut) {
            message.remove("output_config");
        }
    }
    if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
        tools.retain(|tool| {
            let server_tool = tool.get("type").and_then(Value::as_str).is_some_and(|t| {
                COPILOT_REJECTED_TOOLS.iter().any(|name| {
                    t.strip_prefix(name)
                        .is_some_and(|rest| rest.starts_with('_'))
                })
            });
            if server_tool {
                tracing::debug!(tool = %tool["type"], "dropping a server tool Copilot rejects");
            }
            !server_tool
        });
        if tools.is_empty() {
            body.remove("tools");
        }
    }
}

/// Remove `scope` from every `cache_control` marker in `value`.
fn strip_cache_scope(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(cc) = map.get_mut("cache_control").and_then(Value::as_object_mut) {
                cc.remove("scope");
            }
            map.values_mut().for_each(strip_cache_scope);
        }
        Value::Array(items) => items.iter_mut().for_each(strip_cache_scope),
        _ => {}
    }
}

/// Claude Code's compaction requests: they carry no tools yet must run on
/// the model the user chose, since their output replaces the conversation.
const COMPACTION_PROMPTS: &[&str] = &[
    "You are a helpful AI assistant tasked with summarizing conversations",
    "Your task is to create a detailed summary of the conversation so far",
];

/// Whether a request is one of the incidental calls Claude Code makes
/// around a turn (a title, a suggestion, a summary): no tools, and not a
/// compaction. On a per-request Copilot plan each one costs as much as a
/// real turn, so `providers.copilot.small_model` may serve them instead.
pub(super) fn is_incidental(body: &Value) -> bool {
    let has_tools = body
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(|t| !t.is_empty());
    if has_tools {
        return false;
    }
    let mut texts = Vec::new();
    match body.get("system") {
        Some(Value::String(s)) => texts.push(s.as_str()),
        Some(Value::Array(blocks)) => {
            texts.extend(blocks.iter().filter_map(|b| b["text"].as_str()));
        }
        _ => {}
    }
    if let Some(last) = body
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|m| m.last())
    {
        match last.get("content") {
            Some(Value::String(s)) => texts.push(s.as_str()),
            Some(Value::Array(blocks)) => {
                texts.extend(blocks.iter().filter_map(|b| b["text"].as_str()));
            }
            _ => {}
        }
    }
    !texts.iter().any(|t| {
        let t = t.trim_start();
        COMPACTION_PROMPTS.iter().any(|p| t.starts_with(p))
    })
}

/// Route Anthropic-format request to Copilot's native `/v1/messages` endpoint.
///
/// Copilot provides a native Anthropic-compatible Messages API at
/// `api.githubcopilot.com/v1/messages`. This handler authenticates as the
/// account's Copilot client and forwards the request verbatim.
///
/// A Copilot API token that is rejected before its stated expiry is
/// exchanged again once. With multiple Copilot accounts, transient failures
/// are retried with quota-aware rotation.
#[allow(clippy::too_many_lines)]
#[tracing::instrument(skip_all, fields(
    model = %body.get("model").and_then(serde_json::Value::as_str).unwrap_or("-"),
    stream,
    attempt = tracing::field::Empty,
))]
async fn copilot_messages(
    state: &Arc<AppState>,
    mut body: Value,
    stream: bool,
    beta: &str,
) -> Result<Response, ApiError> {
    strip_copilot_unsupported(&mut body);
    let copilot = copilot_upstream(state);
    let small_model = state
        .config
        .load()
        .providers
        .get(&ProviderId::Copilot)
        .and_then(|c| c.small_model.clone());
    if let Some(small) = small_model
        && is_incidental(&body)
    {
        tracing::info!(small_model = %small, "serving a tool-less request with the small model");
        body["model"] = Value::String(small);
    }

    let accounts = state
        .auth
        .list_accounts(ProviderId::Copilot)
        .await
        .unwrap_or_default();
    let max_attempts = if accounts.len() > 1 {
        accounts.len().min(3)
    } else {
        1
    };

    let accept = if stream {
        "text/event-stream"
    } else {
        "application/json"
    };
    let messages = body
        .get("messages")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let conversation = Conversation::from_messages(messages);
    let model_name = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();

    let mut last_err = None;
    let mut attempt = 0;
    let mut token_refreshed = false;
    while attempt < max_attempts {
        tracing::Span::current().record("attempt", attempt);
        let creds = match copilot.credentials().await {
            Ok(c) => c,
            Err(e) => {
                if max_attempts > 1 {
                    tracing::warn!(attempt, error = %e, "copilot token failed, trying next account");
                    CopilotUpstream::invalidate_current_account();
                    last_err = Some(ApiError::from(e));
                    attempt += 1;
                    continue;
                }
                return Err(ApiError::from(e));
            }
        };
        tracing::info!(
            endpoint = %creds.endpoint,
            model = %body.get("model").and_then(|v| v.as_str()).unwrap_or("unknown"),
            stream, ?conversation, attempt,
            "routing Anthropic messages through Copilot"
        );

        let pending = copilot_request(
            &state.http,
            "/v1/messages",
            &creds,
            beta,
            copilot.identity(),
            &conversation,
            &body,
        )
        .header("accept", accept)
        .send();

        // Copilot does its own account rotation inside CopilotUpstream; the
        // specific account isn't exposed here, so usage goes to DEFAULT_ACCOUNT.
        let attribution = Attribution::new(
            state.usage.clone(),
            model_name.clone(),
            ProviderId::Copilot,
            byokey_types::DEFAULT_ACCOUNT,
        );
        // Only the last attempt may hand the client a response before the
        // upstream answered: an earlier one still needs the status to decide
        // whether to try the next account.
        let last_attempt = attempt + 1 >= max_attempts;
        let outcome = if last_attempt {
            forward(pending, stream, attribution, false).await
        } else {
            match pending.await {
                Ok(resp) => forward_response(resp, stream, attribution, false).await,
                Err(e) => Err(ApiError::from(ByokError::from(e))),
            }
        };
        let err = match outcome {
            Ok(response) => return Ok(response),
            Err(err) => err,
        };
        // The cached token may have been revoked ahead of its expiry.
        if matches!(err.error, ByokError::Upstream { status: 401, .. })
            && !token_refreshed
            && CopilotUpstream::forget_token(&creds)
        {
            token_refreshed = true;
            tracing::warn!(attempt, "copilot rejected its token, exchanging a new one");
            continue;
        }
        if !err.error.is_retryable() || last_attempt {
            return Err(err);
        }
        tracing::warn!(attempt, error = %err.error, "copilot messages failed, trying next account");
        CopilotUpstream::invalidate_current_account();
        last_err = Some(err);
        attempt += 1;
    }

    tracing::error!(
        attempts = max_attempts,
        "all copilot accounts exhausted for messages request"
    );
    Attribution::new(
        state.usage.clone(),
        model_name,
        ProviderId::Copilot,
        byokey_types::DEFAULT_ACCOUNT,
    )
    .failure();
    Err(last_err
        .unwrap_or_else(|| ApiError::from(ByokError::Auth("no copilot accounts available".into()))))
}

/// Forward the response to `pending` back to the client.
///
/// A streaming client is answered as soon as [`FIRST_BYTE_GRACE`] passes
/// without upstream headers: it gets a `200` and keepalive comments until
/// the upstream's body arrives, or its error as an in-stream `error` event.
/// Upstream errors that arrive within the grace period, and every
/// non-streaming response, keep their HTTP status.
async fn forward(
    pending: impl Future<Output = reqwest::Result<reqwest::Response>> + Send + 'static,
    stream: bool,
    attribution: Attribution,
    reverse_remap_tools: bool,
) -> Result<Response, ApiError> {
    if !stream {
        let resp = pending
            .await
            .map_err(|e| ApiError::from(ByokError::from(e)))?;
        return forward_response(resp, false, attribution, reverse_remap_tools).await;
    }
    let mut pending = Box::pin(pending);
    match tokio::time::timeout(FIRST_BYTE_GRACE, &mut pending).await {
        Ok(Ok(resp)) => forward_response(resp, true, attribution, reverse_remap_tools).await,
        Ok(Err(e)) => Err(ApiError::from(ByokError::from(e))),
        Err(_elapsed) => {
            tracing::info!(
                grace_secs = FIRST_BYTE_GRACE.as_secs(),
                "upstream headers are late; streaming keepalives to the client"
            );
            Ok(stream_response(
                StatusCode::OK,
                &HeaderMap::new(),
                deferred_stream(pending),
                attribution,
                reverse_remap_tools,
            ))
        }
    }
}

/// Forward an upstream response back to the client, recording token usage.
async fn forward_response(
    resp: reqwest::Response,
    stream: bool,
    attribution: Attribution,
    reverse_remap_tools: bool,
) -> Result<Response, ApiError> {
    let status = resp.status();
    if !status.is_success() {
        let err = ByokError::from_response(resp).await;
        tracing::error!(status = status.as_u16(), "upstream error");
        attribution.failure();
        return Err(ApiError::from(err));
    }

    let upstream_status = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::OK);

    // Collect upstream response headers and strip gateway fingerprints before
    // forwarding anything to the client.
    let mut upstream_headers = axum::http::HeaderMap::new();
    for (name, value) in resp.headers() {
        if let Ok(name) = axum::http::HeaderName::from_bytes(name.as_str().as_bytes())
            && let Ok(value) = axum::http::HeaderValue::from_bytes(value.as_bytes())
        {
            upstream_headers.insert(name, value);
        }
    }
    strip_gateway_headers(&mut upstream_headers);
    // Both branches re-encode the body, so the upstream framing no longer
    // describes it. A stale content-length makes hyper panic mid-response.
    for framing in [
        axum::http::header::CONTENT_LENGTH,
        axum::http::header::TRANSFER_ENCODING,
        axum::http::header::CONTENT_ENCODING,
    ] {
        upstream_headers.remove(framing);
    }

    if stream {
        return Ok(stream_response(
            upstream_status,
            &upstream_headers,
            response_to_stream(resp),
            attribution,
            reverse_remap_tools,
        ));
    }
    let mut json: Value = resp
        .json()
        .await
        .map_err(|e| ApiError::from(ByokError::from(e)))?;
    if reverse_remap_tools {
        byokey_provider::cloak::reverse_remap_tool_names_response(&mut json);
    }
    attribution.success(Usage::from_response(&json));
    let mut response = (upstream_status, axum::Json(json)).into_response();
    // Merge upstream headers (gateway-stripped) into the JSON response.
    for (name, value) in &upstream_headers {
        response
            .headers_mut()
            .entry(name)
            .or_insert_with(|| value.clone());
    }
    Ok(response)
}

/// The SSE response for an upstream byte stream: tool names mapped back for
/// OAuth, usage recorded, keepalives while the upstream is silent, and a
/// guaranteed terminal event.
fn stream_response(
    status: StatusCode,
    upstream_headers: &HeaderMap,
    raw: ByteStream,
    attribution: Attribution,
    reverse_remap_tools: bool,
) -> Response {
    let remapped: ByteStream = if reverse_remap_tools {
        Box::pin(raw.map(move |chunk| {
            let bytes = chunk?;
            let text = String::from_utf8_lossy(&bytes);
            let mut output = String::new();
            for line in text.split_inclusive('\n') {
                if let Some(data) = line.trim().strip_prefix("data: ")
                    && let Ok(mut ev) = serde_json::from_str::<Value>(data)
                {
                    byokey_provider::cloak::reverse_remap_tool_name_sse(&mut ev);
                    let _ = writeln!(output, "data: {ev}");
                    continue;
                }
                output.push_str(line);
            }
            Ok(Bytes::from(output))
        }))
    } else {
        raw
    };
    let tapped = tap_usage_stream(remapped, attribution);
    let alive = keep_alive(tapped, KEEPALIVE_INTERVAL, SILENCE_LIMIT);
    let mapped =
        terminate_anthropic_stream(alive).map_err(|e| std::io::Error::other(e.to_string()));
    let mut sse = sse_response(status, mapped);
    // Merge upstream headers (gateway-stripped) into the SSE response,
    // without overwriting the SSE-specific ones sse_response set.
    for (name, value) in upstream_headers {
        sse.headers_mut()
            .entry(name)
            .or_insert_with(|| value.clone());
    }
    sse
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn explicit_prefix_beats_global_backend_and_is_stripped() {
        let mut config = byokey_config::Config::default();
        config.providers.insert(
            ProviderId::Claude,
            byokey_config::ProviderConfig {
                backend: Some(ProviderId::Copilot),
                ..Default::default()
            },
        );
        let route = |model: &str| {
            let mut body = json!({"model": model});
            let backend = Backend::route(&config, &mut body);
            (backend, body["model"].as_str().unwrap().to_owned())
        };
        assert!(matches!(route("cursor/opus"), (Backend::Cursor, m) if m == "opus"));
        assert!(
            matches!(route("copilot/claude-opus-5.5"), (Backend::Copilot, m) if m == "claude-opus-5.5")
        );
        assert!(
            matches!(route("claude-opus-5-5"), (Backend::Copilot, m) if m == "claude-opus-5-5")
        );
        assert!(matches!(route("codex/gpt-5.4"), (Backend::Copilot, m) if m == "codex/gpt-5.4"));
    }

    #[test]
    fn long_context_suffix_comes_off_the_model_and_becomes_a_beta() {
        let mut body = json!({"model": "claude-sonnet-5[1m]", "betas": ["x-beta"]});
        let long_context = take_long_context_suffix(&mut body);
        assert!(long_context);
        assert_eq!(body["model"], "claude-sonnet-5");
        let beta = build_beta_header(
            &mut body,
            &HeaderMap::new(),
            long_context.then_some(CONTEXT_1M_BETA),
        );
        let betas: Vec<&str> = beta.split(',').collect();
        assert!(betas.contains(&CONTEXT_1M_BETA));
        assert!(betas.contains(&"x-beta"));
        assert!(body.get("betas").is_none());

        let mut body = json!({"model": "claude-sonnet-5"});
        assert!(!take_long_context_suffix(&mut body));
        assert_eq!(body["model"], "claude-sonnet-5");
        let beta = build_beta_header(&mut body, &HeaderMap::new(), None);
        assert!(!beta.contains(CONTEXT_1M_BETA));

        let mut body = json!({"max_tokens": 1});
        assert!(!take_long_context_suffix(&mut body), "no model at all");
    }

    #[test]
    fn a_long_context_model_still_gets_its_thinking_and_backend_resolved() {
        let mut body = json!({"model": "claude-opus-5-5[1m]", "thinking": {"type": "auto"}});
        take_long_context_suffix(&mut body);
        sanitize_thinking(&mut body);
        assert_eq!(body["thinking"]["type"], "adaptive");

        let mut body = json!({"model": "copilot/claude-opus-5.5[1m]"});
        assert!(take_long_context_suffix(&mut body));
        let backend = Backend::route(&byokey_config::Config::default(), &mut body);
        assert!(matches!(backend, Backend::Copilot));
        assert_eq!(body["model"], "claude-opus-5.5");
    }

    #[test]
    fn copilot_request_drops_fields_copilot_rejects_and_keeps_the_rest() {
        let mut body = json!({
            "model": "claude-fable-5-1",
            "safeguards": {"mode": "default"},
            "output_config": {"effort": "high"},
            "context_management": {"edits": []},
            "system": [{"type": "text", "text": "s", "cache_control": {"type": "ephemeral", "scope": "global"}}],
            "tools": [
                {"name": "t", "input_schema": {}, "eager_input_streaming": true},
                {"type": "web_search_20250305", "name": "web_search", "max_uses": 3},
                {"type": "web_fetch_20250910", "name": "web_fetch"},
                {"type": "text_editor_20250728", "name": "str_replace_based_edit_tool"}
            ],
            "messages": [
                {"role": "user", "content": "hi", "output_config": {"effort": "low"}},
                {"role": "assistant", "content": "hello"},
                {"role": "user", "content": [
                    {"type": "text", "text": "x", "cache_control": {"type": "ephemeral", "ttl": "1h", "scope": "global"}}
                ]}
            ]
        });
        strip_copilot_unsupported(&mut body);
        assert_eq!(
            body,
            json!({
                "model": "claude-fable-5-1",
                "output_config": {"effort": "high"},
                "context_management": {"edits": []},
                "system": [{"type": "text", "text": "s", "cache_control": {"type": "ephemeral"}}],
                "tools": [
                    {"name": "t", "input_schema": {}, "eager_input_streaming": true},
                    {"type": "text_editor_20250728", "name": "str_replace_based_edit_tool"}
                ],
                "messages": [
                    {"role": "user", "content": "hi"},
                    {"role": "assistant", "content": "hello"},
                    {"role": "user", "content": [
                        {"type": "text", "text": "x", "cache_control": {"type": "ephemeral", "ttl": "1h"}}
                    ]}
                ]
            })
        );
    }

    #[test]
    fn a_request_left_with_only_rejected_tools_has_no_tools_field() {
        let mut body = json!({
            "model": "claude-sonnet-5",
            "tools": [{"type": "web_search_20250305", "name": "web_search"}],
            "messages": [{"role": "user", "content": "hi"}]
        });
        strip_copilot_unsupported(&mut body);
        assert!(body.get("tools").is_none());
        // A custom tool that merely mentions the name is kept.
        let mut body = json!({"tools": [{"name": "web_search_notes", "input_schema": {}}]});
        strip_copilot_unsupported(&mut body);
        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn the_billing_header_cch_segment_is_pinned() {
        let header =
            "x-anthropic-billing-header: cc_version=2.1.282.7f3a; cc_entrypoint=cli; cch=a1b2c;";
        let mut body = json!({
            "system": [
                {"type": "text", "text": header, "cache_control": {"type": "ephemeral"}},
                {"type": "text", "text": "You are Claude Code."}
            ]
        });
        sanitize_system(&mut body);
        assert_eq!(
            body["system"][0]["text"],
            "x-anthropic-billing-header: cc_version=2.1.282.7f3a; cc_entrypoint=cli; cch=00000;"
        );
        assert_eq!(body["system"][1]["text"], "You are Claude Code.");

        let mut body = json!({"system": header});
        sanitize_system(&mut body);
        assert!(body["system"].as_str().unwrap().ends_with("cch=00000;"));

        // Not a billing header, or no cch: untouched.
        assert!(stabilize_billing_header("cch=zzz; something").is_none());
        assert!(stabilize_billing_header("x-anthropic-billing-header: cc_version=1;").is_none());
        assert!(stabilize_billing_header("x-anthropic-billing-header: cch=00000;").is_none());
    }

    #[test]
    fn incidental_requests_have_no_tools_and_are_not_compactions() {
        assert!(is_incidental(&json!({
            "system": "Generate a short title.",
            "messages": [{"role": "user", "content": "hi"}]
        })));
        assert!(is_incidental(&json!({
            "tools": [],
            "messages": [{"role": "user", "content": "hi"}]
        })));
        assert!(!is_incidental(&json!({
            "tools": [{"name": "Bash"}],
            "messages": [{"role": "user", "content": "hi"}]
        })));
        assert!(!is_incidental(&json!({
            "system": [{"type": "text", "text": "You are a helpful AI assistant tasked with summarizing conversations."}],
            "messages": [{"role": "user", "content": "go"}]
        })));
        assert!(!is_incidental(&json!({
            "messages": [{"role": "user", "content": [
                {"type": "text", "text": "Your task is to create a detailed summary of the conversation so far."}
            ]}]
        })));
    }

    // ── sanitize_thinking: tool_choice conflict ────────────────────────

    #[test]
    fn tool_choice_any_strips_thinking() {
        let mut body = json!({
            "model": "claude-opus-5-5",
            "thinking": {"type": "enabled", "budget_tokens": 10000},
            "tool_choice": {"type": "any"},
            "output_config": {"effort": "high"}
        });
        sanitize_thinking(&mut body);
        assert!(body.get("thinking").is_none());
        assert!(body.get("output_config").is_none());
    }

    #[test]
    fn tool_choice_tool_strips_thinking() {
        let mut body = json!({
            "model": "claude-opus-5-5",
            "thinking": {"type": "adaptive"},
            "tool_choice": {"type": "tool", "name": "get_weather"}
        });
        sanitize_thinking(&mut body);
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn tool_choice_auto_does_not_strip() {
        let mut body = json!({
            "model": "claude-opus-5-5",
            "thinking": {"type": "adaptive"},
            "tool_choice": {"type": "auto"}
        });
        sanitize_thinking(&mut body);
        assert_eq!(body["thinking"]["type"], "adaptive");
    }

    // ── sanitize_thinking: "auto" translation ──────────────────────────

    #[test]
    fn auto_on_hybrid_model_becomes_adaptive() {
        // claude-opus-5-5 is Hybrid → should translate to "adaptive".
        let mut body = json!({
            "model": "claude-opus-5-5",
            "thinking": {"type": "auto"},
            "output_config": {"effort": "high"}
        });
        sanitize_thinking(&mut body);
        assert_eq!(body["thinking"]["type"], "adaptive");
        // output_config should be removed — adaptive picks its own effort.
        assert!(body.get("output_config").is_none());
    }

    #[test]
    fn auto_on_unknown_model_strips_thinking() {
        // Unknown model has no thinking support → strip entirely.
        let mut body = json!({
            "model": "gpt-4o",
            "thinking": {"type": "auto"}
        });
        sanitize_thinking(&mut body);
        assert!(body.get("thinking").is_none());
    }

    // ── sanitize_thinking: valid types pass through ────────────────────

    #[test]
    fn enabled_type_passes_through() {
        let mut body = json!({
            "model": "claude-opus-5-5",
            "thinking": {"type": "enabled", "budget_tokens": 8000}
        });
        sanitize_thinking(&mut body);
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 8000);
    }

    #[test]
    fn adaptive_type_passes_through() {
        let mut body = json!({
            "model": "claude-opus-5-5",
            "thinking": {"type": "adaptive"}
        });
        sanitize_thinking(&mut body);
        assert_eq!(body["thinking"]["type"], "adaptive");
    }

    #[test]
    fn no_thinking_field_is_noop() {
        let mut body = json!({"model": "claude-opus-5-5", "max_tokens": 1024});
        let expected = body.clone();
        sanitize_thinking(&mut body);
        assert_eq!(body, expected);
    }

    // ── strip_thinking_fields ──────────────────────────────────────────

    #[test]
    fn strip_cleans_output_config_effort() {
        let mut body = json!({
            "thinking": {"type": "enabled"},
            "output_config": {"effort": "high", "format": "json"}
        });
        strip_thinking_fields(&mut body);
        assert!(body.get("thinking").is_none());
        // "format" remains, only "effort" removed.
        assert!(body["output_config"].get("effort").is_none());
        assert_eq!(body["output_config"]["format"], "json");
    }

    #[test]
    fn strip_removes_empty_output_config() {
        let mut body = json!({
            "thinking": {"type": "enabled"},
            "output_config": {"effort": "high"}
        });
        strip_thinking_fields(&mut body);
        assert!(body.get("output_config").is_none());
    }

    // ── normalize_temperature_for_thinking ─────────────────────────────

    #[test]
    fn adaptive_thinking_coerces_temperature_to_one() {
        let mut body = json!({
            "model": "claude-opus-5-5",
            "temperature": 0,
            "thinking": {"type": "adaptive"}
        });
        sanitize_thinking(&mut body);
        assert_eq!(body["temperature"], 1);
    }

    #[test]
    fn enabled_thinking_coerces_temperature_to_one() {
        let mut body = json!({
            "model": "claude-opus-5-5",
            "temperature": 0.2,
            "thinking": {"type": "enabled", "budget_tokens": 2048}
        });
        sanitize_thinking(&mut body);
        assert_eq!(body["temperature"], 1);
    }

    #[test]
    fn temperature_one_with_thinking_is_unchanged() {
        let mut body = json!({
            "model": "claude-opus-5-5",
            "temperature": 1,
            "thinking": {"type": "adaptive"}
        });
        sanitize_thinking(&mut body);
        assert_eq!(body["temperature"], 1);
    }

    #[test]
    fn no_thinking_leaves_temperature_alone() {
        let mut body = json!({
            "model": "claude-opus-5-5",
            "temperature": 0,
            "messages": [{"role": "user", "content": "hi"}]
        });
        sanitize_thinking(&mut body);
        assert_eq!(body["temperature"], 0);
    }

    #[test]
    fn forced_tool_choice_strips_thinking_keeps_temperature() {
        let mut body = json!({
            "model": "claude-opus-5-5",
            "temperature": 0,
            "thinking": {"type": "adaptive"},
            "tool_choice": {"type": "any"}
        });
        sanitize_thinking(&mut body);
        assert!(body.get("thinking").is_none());
        // Temperature should remain at 0 — thinking was stripped.
        assert_eq!(body["temperature"], 0);
    }

    #[test]
    fn no_temperature_with_thinking_is_fine() {
        let mut body = json!({
            "model": "claude-opus-5-5",
            "thinking": {"type": "adaptive"}
        });
        sanitize_thinking(&mut body);
        assert!(body.get("temperature").is_none());
    }

    // ── forward_response: re-encoded bodies ────────────────────────────

    #[tokio::test]
    async fn non_stream_response_does_not_forward_upstream_content_length() {
        // The body is parsed and re-serialized, so its length can change; the
        // upstream content-length then disagrees with it and hyper panics.
        let upstream_body = r#"{"id": "msg_1", "type": "message", "content": []}"#;
        let upstream: reqwest::Response = axum::http::Response::builder()
            .header("content-type", "application/json")
            .header("content-length", upstream_body.len())
            .header("x-upstream-marker", "kept")
            .body(upstream_body)
            .unwrap()
            .into();

        let attribution = Attribution::new(
            Arc::new(crate::UsageRecorder::new(None)),
            "m",
            ProviderId::Copilot,
            "a",
        );
        let Ok(response) = forward_response(upstream, false, attribution, false).await else {
            panic!("a 200 upstream response must forward");
        };

        assert_eq!(response.headers()["x-upstream-marker"], "kept");
        let declared = response
            .headers()
            .get(axum::http::header::CONTENT_LENGTH)
            .cloned();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        if let Some(declared) = declared {
            assert_eq!(declared.to_str().unwrap(), body.len().to_string());
        }
        assert_ne!(
            body.len(),
            upstream_body.len(),
            "fixture must change length"
        );
    }
}
