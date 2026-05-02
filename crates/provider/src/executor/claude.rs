//! Claude executor — Anthropic Messages API.
//!
//! Auth: `x-api-key` for raw API keys, `Authorization: Bearer` for OAuth tokens.
//! Format: `OpenAI` -> Anthropic (translate), Anthropic -> `OpenAI` (translate).
//!
//! Transport (URL/header construction) is delegated to
//! [`aigw::anthropic::Transport`], while HTTP sending uses `rquest` for TLS
//! fingerprinting.
use crate::cloak;
use crate::device_profile::{DeviceProfile, DeviceProfileCache};
use crate::http_util::ProviderHttp;
use crate::registry;
use aigw::anthropic::translate::{AnthropicRequestTranslator, AnthropicResponseTranslator};
use aigw::anthropic::{AuthMode as AigwAuthMode, Transport, TransportConfig};
use aigw_core::translate::{RequestTranslator as _, ResponseTranslator as _};
use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_config::CloakConfig;
use byokey_translate::inject_cache_control;
use byokey_types::{
    ChatRequest, ProviderId, RateLimitStore,
    traits::{ByteStream, ProviderExecutor, ProviderResponse, Result},
};
use bytes::Bytes;
use futures_util::{StreamExt as _, stream::try_unfold};
use rquest::Client;
use secrecy::SecretString;
use serde_json::Value;
use std::sync::Arc;

/// Default Anthropic API base URL.
const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

// ── Shared Claude fingerprint constants ─────────────────────────────
// Re-exported via `byokey_provider::claude_headers` so the proxy crate's
// `/v1/messages` passthrough handler stays in sync.

/// Required Anthropic API version header value.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Beta features to enable.
///
/// `prompt-caching-2024-07-31` removed: prompt caching is GA since Dec 2024;
/// the beta header is now rejected by the API.
pub const ANTHROPIC_BETA: &str = "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14,redact-thinking-2026-02-12,context-management-2025-06-27,prompt-caching-scope-2026-01-05,advanced-tool-use-2025-11-20,effort-2025-11-24,structured-outputs-2025-12-15,fast-mode-2026-02-01,token-efficient-tools-2026-03-28";

/// User-Agent matching the Claude CLI version.
pub const USER_AGENT: &str = "claude-cli/2.1.109 (external, cli)";

/// Anthropic SDK package version (matches @anthropic-ai/sdk bundled with CLI 2.1.109).
pub const SDK_PACKAGE_VERSION: &str = "0.74.0";

/// Node.js runtime version for X-Stainless-Runtime-Version.
pub const RUNTIME_VERSION: &str = "v24.14.1";

/// Credential resolved at request time.
enum Credential {
    /// Raw API key.
    ApiKey(String),
    /// OAuth token.
    Bearer(String),
}

/// Build the `extra_headers` for [`TransportConfig`] from a device fingerprint.
///
/// Includes all headers that a real Claude Code CLI sends:
/// - Device fingerprint (`x-stainless-*`, `user-agent`)
/// - Session identity (`x-claude-code-session-id`)
/// - Request identity (`x-client-request-id`)
/// - Access flags (`x-app`)
///
/// The `anthropic-dangerous-direct-browser-access` header is included only
/// when `is_api_key` is `true` (raw `x-api-key` auth). On OAuth
/// (`Authorization: Bearer`) the real Claude Code CLI omits this header.
///
/// # Panics
///
/// Panics if any profile field contains non-ASCII characters that are invalid
/// in HTTP header values. All default profiles use ASCII-only values.
pub fn build_fingerprint_headers(
    profile: &DeviceProfile,
    is_api_key: bool,
) -> reqwest::header::HeaderMap {
    let mut h = reqwest::header::HeaderMap::new();
    if is_api_key {
        h.insert(
            "anthropic-dangerous-direct-browser-access",
            "true".parse().expect("static header"),
        );
    }
    h.insert("x-app", "cli".parse().expect("static header"));
    h.insert(
        reqwest::header::USER_AGENT,
        profile.user_agent.parse().expect("valid user-agent"),
    );
    // Claude Code session ID — one per CLI process, stable across requests.
    h.insert(
        "x-claude-code-session-id",
        profile.session_id.parse().expect("valid session id"),
    );
    h.insert("x-stainless-lang", "js".parse().expect("static header"));
    h.insert(
        "x-stainless-runtime",
        "node".parse().expect("static header"),
    );
    h.insert(
        "x-stainless-runtime-version",
        profile
            .runtime_version
            .parse()
            .expect("valid runtime version"),
    );
    h.insert(
        "x-stainless-package-version",
        profile
            .package_version
            .parse()
            .expect("valid package version"),
    );
    h.insert("x-stainless-os", profile.os.parse().expect("valid os"));
    h.insert(
        "x-stainless-arch",
        profile.arch.parse().expect("valid arch"),
    );
    h.insert(
        "x-stainless-retry-count",
        "0".parse().expect("static header"),
    );
    h.insert("x-stainless-timeout", "600".parse().expect("static header"));
    // Per-request unique identifier.
    h.insert(
        "x-client-request-id",
        uuid::Uuid::new_v4()
            .to_string()
            .parse()
            .expect("valid uuid"),
    );
    h
}

