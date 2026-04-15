//! Kimi executor — Moonshot AI (Kimi) OpenAI-compatible API.
//!
//! Uses Kimi's OpenAI-compatible chat completions endpoint via `aigw::openai_compat`.
//! Auth: `Authorization: Bearer {token}` for both OAuth and API key.
//! Model names are prefixed with `kimi-` locally and stripped before upstream dispatch.
//!
//! Kimi uses a non-standard path (`/coding/v1/chat/completions`). This is handled
//! by setting `base_url` to `https://api.kimi.com/coding/v1`; aigw appends
//! `/chat/completions` to produce the correct full URL.

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
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Default Kimi API base URL (includes `/coding/v1`; aigw appends `/chat/completions`).
const DEFAULT_BASE_URL: &str = "https://api.kimi.com/coding/v1";

/// Executor for the Moonshot AI (Kimi) API.
pub struct KimiExecutor {
    ph: ProviderHttp,
    api_key: Option<String>,
    base_url: String,
    auth: Arc<AuthManager>,
    device_id: String,
}

#[bon::bon]
impl KimiExecutor {
    /// Creates a new Kimi executor.
    #[builder]
    #[allow(clippy::needless_pass_by_value)]
    pub fn new(
        http: rquest::Client,
        auth: Arc<AuthManager>,
        api_key: Option<String>,
        base_url: Option<String>,
        ratelimit: Option<Arc<RateLimitStore>>,
    ) -> Self {
        let mut ph = ProviderHttp::new(http);
        if let Some(store) = ratelimit {
            ph = ph.with_ratelimit(store, ProviderId::Kimi);
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
            device_id: byokey_auth::provider::kimi::device_id(),
        }
    }

    /// Returns the Bearer token: API key if configured, otherwise OAuth access token.
    async fn bearer_token(&self) -> Result<String> {
        crate::http_util::resolve_bearer_token(
            self.api_key.as_deref(),
            &self.auth,
            &ProviderId::Kimi,
        )
        .await
    }

    /// Builds an [`OpenAICompatProvider`] for a single request.
    ///
    /// Static Kimi-specific headers are placed in `default_headers` so aigw
    /// includes them in every request it builds.
    fn build_provider(&self, token: String) -> Result<OpenAICompatProvider> {
        let mut default_headers = BTreeMap::new();
        default_headers.insert("user-agent".to_owned(), "KimiCLI/1.10.6".to_owned());
        default_headers.insert("x-msh-platform".to_owned(), "kimi_cli".to_owned());
        default_headers.insert("x-msh-version".to_owned(), "1.10.6".to_owned());
        default_headers.insert(
            "x-msh-device-name".to_owned(),
            byokey_auth::provider::kimi::device_name().clone(),
        );
        default_headers.insert(
            "x-msh-device-model".to_owned(),
            byokey_auth::provider::kimi::DEVICE_MODEL.to_owned(),
        );
        default_headers.insert("x-msh-device-id".to_owned(), self.device_id.clone());

        OpenAICompatProvider::new(OpenAICompatConfig {
            name: "kimi".to_owned(),
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

/// Strip the `kimi-` prefix from a model name for the upstream API.
fn strip_kimi_prefix(model: &str) -> &str {
    model.strip_prefix("kimi-").unwrap_or(model)
}

#[async_trait]
impl ProviderExecutor for KimiExecutor {
    async fn chat_completion(&self, request: ChatRequest) -> Result<ProviderResponse> {
        let stream = request.stream;
        let mut body = request.into_body();

        // Strip kimi- prefix for upstream API before translation.
        if let Some(model) = body.get("model").and_then(Value::as_str).map(String::from) {
            body["model"] = Value::String(strip_kimi_prefix(&model).to_string());
        }

        let token = self.bearer_token().await?;
        let provider = self.build_provider(token)?;
        let translator = OpenAICompatRequestTranslator::new(&provider)
            .map_err(|e| ByokError::Config(e.to_string()))?;

        // Translate byokey ChatRequest → aigw ChatRequest → OpenAI body.
        let aigw_request: aigw_core::model::ChatRequest =
            serde_json::from_value(body).map_err(|e| ByokError::Translation(e.to_string()))?;

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
        registry::models_for_provider(&ProviderId::Kimi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_executor() -> KimiExecutor {
        let (client, auth) = crate::http_util::test_auth();
        KimiExecutor::builder().http(client).auth(auth).build()
    }

    #[test]
    fn test_supported_models_non_empty() {
        let ex = make_executor();
        assert!(!ex.supported_models().is_empty());
    }

    #[test]
    fn test_strip_kimi_prefix() {
        assert_eq!(strip_kimi_prefix("kimi-k2-0711"), "k2-0711");
        assert_eq!(strip_kimi_prefix("kimi-moonshot-v1"), "moonshot-v1");
        assert_eq!(strip_kimi_prefix("k2-0711"), "k2-0711");
    }
}
