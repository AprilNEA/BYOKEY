//! Qwen executor — Alibaba Qwen (Tongyi Qianwen) API.
//!
//! Uses Qwen's OpenAI-compatible endpoint via `aigw::openai_compat`.
//! Auth: `Authorization: Bearer {token}` for both OAuth and API key.
use crate::http_util::ProviderHttp;
use crate::registry;
use aigw::openai::translate::OpenAIResponseTranslator;
use aigw::openai::{HttpTransportConfig, OpenAIAuthConfig};
use aigw::openai_compat::translate::OpenAICompatRequestTranslator;
use aigw::openai_compat::{OpenAICompatConfig, OpenAICompatProvider, Quirks};
use aigw_core::translate::{RequestTranslator as _, ResponseTranslator as _};
use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_types::{
    ByokError, ChatRequest, ProviderId, RateLimitStore,
    traits::{ByteStream, ProviderExecutor, ProviderResponse, Result},
};
use secrecy::SecretString;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Default Qwen API base URL (`/v1` suffix; aigw appends `/chat/completions`).
const DEFAULT_BASE_URL: &str = "https://portal.qwen.ai/v1";

const DEFAULT_USER_AGENT: &str = "QwenCode/0.10.3 (darwin; arm64)";

/// Executor for the Alibaba Qwen API.
pub struct QwenExecutor {
    ph: ProviderHttp,
    api_key: Option<String>,
    base_url: String,
    auth: Arc<AuthManager>,
    user_agent: String,
}

#[bon::bon]
impl QwenExecutor {
    /// Creates a new Qwen executor.
    #[builder]
    #[allow(clippy::needless_pass_by_value)]
    pub fn new(
        http: rquest::Client,
        auth: Arc<AuthManager>,
        api_key: Option<String>,
        base_url: Option<String>,
        ratelimit: Option<Arc<RateLimitStore>>,
        user_agent: Option<String>,
    ) -> Self {
        let mut ph = ProviderHttp::new(http);
        if let Some(store) = ratelimit {
            ph = ph.with_ratelimit(store, ProviderId::Qwen);
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
            user_agent: user_agent.unwrap_or_else(|| DEFAULT_USER_AGENT.to_string()),
        }
    }

    /// Returns the Bearer token: API key if configured, otherwise OAuth access token.
    async fn bearer_token(&self) -> Result<String> {
        crate::http_util::resolve_bearer_token(
            self.api_key.as_deref(),
            &self.auth,
            &ProviderId::Qwen,
        )
        .await
    }

    /// Builds an [`OpenAICompatProvider`] for a single request.
    fn build_provider(&self, token: String) -> Result<OpenAICompatProvider> {
        let mut default_headers = BTreeMap::new();
        default_headers.insert("user-agent".to_owned(), self.user_agent.clone());
        default_headers.insert("x-dashscope-useragent".to_owned(), self.user_agent.clone());
        default_headers.insert("x-dashscope-authtype".to_owned(), "qwen-oauth".to_owned());
        default_headers.insert(
            "x-stainless-runtime-version".to_owned(),
            "v24.14.1".to_owned(),
        );
        default_headers.insert("x-stainless-lang".to_owned(), "js".to_owned());
        default_headers.insert("x-stainless-arch".to_owned(), "arm64".to_owned());
        default_headers.insert(
            "x-stainless-package-version".to_owned(),
            "5.11.0".to_owned(),
        );
        default_headers.insert("x-dashscope-cachecontrol".to_owned(), "enable".to_owned());
        default_headers.insert("x-stainless-retry-count".to_owned(), "0".to_owned());
        default_headers.insert("x-stainless-os".to_owned(), "MacOS".to_owned());
        default_headers.insert("x-stainless-runtime".to_owned(), "node".to_owned());

        OpenAICompatProvider::new(OpenAICompatConfig {
            name: "qwen".to_owned(),
            http: HttpTransportConfig {
                base_url: self.base_url.clone(),
                timeout_seconds: 600,
                default_headers,
            },
            auth: OpenAIAuthConfig {
                api_key: SecretString::from(token),
                organization: None,
                project: None,
            },
            quirks: Quirks::default(),
        })
        .map_err(|e| ByokError::Config(e.to_string()))
    }
}

#[async_trait]
impl ProviderExecutor for QwenExecutor {
    async fn chat_completion(&self, request: ChatRequest) -> Result<ProviderResponse> {
        let stream = request.stream;

        let token = self.bearer_token().await?;
        let provider = self.build_provider(token)?;
        let translator = OpenAICompatRequestTranslator::new(&provider)
            .map_err(|e| ByokError::Config(e.to_string()))?;

        // Translate byokey ChatRequest → aigw ChatRequest → OpenAI body.
        let aigw_request: aigw_core::model::ChatRequest =
            serde_json::from_value(request.into_body())
                .map_err(|e| ByokError::Translation(e.to_string()))?;

        let translated = if stream {
            translator
                .translate_stream_request(&aigw_request)
                .map_err(|e| ByokError::Translation(e.to_string()))?
        } else {
            translator
                .translate_request(&aigw_request)
                .map_err(|e| ByokError::Translation(e.to_string()))?
        };

        // Build rquest from TranslatedRequest URL/headers + body.
        let mut builder = self.ph.client().post(&translated.url);
        for (name, value) in &translated.headers {
            if let Ok(v) = value.to_str() {
                builder = builder.header(name.as_str(), v);
            }
        }
        let builder = builder
            .header("accept-encoding", "identity")
            .body(translated.body.to_vec());

        let resp = self.ph.send(builder).await?;

        if stream {
            let byte_stream: ByteStream = ProviderHttp::byte_stream(resp);
            Ok(ProviderResponse::Stream(byte_stream))
        } else {
            let resp_bytes = resp.bytes().await.map_err(ByokError::from)?;
            let aigw_response = OpenAIResponseTranslator
                .translate_response(http::StatusCode::OK, &resp_bytes)
                .map_err(|e| ByokError::Translation(e.to_string()))?;
            let value = serde_json::to_value(aigw_response)
                .map_err(|e| ByokError::Translation(e.to_string()))?;
            Ok(ProviderResponse::Complete(value))
        }
    }

    fn supported_models(&self) -> Vec<String> {
        registry::models_for_provider(&ProviderId::Qwen)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_executor() -> QwenExecutor {
        let (client, auth) = crate::http_util::test_auth();
        QwenExecutor::builder().http(client).auth(auth).build()
    }

    #[test]
    fn test_supported_models_non_empty() {
        let ex = make_executor();
        assert!(!ex.supported_models().is_empty());
    }
}