/// Executor for the Anthropic Claude API.
pub struct ClaudeExecutor {
    ph: ProviderHttp,
    api_key: Option<String>,
    base_url: String,
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
        let base_url = base_url
            .as_deref()
            .unwrap_or(DEFAULT_BASE_URL)
            .trim_end_matches('/')
            .to_owned();
        Self {
            ph,
            api_key,
            base_url,
            auth,
            profile_cache,
            cloak_config,
        }
    }

    /// Resolves the credential: API key if present, otherwise OAuth token.
    async fn get_credential(&self) -> Result<Credential> {
        if let Some(key) = &self.api_key {
            return Ok(Credential::ApiKey(key.clone()));
        }
        let token = self.auth.get_token(&ProviderId::Claude).await?;
        Ok(Credential::Bearer(token.access_token))
    }

    /// Build an [`aigw::anthropic::Transport`] for the current request.
    ///
    /// The transport pre-builds all standard Anthropic headers (auth, version,
    /// beta) plus device-fingerprint headers, so callers only need to copy them
    /// into the `rquest` builder.
    fn build_transport(
        &self,
        credential: &Credential,
        fingerprint: &DeviceProfile,
    ) -> Result<Transport> {
        let (secret, auth_mode) = match credential {
            Credential::ApiKey(k) => (k.clone(), AigwAuthMode::ApiKey),
            Credential::Bearer(t) => (t.clone(), AigwAuthMode::Bearer),
        };
        Transport::new(TransportConfig {
            api_key: SecretString::from(secret),
            auth_mode,
            base_url: self.base_url.clone(),
            version: ANTHROPIC_VERSION.to_owned(),
            beta: Some(ANTHROPIC_BETA.to_owned()),
            extra_headers: build_fingerprint_headers(fingerprint, matches!(credential, Credential::ApiKey(_))),
            ..Default::default()
        })
        .map_err(|e| byokey_types::ByokError::Config(e.to_string()))
    }
}

#[async_trait]
impl ProviderExecutor for ClaudeExecutor {
    async fn chat_completion(&self, request: ChatRequest) -> Result<ProviderResponse> {
        let stream = request.stream;
        let credential = self.get_credential().await?;

        // Resolve fingerprint: use cached profile when available, else statics.
        let scope_key = match &credential {
            Credential::ApiKey(k) => format!("api_key:{k}"),
            Credential::Bearer(_) => "global".to_string(),
        };
        let fingerprint = self
            .profile_cache
            .as_ref()
            .map_or_else(DeviceProfile::default, |cache| cache.resolve(&scope_key));

        // Build Transport + Translator.
        let transport = self.build_transport(&credential, &fingerprint)?;
        let translator = AnthropicRequestTranslator::new(&transport, None);

        // Translate: BYOKEY ChatRequest → aigw ChatRequest → Anthropic body.
        let aigw_request: aigw_core::model::ChatRequest =
            serde_json::from_value(request.into_body())
                .map_err(|e| byokey_types::ByokError::Translation(e.to_string()))?;
        let translated = translator
            .translate_request(&aigw_request)
            .map_err(|e| byokey_types::ByokError::Translation(e.to_string()))?;

        // Post-process on the Anthropic-format body (cache control, temperature, cloaking).
        let mut body: Value = serde_json::from_slice(&translated.body)
            .map_err(|e| byokey_types::ByokError::Translation(e.to_string()))?;
        body = inject_cache_control(body);
        normalize_temperature_for_thinking(&mut body);

        // Apply cloaking with identity from the device profile.
        if let Some(ref cc) = self.cloak_config
            && cc.enabled
        {
            // account_uuid: derive a stable UUID from the scope key.
            let account_uuid =
                uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, scope_key.as_bytes()).to_string();
            cloak::apply_cloaking(
                &mut body,
                cc,
                &fingerprint.device_id,
                &account_uuid,
                &fingerprint.session_id,
            );
        }

