//! Gemini executor — Google Generative Language API (OpenAI-compat shim).
//!
//! Routes through Google's official OpenAI-compatible endpoint:
//! `https://generativelanguage.googleapis.com/v1beta/openai/chat/completions`
//!
//! Auth:
//! - API key: `Authorization: Bearer {key}` (Google's shim accepts Bearer for
//!   API keys — identical to `x-goog-api-key` on the OpenAI-compat path).
//! - OAuth: `Authorization: Bearer {token}`.
//!
//! Both modes are handled identically by `aigw::openai_compat` since the shim
//! accepts a Bearer token for both auth schemes.
use crate::{http_util::ProviderHttp, registry};
use aigw::openai::translate::OpenAIResponseTranslator;
use aigw::openai::{HttpTransportConfig, OpenAIAuthConfig};
use aigw::openai_compat::translate::OpenAICompatRequestTranslator;
use aigw::openai_compat::{OpenAICompatConfig, OpenAICompatProvider, Quirks};
use aigw_core::translate::{RequestTranslator as _, ResponseTranslator as _};
use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_types::{
    ChatRequest, ProviderId, RateLimitStore,
    traits::{ByteStream, ProviderExecutor, ProviderResponse, Result},
};
use rquest::Client;
use secrecy::SecretString;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Google's OpenAI-compatible base URL (path `/chat/completions` appended by aigw).
const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta/openai";

/// Executor for the Google Gemini API via its OpenAI-compat shim.
pub struct GeminiExecutor {
    ph: ProviderHttp,
    api_key: Option<String>,
    base_url: String,
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

    /// Resolves the bearer token: API key if present, OAuth token otherwise.
    ///
    /// Google's OpenAI-compat endpoint accepts `Authorization: Bearer <API_KEY>`
    /// in addition to OAuth tokens, so no special header logic is required.
    async fn bearer_token(&self) -> Result<String> {
        crate::http_util::resolve_bearer_token(
            self.api_key.as_deref(),
            &self.auth,
            &ProviderId::Gemini,
        )
        .await
    }

    /// Builds an [`OpenAICompatProvider`] for the current request.
    ///
    /// The provider is constructed per-request because the OAuth token may
    /// change between calls.
    fn build_provider(&self, token: String) -> Result<OpenAICompatProvider> {
        OpenAICompatProvider::new(OpenAICompatConfig {
            name: "gemini".to_owned(),
            http: HttpTransportConfig {
                base_url: self.base_url.clone(),
                timeout_seconds: 600,
                default_headers: BTreeMap::new(),
            },
            auth: OpenAIAuthConfig {
                api_key: SecretString::from(token),
                organization: None,
                project: None,
            },
            quirks: Quirks::default(),
        })
        .map_err(|e| byokey_types::ByokError::Config(e.to_string()))
    }
}

#[async_trait]
impl ProviderExecutor for GeminiExecutor {
    async fn chat_completion(&self, request: ChatRequest) -> Result<ProviderResponse> {
        let stream = request.stream;

        let token = self.bearer_token().await?;
        let provider = self.build_provider(token)?;
        let translator = OpenAICompatRequestTranslator::new(&provider).map_err(
            |e: aigw::openai::OpenAITransportConfigError| {
                byokey_types::ByokError::Config(e.to_string())
            },
        )?;

        // Translate canonical ChatRequest → aigw ChatRequest → OpenAI-compat body.
        let aigw_request: aigw_core::model::ChatRequest =
            serde_json::from_value(request.into_body())
                .map_err(|e| byokey_types::ByokError::Translation(e.to_string()))?;

        let translated = if stream {
            translator.translate_stream_request(&aigw_request).map_err(
                |e: aigw_core::error::TranslateError| {
                    byokey_types::ByokError::Translation(e.to_string())
                },
            )?
        } else {
            translator.translate_request(&aigw_request).map_err(
                |e: aigw_core::error::TranslateError| {
                    byokey_types::ByokError::Translation(e.to_string())
                },
            )?
        };

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
            // Google's OpenAI-compat shim returns standard OpenAI SSE — passthrough.
            let byte_stream: ByteStream = ProviderHttp::byte_stream(resp);
            Ok(ProviderResponse::Stream(byte_stream))
        } else {
            let resp_bytes = resp.bytes().await.map_err(byokey_types::ByokError::from)?;
            let aigw_response = OpenAIResponseTranslator
                .translate_response(http::StatusCode::OK, &resp_bytes)
                .map_err(|e: aigw_core::error::TranslateError| {
                    byokey_types::ByokError::Translation(e.to_string())
                })?;
            let value = serde_json::to_value(aigw_response)
                .map_err(|e| byokey_types::ByokError::Translation(e.to_string()))?;
            Ok(ProviderResponse::Complete(value))
        }
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

    #[test]
    fn test_build_provider_api_key() {
        let (client, auth) = crate::http_util::test_auth();
        let ex = GeminiExecutor::builder()
            .http(client)
            .auth(auth)
            .api_key("AIza-test-key".to_owned())
            .build();
        let provider = ex
            .build_provider("AIza-test-key".to_owned())
            .expect("provider should build");
        assert_eq!(
            provider.base_url(),
            "https://generativelanguage.googleapis.com/v1beta/openai"
        );
        assert_eq!(provider.name(), "gemini");
    }

    #[test]
    fn test_build_provider_custom_base_url() {
        let (client, auth) = crate::http_util::test_auth();
        let ex = GeminiExecutor::builder()
            .http(client)
            .auth(auth)
            .base_url("https://custom.example.com/v1".to_owned())
            .build();
        let provider = ex
            .build_provider("token".to_owned())
            .expect("provider should build");
        assert_eq!(provider.base_url(), "https://custom.example.com/v1");
    }
}
