//! Anthropic Messages API passthrough handler.
//!
//! Accepts requests in native Anthropic format and forwards them to
//! either `api.anthropic.com/v1/messages` (default) or
//! `api.githubcopilot.com/v1/messages` when `claude.backend: copilot`
//! is configured.
//!
//! The response (streaming SSE or complete JSON) is returned as-is.

use aigw::anthropic::{AuthMode, Transport, TransportConfig};
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use byokey_provider::CopilotExecutor;
use byokey_provider::claude_headers::{ANTHROPIC_BETA, ANTHROPIC_VERSION};
use byokey_provider::cloak::{derive_cc_entrypoint, inject_billing_header};
use byokey_types::{ByokError, ProviderId, ThinkingCapability, traits::ByteStream};
use bytes::Bytes;
use futures_util::{StreamExt as _, TryStreamExt as _};
use secrecy::SecretString;
use serde_json::Value;
use std::fmt::Write as _;
use std::sync::Arc;

use crate::util::stream::{AnthropicParser, response_to_stream, tap_usage_stream};
use crate::util::{extract_usage, sse_response, strip_gateway_headers};
use crate::{AppState, UsageRecorder, error::ApiError};

/// Default thinking budget (tokens) for `Auto` mode on legacy Claude models
/// that require an explicit `budget_tokens` value with `thinking.type: "enabled"`.
const DEFAULT_AUTO_BUDGET: u32 = 10_000;

// Copilot identification headers (matching VS Code Copilot Chat extension).
const COPILOT_USER_AGENT: &str = "GitHubCopilotChat/0.35.0";
const COPILOT_EDITOR_VERSION: &str = "vscode/1.107.0";
const COPILOT_PLUGIN_VERSION: &str = "copilot-chat/0.35.0";
const COPILOT_INTEGRATION_ID: &str = "vscode-chat";
const COPILOT_OPENAI_INTENT: &str = "conversation-panel";
const COPILOT_GITHUB_API_VERSION: &str = "2025-04-01";

