//! Kiro executor — Anthropic-compatible API served by Kiro.
//!
//! Kiro exposes an Anthropic Messages API at its own endpoint.
//! Format: `OpenAI` -> Anthropic (translate), Anthropic -> `OpenAI` (translate).
use crate::http_util::ProviderHttp;
use crate::registry;
use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_translate::{ClaudeToOpenAI, OpenAIToClaude};
use byokey_types::{
    ChatRequest, ProviderId, RateLimitStore,
    traits::{ProviderExecutor, ProviderResponse, RequestTranslator, ResponseTranslator, Result},
};
use rquest::Client;
use serde_json::Value;
use std::sync::Arc;

/// Default Kiro API base URL.
const DEFAULT_BASE_URL: &str = "https://api.kiro.dev";
/// Messages API path.
const API_PATH: &str = "/v1/messages";

/// Required Anthropic API version header value.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Executor for the Kiro API (Anthropic-compatible).
pub struct KiroExecutor {
    ph: ProviderHttp,
    api_key: Option<String>,
    api_url: String,
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
}

#[async_trait]
impl ProviderExecutor for KiroExecutor {
    async fn chat_completion(&self, request: ChatRequest) -> Result<ProviderResponse> {
        let stream = request.stream;
        let mut body = OpenAIToClaude.translate_request(request.into_body())?;
        body["stream"] = Value::Bool(stream);

        let token = self.bearer_token().await?;
        let builder = self
            .ph
            .client()
            .post(&self.api_url)
            .header("authorization", format!("Bearer {token}"))
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body);

        let resp = self.ph.send(builder).await?;

        if stream {
            Ok(ProviderResponse::Stream(
                super::claude::translate_claude_sse(ProviderHttp::byte_stream(resp)),
            ))
        } else {
            let json: Value = resp.json().await?;
            let translated = ClaudeToOpenAI.translate_response(json)?;
            Ok(ProviderResponse::Complete(translated))
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