        // Build rquest from TranslatedRequest URL/headers + modified body.
        let api_url = format!("{}?beta=true", translated.url);
        let mut builder = self.ph.client().post(&api_url);
        for (name, value) in &translated.headers {
            if let Ok(v) = value.to_str() {
                builder = builder.header(name.as_str(), v);
            }
        }
        // Prevent compressed SSE streams from breaking the line scanner.
        let builder = builder.header("accept-encoding", "identity");

        let resp = self.ph.send(builder.json(&body)).await?;

        if stream {
            let byte_stream: ByteStream = ProviderHttp::byte_stream(resp);
            Ok(ProviderResponse::Stream(translate_claude_sse(byte_stream)))
        } else {
            // Use aigw's response translator for non-streaming responses.
            let resp_bytes = resp.bytes().await.map_err(byokey_types::ByokError::from)?;
            let aigw_response = AnthropicResponseTranslator
                .translate_response(http::StatusCode::OK, &resp_bytes)
                .map_err(|e| byokey_types::ByokError::Translation(e.to_string()))?;
            let value = serde_json::to_value(aigw_response)
                .map_err(|e| byokey_types::ByokError::Translation(e.to_string()))?;
            Ok(ProviderResponse::Complete(value))
        }
    }

    fn supported_models(&self) -> Vec<String> {
        registry::models_for_provider(&ProviderId::Claude)
    }
}

/// Force `temperature` to `1` when thinking is active.
///
/// Anthropic API returns 400 if temperature != 1 while `thinking.type` is
/// `enabled` or `adaptive`. This runs after aigw translation so the body is
/// already in Anthropic format.
fn normalize_temperature_for_thinking(body: &mut Value) {
    let thinking_active = body
        .get("thinking")
        .and_then(|th| th.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|t| matches!(t, "enabled" | "adaptive"));

    if !thinking_active {
        return;
    }

    match body.get("temperature") {
        None => {}
        Some(v) if v.as_f64() == Some(1.0) => {}
        Some(_) => {
            body["temperature"] = serde_json::json!(1);
        }
    }
}

/// Wraps a raw Claude SSE `ByteStream` and translates its events to
/// `OpenAI` chat completion chunk SSE format line-by-line.
///
/// Delegates semantic parsing to [`aigw`]'s `AnthropicStreamParser`, then
/// converts the canonical `StreamEvent`s to `OpenAI` SSE bytes via
/// [`stream_bridge`](crate::stream_bridge).
pub(crate) fn translate_claude_sse(inner: ByteStream) -> ByteStream {
    use crate::stream_bridge::{SseContext, stream_events_to_sse};
    use aigw::anthropic::translate::AnthropicStreamParser;
    use aigw_core::translate::StreamParser;

    struct State {
        inner: ByteStream,
        buf: Vec<u8>,
        parser: AnthropicStreamParser,
        ctx: SseContext,
        done: bool,
    }

    Box::pin(try_unfold(
        State {
            inner,
            buf: Vec::new(),
            parser: AnthropicStreamParser::new(),
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
                                    // Check if Done was emitted.
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
                                tracing::warn!(error = %e, "claude stream parse error");
                            }
                            _ => {} // empty events (ping, content_block_stop)
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
