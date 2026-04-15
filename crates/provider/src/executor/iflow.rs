//! iFlow executor — Z.ai / GLM OpenAI-compatible API.
//!
//! Uses iFlow's OpenAI-compatible chat completions endpoint via `aigw::openai_compat`.
//! Auth: `Authorization: Bearer {token}` (API key or OAuth).
//! Signing: every request carries HMAC-SHA256 headers (`session-id`,
//! `x-iflow-timestamp`, `x-iflow-signature`) derived from the API key,
//! a per-request UUID session id, and a millisecond timestamp.  These are
//! dynamic, so they are appended to the builder *after* aigw translates the
//! request rather than going into `default_headers`.

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
use hmac::{Hmac, Mac};
use rquest::Client;
use secrecy::SecretString;
use sha2::Sha256;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Default iFlow API base URL (`/v1` suffix; aigw appends `/chat/completions`).
const DEFAULT_BASE_URL: &str = "https://apis.iflow.cn/v1";

/// Executor for the iFlow (Z.ai / GLM) API.
pub struct IFlowExecutor {
    ph: ProviderHttp,
    api_key: Option<String>,
    base_url: String,
    auth: Arc<AuthManager>,
}

#[bon::bon]
impl IFlowExecutor {
    /// Creates a new iFlow executor.
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
            ph = ph.with_ratelimit(store, ProviderId::IFlow);
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

    /// Returns the Bearer token: API key if configured, otherwise OAuth access token.
    async fn bearer_token(&self) -> Result<String> {
        crate::http_util::resolve_bearer_token(
            self.api_key.as_deref(),
            &self.auth,
            &ProviderId::IFlow,
        )
        .await
    }

    /// Builds an [`OpenAICompatProvider`] for a single request.
    ///
    /// iFlow uses a standard `user-agent` header which can go in `default_headers`.
    /// The HMAC signing headers are dynamic and are added post-translation.
    fn build_provider(&self, token: String) -> Result<OpenAICompatProvider> {
        let mut default_headers = BTreeMap::new();
        default_headers.insert("user-agent".to_owned(), "iFlow-Cli".to_owned());

        OpenAICompatProvider::new(OpenAICompatConfig {
            name: "iflow".to_owned(),
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

/// Compute HMAC-SHA256 signature for iFlow request authentication.
///
/// Payload format: `iFlow-Cli:{session_id}:{timestamp}`
fn create_signature(api_key: &str, session_id: &str, timestamp: u64) -> String {
    let payload = format!("iFlow-Cli:{session_id}:{timestamp}");
    let mut mac =
        <Hmac<Sha256>>::new_from_slice(api_key.as_bytes()).expect("HMAC can take key of any size");
    mac.update(payload.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

#[async_trait]
impl ProviderExecutor for IFlowExecutor {
    async fn chat_completion(&self, request: ChatRequest) -> Result<ProviderResponse> {
        let stream = request.stream;

        let token = self.bearer_token().await?;
        let provider = self.build_provider(token.clone())?;
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

        // Build per-request HMAC-SHA256 signing headers (dynamic — cannot be in default_headers).
        let session_id = format!("session-{}", uuid::Uuid::new_v4());
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX);
        let signature = create_signature(&token, &session_id, timestamp);

        // Build rquest from TranslatedRequest URL/headers + body, then append signing headers.
        let mut builder = self.ph.client().post(&translated.url);
        for (name, value) in &translated.headers {
            if let Ok(v) = value.to_str() {
                builder = builder.header(name.as_str(), v);
            }
        }
        let builder = builder
            .header("accept-encoding", "identity")
            .header("session-id", &session_id)
            .header("x-iflow-timestamp", timestamp.to_string())
            .header("x-iflow-signature", &signature)
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
        registry::models_for_provider(&ProviderId::IFlow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_executor() -> IFlowExecutor {
        let (client, auth) = crate::http_util::test_auth();
        IFlowExecutor::builder().http(client).auth(auth).build()
    }

    #[test]
    fn test_supported_models_non_empty() {
        let ex = make_executor();
        assert!(!ex.supported_models().is_empty());
    }

    #[test]
    fn test_create_signature_deterministic() {
        let sig1 = create_signature("key123", "session-abc", 1_700_000_000);
        let sig2 = create_signature("key123", "session-abc", 1_700_000_000);
        assert_eq!(sig1, sig2);
        assert!(!sig1.is_empty());
        // HMAC-SHA256 produces 64 hex chars
        assert_eq!(sig1.len(), 64);
    }

    #[test]
    fn test_create_signature_differs_with_different_key() {
        let sig1 = create_signature("key1", "session-abc", 1_700_000_000);
        let sig2 = create_signature("key2", "session-abc", 1_700_000_000);
        assert_ne!(sig1, sig2);
    }
}