/// Handles `POST /v1/messages` — Anthropic native format passthrough.
///
/// Authenticates with the Claude provider (API key or OAuth), then forwards
/// the request body verbatim to the Anthropic API and streams the response
/// back without translation.
/// Strip empty system content to prevent "text content blocks must be non-empty" API error.
///
/// Handles both string (`"system": ""`) and array forms
/// (`"system": [{"type": "text", "text": ""}]`).
fn sanitize_system(body: &mut Value) {
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
/// stripping an optional `<prefix>#` cache key). Antigravity / Gemini
/// thinking blocks use a different format and would be rejected by the
/// Claude API if forwarded.
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
///
/// This handler operates on raw Anthropic-native JSON (no aigw round-trip),
/// so the structural source-tag check used by aigw-anthropic doesn't apply
/// here — the signature-prefix heuristic is the right tool for the
/// passthrough path.
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

/// Merge betas from the request body's `betas` array and the client's
/// `anthropic-beta` HTTP header into the base beta string, then strip the
/// body field so the upstream API doesn't reject it as unknown.
fn build_beta_header(body: &mut Value, client_headers: &HeaderMap) -> String {
    let mut betas = ANTHROPIC_BETA.to_string();

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

/// Detect the `X-Initiator` value from Anthropic-format messages.
fn detect_initiator(body: &Value) -> &'static str {
    let is_agent = body
        .get("messages")
        .and_then(Value::as_array)
        .is_some_and(|msgs| {
            msgs.iter().any(|m| {
                matches!(
                    m.get("role").and_then(Value::as_str),
                    Some("assistant" | "tool")
                )
            })
        });
    if is_agent { "agent" } else { "user" }
}

use byokey_provider::executor::claude::build_fingerprint_headers;

#[tracing::instrument(skip_all, fields(
    model = %body.0.get("model").and_then(serde_json::Value::as_str).unwrap_or("-"),
    stream = body.0.get("stream").and_then(serde_json::Value::as_bool).unwrap_or(false),
))]
#[allow(clippy::too_many_lines)] // Single-pass handler — keeping one function boundary is clearer than splitting.
pub async fn anthropic_messages(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::extract::Json<Value>,
) -> Result<Response, ApiError> {
    let mut body = body.0;
    sanitize_system(&mut body);
    sanitize_thinking(&mut body);
    strip_invalid_thinking_signatures(&mut body);
    let stream = body.get("stream").and_then(Value::as_bool).unwrap_or(false);
    let beta = build_beta_header(&mut body, &headers);

    // Global backend override: `claude.backend: copilot`.
    let config = state.config.load();
    let claude_config = config
        .providers
        .get(&ProviderId::Claude)
        .cloned()
        .unwrap_or_default();

    if claude_config.backend.as_ref() == Some(&ProviderId::Copilot) {
        return copilot_messages(&state, body, stream, &beta).await;
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
            Some(&profile.device_id),
            Some(&account_uuid),
            Some(&profile.session_id),
            entrypoint,
            workload,
        );
        byokey_provider::cloak::remap_tool_names_request(&mut body);
    }

    // Resolve auth credential + mode, capturing the account_id used for
    // per-account usage attribution.
    let (credential, auth_mode, account_id) = if let Some(key) = api_key {
        (
            key,
            AuthMode::ApiKey,
            byokey_types::DEFAULT_ACCOUNT.to_string(),
        )
    } else {
        let (account_id, token) = state
            .auth
            .get_token_with_account(&ProviderId::Claude)
            .await
            .map_err(ApiError::from)?;
        (token.access_token, AuthMode::Bearer, account_id)
    };

    // Build Transport — handles auth header, version, beta, and fingerprint.
    let transport = Transport::new(TransportConfig {
        api_key: SecretString::from(credential),
        auth_mode,
        base_url: provider_cfg
            .and_then(|pc| pc.base_url.clone())
            .unwrap_or_else(|| "https://api.anthropic.com".to_owned()),
        beta: Some(beta.clone()),
        extra_headers: build_fingerprint_headers(&profile, !is_oauth),
        ..Default::default()
    })
    .map_err(|e| ApiError(ByokError::Config(e.to_string())))?;

    let api_url = format!("{}?beta=true", transport.url("/v1/messages"));

    let accept = if stream {
        "text/event-stream"
    } else {
        "application/json"
    };

    // Apply Transport headers to rquest builder.
    let mut builder = state.http.post(&api_url);
    for (name, value) in transport.headers() {
        if let Ok(v) = value.to_str() {
            builder = builder.header(name.as_str(), v);
        }
    }
    let builder = builder
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

    let resp = builder
        .json(&body)
        .send()
        .await
        .map_err(|e| ApiError(ByokError::from(e)))?;

    forward_response(
        resp,
        stream,
        &state.usage,
        &model_name,
        "claude",
        &account_id,
        is_oauth,
    )
    .await
}

/// Build a Copilot Messages API request with standard headers.
fn build_copilot_messages_request(
    http: &rquest::Client,
    url: &str,
    token: &str,
    beta: &str,
    accept: &str,
    initiator: &str,
    body: &Value,
) -> rquest::RequestBuilder {
    http.post(url)
        .header("authorization", format!("Bearer {token}"))
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("anthropic-beta", beta)
        .header("content-type", "application/json")
        .header("accept", accept)
        .header("user-agent", COPILOT_USER_AGENT)
        .header("editor-version", COPILOT_EDITOR_VERSION)
        .header("editor-plugin-version", COPILOT_PLUGIN_VERSION)
        .header("copilot-integration-id", COPILOT_INTEGRATION_ID)
        .header("openai-intent", COPILOT_OPENAI_INTENT)
        .header("x-github-api-version", COPILOT_GITHUB_API_VERSION)
        .header("x-initiator", initiator)
        .json(body)
}

