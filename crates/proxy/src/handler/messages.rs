//! Anthropic Messages API passthrough handler.
//!
//! Accepts requests in native Anthropic format and forwards them to the
//! provider [`route`] picks: `api.anthropic.com/v1/messages`, Copilot's own
//! Messages endpoint (see [`super::copilot_messages`]), Cursor (see
//! [`super::cursor_messages`]) or a configured Messages gateway (see
//! [`super::custom_messages`]). Request bodies are normalised first (see
//! [`super::normalize`]).
//!
//! The response (streaming SSE or complete JSON) is returned as-is (see
//! [`super::forward`]).

use axum::{extract::State, http::HeaderMap, response::Response};
use byokey_provider::claude::{ANTHROPIC_BETA, ANTHROPIC_VERSION, fingerprint_headers};
use byokey_provider::cloak::{derive_cc_entrypoint, inject_billing_header};
use byokey_types::{ByokError, ClaudeModel, ProviderId};
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

#[cfg(test)]
mod integration_tests;

/// Handles `POST /v1/messages` — Anthropic native format passthrough.
///
/// Authenticates with the Claude provider (API key or OAuth), then forwards
/// the request body verbatim to the Anthropic API and streams the response
/// back without translation.
pub async fn anthropic_messages(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: axum::extract::Json<Value>,
) -> Result<Response, ApiError> {
    super::record_model(&body.0).record(
        "stream",
        body.0
            .get("stream")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    );
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
    let config = state.config.load();
    let (provider, long_context) = route(&config, &mut body)?;
    sanitize_thinking(&mut body);
    let stream = body.get("stream").and_then(Value::as_bool).unwrap_or(false);
    let custom = config
        .providers
        .get(&provider)
        .and_then(|p| p.anthropic.as_ref());
    let beta = build_beta_header(
        &mut body,
        &headers,
        if custom.is_some() { "" } else { ANTHROPIC_BETA },
        long_context.then_some(CONTEXT_1M_BETA),
    );
    let accept = if stream {
        "text/event-stream"
    } else {
        "application/json"
    };

    if let Some(upstream) = custom {
        let request =
            super::custom_messages::request(&state.http, upstream, "/v1/messages", &beta)?
                .header("accept", accept)
                .header("accept-encoding", "identity")
                .json(&body);
        let model = body["model"].as_str().expect("model validated by route");
        let exchange = Exchange::start(&state.usage, &provider, model, "configured");
        let pending = exchange.track(request.send());
        return forward(pending, stream, exchange, false).await;
    }
    sanitize_system(&mut body);
    strip_invalid_thinking_signatures(&mut body);
    match provider.as_str() {
        "cursor" => {
            return super::cursor_messages::cursor_messages(&state, body, stream).await;
        }
        "copilot" => return copilot_messages(&state, body, stream, &beta).await,
        _ => {}
    }

    // Default: passthrough to Anthropic API.
    let provider_cfg = config.providers.get("claude");
    let api_key = provider_cfg.and_then(|pc| pc.api_key.as_ref());
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
        let provider_cfg = config.providers.get("claude");
        let (credential, account_id) =
            if let Some(key) = provider_cfg.and_then(|pc| pc.api_key.as_ref()) {
                (
                    Credential::ApiKey(key.resolve()?),
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

/// Split an explicit provider prefix or suffix from its upstream model ID.
/// Prefixes take precedence so legacy requests can carry upstream bracket suffixes.
pub(super) fn qualified_model(model: &str) -> Option<(&str, &str)> {
    model.split_once('/').or_else(|| {
        let (bare, provider) = model.strip_suffix(']')?.rsplit_once('[')?;
        (provider != "1m").then_some((provider, bare))
    })
}

/// The provider and long-context flag for a Messages request.
///
/// A provider prefix or `[provider]` suffix selects the provider and is
/// stripped from `body.model`, along with an optional `[1m]` suffix.
/// A Claude model otherwise goes where
/// `routes` sends it; any other model goes to `routes.default` (see
/// [`Routes::fallback`](byokey_config::Routes::fallback)). Cursor knows a
/// routed Claude model only by Anthropic's undated id, so `body.model`
/// becomes that id for Cursor. Disabled providers reject requests,
/// including requests with an explicit provider.
pub(super) fn route(
    config: &byokey_config::Config,
    body: &mut Value,
) -> Result<(String, bool), ByokError> {
    let mut long_context = take_long_context_suffix(body);
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .filter(|model| !model.is_empty())
        .ok_or_else(|| ByokError::InvalidRequest("model must be a nonempty string".into()))?;
    let provider = if let Some((provider, bare)) = qualified_model(model) {
        let provider = provider.to_owned();
        body["model"] = Value::String(bare.to_owned());
        // Accept `[1m]` on either side of the provider suffix.
        long_context |= take_long_context_suffix(body);
        if body["model"] == "" {
            return Err(ByokError::InvalidRequest(
                "model must be nonempty without the provider qualifier".into(),
            ));
        }
        provider
    } else if let Some(model) = ClaudeModel::from_id(model) {
        let provider = config.anthropic.routes.provider(model);
        if provider == "cursor" {
            body["model"] = Value::String(model.to_string());
        }
        provider.to_owned()
    } else {
        config.anthropic.routes.fallback().0.to_owned()
    };
    config.validate_anthropic_provider(&provider)?;
    if config.providers.get(&provider).is_some_and(|c| !c.enabled) {
        return Err(ByokError::UnsupportedProvider(format!(
            "{provider} is disabled"
        )));
    }
    Ok((provider, long_context))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_prefix_beats_the_routes_and_is_stripped() {
        let config = byokey_config::Config::from_yaml(
            "anthropic:\n  routes:\n    default: copilot\n    families:\n      sonnet: claude\n    models:\n      claude-opus-5-5: cursor\n",
        )
        .unwrap();
        let route = |model: &str| {
            let mut body = json!({"model": model});
            let (provider, _) = route(&config, &mut body).unwrap();
            (provider, body["model"].as_str().unwrap().to_owned())
        };
        assert_eq!(route("cursor/opus"), ("cursor".into(), "opus".into()));
        assert_eq!(
            route("copilot/claude-opus-5.5"),
            ("copilot".into(), "claude-opus-5.5".into())
        );
        assert_eq!(
            route("claude-opus-5.5"),
            ("cursor".into(), "claude-opus-5-5".into()),
            "a model's route, whatever its spelling, under Anthropic's id for Cursor"
        );
        assert_eq!(
            route("claude-haiku-4-5-20251001"),
            ("copilot".into(), "claude-haiku-4-5-20251001".into()),
            "Copilot takes the id as sent"
        );
        assert_eq!(
            route("claude-sonnet-5"),
            ("claude".into(), "claude-sonnet-5".into()),
            "a family's route"
        );
        assert_eq!(
            route("claude-fable-5-1"),
            ("copilot".into(), "claude-fable-5-1".into())
        );
        assert_eq!(
            route("claude-opus-4.8-fast"),
            ("copilot".into(), "claude-opus-4.8-fast".into()),
            "a variant follows the default"
        );
        assert!(super::route(&config, &mut json!({"model": "codex/gpt-5.4"})).is_err());
        let unrouted = byokey_config::Config::default();
        assert_eq!(
            super::route(&unrouted, &mut json!({"model": "claude-sonnet-5"})).unwrap(),
            ("claude".into(), false)
        );
    }

    #[test]
    fn a_long_context_model_still_gets_its_thinking_and_provider_resolved() {
        let config = byokey_config::Config::default();
        let mut body = json!({"model": "claude-opus-5-5[1m]", "thinking": {"type": "auto"}});
        assert_eq!(route(&config, &mut body).unwrap(), ("claude".into(), true));
        sanitize_thinking(&mut body);
        assert_eq!(body["thinking"]["type"], "adaptive");

        for model in [
            "copilot/claude-opus-5.5[1m]",
            "claude-opus-5.5[copilot][1m]",
            "claude-opus-5.5[1m][copilot]",
        ] {
            let mut body = json!({"model": model});
            assert_eq!(route(&config, &mut body).unwrap(), ("copilot".into(), true));
            assert_eq!(body["model"], "claude-opus-5.5");
        }
    }
}
