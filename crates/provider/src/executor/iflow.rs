//! iFlow executor — Z.ai / GLM OpenAI-compatible API.
//!
//! Auth: Bearer API key (obtained via OAuth + userInfo exchange).
//! Format: `OpenAI` passthrough (no translation needed).

use crate::http_util::ProviderHttp;
use crate::registry;
use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_types::{
    ChatRequest, ProviderId, RateLimitStore,
    traits::{ProviderExecutor, ProviderResponse, Result},
};
use hmac::{Hmac, Mac};
use rquest::Client;
use sha2::Sha256;
use std::sync::Arc;

/// Default iFlow API base URL.
const DEFAULT_BASE_URL: &str = "https://apis.iflow.cn";
/// Chat completions API path.
const API_PATH: &str = "/v1/chat/completions";

/// Executor for the iFlow (Z.ai / GLM) API.
pub struct IFlowExecutor {
    ph: ProviderHttp,
    api_key: Option<String>,
    api_url: String,
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

    /// Resolves the API key: config-provided key first, otherwise from the auth store.
    async fn resolve_api_key(&self) -> Result<String> {
        crate::http_util::resolve_bearer_token(
            self.api_key.as_deref(),
            &self.auth,
            &ProviderId::IFlow,
        )
        .await
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
        let mut body = request.into_body();
        crate::http_util::ensure_stream_options(&mut body, stream);

        let api_key = self.resolve_api_key().await?;

        let session_id = format!("session-{}", uuid::Uuid::new_v4());
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX);
        let signature = create_signature(&api_key, &session_id, timestamp);
        let accept = crate::http_util::accept_for_stream(stream);

        let builder = self
            .ph
            .client()
            .post(&self.api_url)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {api_key}"))
            .header("user-agent", "iFlow-Cli")
            .header("session-id", &session_id)
            .header("x-iflow-timestamp", timestamp.to_string())
            .header("x-iflow-signature", &signature)
            .header("accept", accept)
            .json(&body);

        self.ph.send_passthrough(builder, stream).await
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