/// Route Anthropic-format request to Copilot's native `/v1/messages` endpoint.
///
/// Copilot provides a native Anthropic-compatible Messages API at
/// `api.githubcopilot.com/v1/messages`. This handler authenticates via
/// the Copilot token exchange flow and forwards the request verbatim.
///
/// With multiple Copilot accounts, retries with quota-aware rotation
/// on transient failures.
#[allow(clippy::too_many_lines)]
#[tracing::instrument(skip_all, fields(
    model = %body.get("model").and_then(serde_json::Value::as_str).unwrap_or("-"),
    stream,
    attempt = tracing::field::Empty,
))]
async fn copilot_messages(
    state: &Arc<AppState>,
    body: Value,
    stream: bool,
    beta: &str,
) -> Result<Response, ApiError> {
    let copilot_config = state
        .config
        .load()
        .providers
        .get(&ProviderId::Copilot)
        .cloned()
        .unwrap_or_default();

    let executor = CopilotExecutor::builder()
        .http(state.http.clone())
        .auth(state.auth.clone())
        .maybe_api_key(copilot_config.api_key)
        .maybe_base_url(copilot_config.base_url)
        .ratelimit(state.ratelimits.clone())
        .build();

    let accounts = state
        .auth
        .list_accounts(&ProviderId::Copilot)
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
    let initiator = detect_initiator(&body);
    let model_name = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();

    let mut last_err = None;
    for attempt in 0..max_attempts {
        tracing::Span::current().record("attempt", attempt);
        let (token, endpoint) = match executor.copilot_token().await {
            Ok(t) => t,
            Err(e) => {
                if max_attempts > 1 {
                    tracing::warn!(attempt, error = %e, "copilot token failed, trying next account");
                    CopilotExecutor::invalidate_current_account();
                    last_err = Some(ApiError::from(e));
                    continue;
                }
                return Err(ApiError::from(e));
            }
        };
        let url = format!("{endpoint}/v1/messages");

        tracing::info!(
            url = %url,
            model = %body.get("model").and_then(|v| v.as_str()).unwrap_or("unknown"),
            stream, initiator, attempt,
            "routing Anthropic messages through Copilot"
        );

        let resp = build_copilot_messages_request(
            &state.http,
            &url,
            &token,
            beta,
            accept,
            initiator,
            &body,
        )
        .send()
        .await;

        match resp {
            Ok(r) if r.status().is_success() => {
                // Copilot does its own account rotation inside CopilotExecutor;
                // the specific account isn't easily exposed here yet, so we
                // attribute to DEFAULT_ACCOUNT for now.
                return forward_response(
                    r,
                    stream,
                    &state.usage,
                    &model_name,
                    "copilot",
                    byokey_types::DEFAULT_ACCOUNT,
                    false,
                )
                .await;
            }
            Ok(r) => {
                let status = r.status().as_u16();
                let text = r.text().await.unwrap_or_default();
                let err = ByokError::Upstream {
                    status,
                    body: text,
                    retry_after: None,
                };
                if !err.is_retryable() || attempt + 1 >= max_attempts {
                    return Err(ApiError(err));
                }
                tracing::warn!(
                    attempt,
                    status,
                    "copilot messages failed, trying next account"
                );
                CopilotExecutor::invalidate_current_account();
                last_err = Some(ApiError(err));
            }
            Err(e) => {
                let err = ByokError::from(e);
                if !err.is_retryable() || attempt + 1 >= max_attempts {
                    return Err(ApiError(err));
                }
                tracing::warn!(attempt, error = %err, "copilot messages transport error, trying next");
                CopilotExecutor::invalidate_current_account();
                last_err = Some(ApiError(err));
            }
        }
    }

    tracing::error!(
        attempts = max_attempts,
        "all copilot accounts exhausted for messages request"
    );
    state
        .usage
        .record_failure_for(&model_name, "copilot", byokey_types::DEFAULT_ACCOUNT);
    Err(last_err
        .unwrap_or_else(|| ApiError(ByokError::Auth("no copilot accounts available".into()))))
}

