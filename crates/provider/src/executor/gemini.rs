//! Gemini executor — Google Generative Language API.
//!
//! Uses Gemini's OpenAI-compatible endpoint for simplicity.
//! Auth: `Authorization: Bearer {token}` for OAuth, `x-goog-api-key` for API key.
use crate::{http_util::ProviderHttp, registry};
use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_types::{
    ChatRequest, ProviderId, RateLimitStore,
    traits::{ProviderExecutor, ProviderResponse, Result},
};
use rquest::Client;
use std::sync::Arc;

/// Default Gemini API base URL.
const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com";
/// OpenAI-compatible chat completions path.
const API_PATH: &str = "/v1beta/openai/chat/completions";

/// Executor for the Google Gemini API.
pub struct GeminiExecutor {
    ph: ProviderHttp,
    api_key: Option<String>,
    api_url: String,
    auth: Arc<AuthManager>,
}

#[bon::bon]
impl GeminiExecutor {
    /// Creates a new Gemini executor.
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
            ph = ph.with_ratelimit(store, ProviderId::Gemini);
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

    /// Returns the auth header: `x-goog-api-key` for API keys, `Authorization: Bearer` for OAuth.
    async fn auth_header(&self) -> Result<(&'static str, String)> {
        if let Some(key) = &self.api_key {
            return Ok(("x-goog-api-key", key.clone()));
        }
        let token = self.auth.get_token(&ProviderId::Gemini).await?;
        Ok(("authorization", format!("Bearer {}", token.access_token)))
    }
}

#[async_trait]
impl ProviderExecutor for GeminiExecutor {
    async fn chat_completion(&self, request: ChatRequest) -> Result<ProviderResponse> {
        let stream = request.stream;
        let mut body = request.into_body();
        body["stream"] = serde_json::Value::Bool(stream);
        crate::http_util::ensure_stream_options(&mut body, stream);

        let (header_name, header_value) = self.auth_header().await?;

        let builder = self
            .ph
            .client()
            .post(&self.api_url)
            .header(header_name, header_value)
            .header("content-type", "application/json")
            .json(&body);

        self.ph.send_passthrough(builder, stream).await
    }

    fn supported_models(&self) -> Vec<String> {
        registry::models_for_provider(&ProviderId::Gemini)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_executor() -> GeminiExecutor {
        let (client, auth) = crate::http_util::test_auth();
        GeminiExecutor::builder().http(client).auth(auth).build()
    }

    #[test]
    fn test_supported_models_non_empty() {
        let ex = make_executor();
        assert!(!ex.supported_models().is_empty());
    }

    #[test]
    fn test_supported_models_start_with_gemini() {
        let ex = make_executor();
        assert!(
            ex.supported_models()
                .iter()
                .all(|m| m.starts_with("gemini-"))
        );
    }
}
