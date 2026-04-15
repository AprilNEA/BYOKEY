//! Kiro executor — Anthropic-compatible API served by Kiro.
//!
//! Kiro exposes an Anthropic Messages API at its own endpoint.
//! Format: `OpenAI` -> Anthropic (translate), Anthropic -> `OpenAI` (translate).
//!
//! Transport (URL/header construction) is delegated to
//! [`aigw::anthropic::Transport`], while HTTP sending uses `rquest`.
use crate::http_util::ProviderHttp;
use crate::registry;
use aigw::anthropic::translate::{AnthropicRequestTranslator, AnthropicResponseTranslator};
use aigw::anthropic::{AuthMode, Transport, TransportConfig};
use aigw_core::translate::{RequestTranslator as _, ResponseTranslator as _};
use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_types::{
    ChatRequest, ProviderId, RateLimitStore,
    traits::{ByteStream, ProviderExecutor, ProviderResponse, Result},
};
use rquest::Client;
use secrecy::SecretString;
use std::sync::Arc;

/// Default Kiro API base URL (origin only — Transport appends `/v1/messages`).
const DEFAULT_BASE_URL: &str = "https://api.kiro.dev";

/// Required Anthropic API version header value.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Executor for the Kiro API (Anthropic-compatible).
pub struct KiroExecutor {
    ph: ProviderHttp,
    api_key: Option<String>,
    base_url: String,
    auth: Arc<AuthManager>,
}

#[bon::bon]
impl KiroExecutor {
    /// Creates a new Kiro executor.
    #[builder]
    #[allow(clippy::needless_pass_by_value)]
    pub fn new(
        http: Client,
        auth: Arc<AuthManager>,
        api_key: Option<String>,
        base_url: Option<String>,
        ratelimit: Option<Arc<RateLimitStore>>,
    ) -> Self {
        let mut ph = ProviderHttp::new(http);
        if let Some(store) = ratelimit {
            ph = ph.with_ratelimit(store, ProviderId::Kiro);
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
        }
    }

    /// Returns the bearer token: API key if present, otherwise fetches an OAuth token.
    async fn bearer_token(&self) -> Result<String> {
        crate::http_util::resolve_bearer_token(
            self.api_key.as_deref(),
            &self.auth,
            &ProviderId::Kiro,
        )
        .await
    }

    /// Build an [`aigw::anthropic::Transport`] for the current request.
    fn build_transport(&self, token: String) -> Result<Transport> {
        Transport::new(TransportConfig {
            api_key: SecretString::from(token),
            auth_mode: AuthMode::Bearer,
            base_url: self.base_url.clone(),
            version: ANTHROPIC_VERSION.to_owned(),
            beta: None,
            extra_headers: reqwest::header::HeaderMap::new(),
            ..Default::default()
        })
        .map_err(|e| byokey_types::ByokError::Config(e.to_string()))
    }
}

#[async_trait]
impl ProviderExecutor for KiroExecutor {
    async fn chat_completion(&self, request: ChatRequest) -> Result<ProviderResponse> {
        let stream = request.stream;

        let token = self.bearer_token().await?;
        let transport = self.build_transport(token)?;
        let translator = AnthropicRequestTranslator::new(&transport, None);

        // Translate: BYOKEY ChatRequest → aigw ChatRequest → Anthropic body.
        let aigw_request: aigw_core::model::ChatRequest =
            serde_json::from_value(request.into_body())
                .map_err(|e| byokey_types::ByokError::Translation(e.to_string()))?;
        let translated = translator
            .translate_request(&aigw_request)
            .map_err(|e| byokey_types::ByokError::Translation(e.to_string()))?;

        // Build rquest from TranslatedRequest URL/headers + body.
        let mut builder = self.ph.client().post(&translated.url);
        for (name, value) in &translated.headers {
            if let Ok(v) = value.to_str() {
                builder = builder.header(name.as_str(), v);
            }
        }
        // Prevent compressed SSE streams from breaking the line scanner.
        let builder = builder
            .header("accept-encoding", "identity")
            .body(translated.body.to_vec());

        let resp = self.ph.send(builder).await?;

        if stream {
            let byte_stream: ByteStream = ProviderHttp::byte_stream(resp);
            Ok(ProviderResponse::Stream(
                super::claude::translate_claude_sse(byte_stream),
            ))
        } else {
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
        registry::models_for_provider(&ProviderId::Kiro)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_executor() -> KiroExecutor {
        let (client, auth) = crate::http_util::test_auth();
        KiroExecutor::builder().http(client).auth(auth).build()
    }

    #[test]
    fn test_supported_models_non_empty() {
        let ex = make_executor();
        assert!(!ex.supported_models().is_empty());
    }
}