/// Forward an upstream response back to the client, recording token usage.
async fn forward_response(
    resp: rquest::Response,
    stream: bool,
    usage: &Arc<UsageRecorder>,
    model: &str,
    provider: &str,
    account_id: &str,
    reverse_remap_tools: bool,
) -> Result<Response, ApiError> {
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        tracing::error!(
            status = status.as_u16(),
            body = %text,
            "anthropic upstream error (non-retryable)"
        );
        usage.record_failure_for(model, provider, account_id);
        return Err(ApiError::from(ByokError::Upstream {
            status: status.as_u16(),
            body: text,
            retry_after: None,
        }));
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

    if stream {
        let raw = response_to_stream(resp);
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
        let tapped = tap_usage_stream(
            remapped,
            usage.clone(),
            model.to_string(),
            provider.to_string(),
            account_id.to_string(),
            AnthropicParser::new(),
        );
        let mapped = tapped.map_err(|e| std::io::Error::other(e.to_string()));
        let mut sse = sse_response(upstream_status, mapped);
        // Merge upstream headers (gateway-stripped) into the SSE response.
        // We do not overwrite the SSE-specific headers set by sse_response.
        for (name, value) in &upstream_headers {
            sse.headers_mut()
                .entry(name)
                .or_insert_with(|| value.clone());
        }
        Ok(sse)
    } else {
        let mut json: Value = resp
            .json()
            .await
            .map_err(|e| ApiError(ByokError::from(e)))?;
        if reverse_remap_tools {
            byokey_provider::cloak::reverse_remap_tool_names_response(&mut json);
        }
        let (input, output) = extract_usage(&json, "/usage/input_tokens", "/usage/output_tokens");
        usage.record_success_for(model, provider, account_id, input, output);
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── sanitize_thinking: tool_choice conflict ────────────────────────

    #[test]
    fn tool_choice_any_strips_thinking() {
        let mut body = json!({
            "model": "claude-opus-4-6",
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
            "model": "claude-opus-4-6",
            "thinking": {"type": "adaptive"},
            "tool_choice": {"type": "tool", "name": "get_weather"}
        });
        sanitize_thinking(&mut body);
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn tool_choice_auto_does_not_strip() {
        let mut body = json!({
            "model": "claude-opus-4-6",
            "thinking": {"type": "adaptive"},
            "tool_choice": {"type": "auto"}
        });
        sanitize_thinking(&mut body);
        assert_eq!(body["thinking"]["type"], "adaptive");
    }

    // ── sanitize_thinking: "auto" translation ──────────────────────────

    #[test]
    fn auto_on_hybrid_model_becomes_adaptive() {
        // claude-opus-4-6 is Hybrid → should translate to "adaptive".
        let mut body = json!({
            "model": "claude-opus-4-6",
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
            "model": "claude-opus-4-6",
            "thinking": {"type": "enabled", "budget_tokens": 8000}
        });
        sanitize_thinking(&mut body);
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 8000);
    }

    #[test]
    fn adaptive_type_passes_through() {
        let mut body = json!({
            "model": "claude-opus-4-6",
            "thinking": {"type": "adaptive"}
        });
        sanitize_thinking(&mut body);
        assert_eq!(body["thinking"]["type"], "adaptive");
    }

    #[test]
    fn no_thinking_field_is_noop() {
        let mut body = json!({"model": "claude-opus-4-6", "max_tokens": 1024});
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
            "model": "claude-opus-4-6",
            "temperature": 0,
            "thinking": {"type": "adaptive"}
        });
        sanitize_thinking(&mut body);
        assert_eq!(body["temperature"], 1);
    }

    #[test]
    fn enabled_thinking_coerces_temperature_to_one() {
        let mut body = json!({
            "model": "claude-opus-4-6",
            "temperature": 0.2,
            "thinking": {"type": "enabled", "budget_tokens": 2048}
        });
        sanitize_thinking(&mut body);
        assert_eq!(body["temperature"], 1);
    }

    #[test]
    fn temperature_one_with_thinking_is_unchanged() {
        let mut body = json!({
            "model": "claude-opus-4-6",
            "temperature": 1,
            "thinking": {"type": "adaptive"}
        });
        sanitize_thinking(&mut body);
        assert_eq!(body["temperature"], 1);
    }

    #[test]
    fn no_thinking_leaves_temperature_alone() {
        let mut body = json!({
            "model": "claude-opus-4-6",
            "temperature": 0,
            "messages": [{"role": "user", "content": "hi"}]
        });
        sanitize_thinking(&mut body);
        assert_eq!(body["temperature"], 0);
    }

    #[test]
    fn forced_tool_choice_strips_thinking_keeps_temperature() {
        let mut body = json!({
            "model": "claude-opus-4-6",
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
            "model": "claude-opus-4-6",
            "thinking": {"type": "adaptive"}
        });
        sanitize_thinking(&mut body);
        assert!(body.get("temperature").is_none());
    }
}
