//! Anthropic Messages API passthrough handler.
//!
//! Accepts requests in native Anthropic format and forwards them to
//! either `api.anthropic.com/v1/messages` (default), Copilot's own
//! Messages endpoint (see [`super::copilot_messages`]) when
//! `claude.backend: copilot` is configured, or Cursor (see
//! [`super::cursor_messages`]) for `claude.backend: cursor` and
//! `cursor/<model>` model names. Request bodies are normalised first (see
//! [`super::normalize`]).
//!
//! The response (streaming SSE or complete JSON) is returned as-is (see
//! [`super::forward`]).

use axum::{extract::State, http::HeaderMap, response::Response};
use byokey_provider::claude::{ANTHROPIC_VERSION, fingerprint_headers};
use byokey_provider::cloak::{derive_cc_entrypoint, inject_billing_header};
use byokey_types::{ByokError, ProviderId};
use serde_json::Value;
use std::sync::Arc;

use super::copilot_messages::copilot_messages;
use super::forward::forward;
use super::normalize::{
    CONTEXT_1M_BETA, build_beta_header, sanitize_system, sanitize_thinking,
    strip_invalid_thinking_signatures, take_long_context_suffix,
};
use crate::exchange::Exchange;
use crate::{AppState, error::ApiError};

/// Handles `POST /v1/messages` — Anthropic native format passthrough.
///
/// Authenticates with the Claude provider (API key or OAuth), then forwards
/// the request body verbatim to the Anthropic API and streams the response
/// back without translation.
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
    tracing::debug!(
        %model, ?keys, auth = if is_oauth { "oauth" } else { "api_key" },
        beta = %beta, "anthropic passthrough"
    );

    let model_name = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();

    let exchange = Exchange::start(
        &state.usage,
        ProviderId::Claude,
        model_name,
        upstream.account_id,
    );
    let pending = exchange.track(builder.json(&body).send());
    forward(pending, stream, exchange, is_oauth).await
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
                let byokey_types::AccountToken { account_id, token } = state
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
}
