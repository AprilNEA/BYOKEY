//! iFlow executor — Z.ai / GLM OpenAI-compatible API.
//!
//! Auth: Bearer API key (obtained via OAuth + userInfo exchange).
//! Format: OpenAI passthrough (no translation needed).

use crate::registry;
use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_types::{
    ByokError, ProviderId,
    traits::{ByteStream, ProviderExecutor, ProviderResponse, Result},
};
use futures_util::StreamExt as _;
use hmac::{Hmac, Mac};
use rquest::Client;
use serde_json::Value;
use sha2::Sha256;
use std::sync::Arc;

/// iFlow OpenAI-compatible chat completions endpoint.
const API_URL: &str = "https://apis.iflow.cn/v1/chat/completions";

/// Executor for the iFlow (Z.ai / GLM) API.
pub struct IFlowExecutor {
    http: Client,
    api_key: Option<String>,
    auth: Arc<AuthManager>,
}

impl IFlowExecutor {
    /// Creates a new iFlow executor with an optional API key and auth manager.
    pub fn new(api_key: Option<String>, auth: Arc<AuthManager>) -> Self {
        Self {
            http: Client::new(),
            api_key,
            auth,
        }
    }

    /// Resolves the API key: config-provided key first, otherwise from the auth store.
    async fn resolve_api_key(&self) -> Result<String> {
        if let Some(key) = &self.api_key {
            return Ok(key.clone());
        }
        let token = self.auth.get_token(&ProviderId::IFlow).await?;
        Ok(token.access_token)
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
    async fn chat_completion(&self, request: Value, stream: bool) -> Result<ProviderResponse> {
        let mut body = request;
        body["stream"] = Value::Bool(stream);

        let api_key = self.resolve_api_key().await?;

        let session_id = format!("session-{}", uuid::Uuid::new_v4());
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let signature = create_signature(&api_key, &session_id, timestamp);

        let accept = if stream {
            "text/event-stream"
        } else {
            "application/json"
        };

        let resp = self
            .http
            .post(API_URL)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {api_key}"))
            .header("user-agent", "iFlow-Cli")
            .header("session-id", &session_id)
            .header("x-iflow-timestamp", timestamp.to_string())
            .header("x-iflow-signature", &signature)
            .header("accept", accept)
            .json(&body)
            .send()
            .await
            .map_err(|e| ByokError::Http(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ByokError::Http(format!("iFlow API {status}: {text}")));
        }

        if stream {
            let byte_stream: ByteStream = Box::pin(
                resp.bytes_stream()
                    .map(|r| r.map_err(|e| ByokError::Http(e.to_string()))),
            );
            Ok(ProviderResponse::Stream(byte_stream))
        } else {
            let json: Value = resp
                .json()
                .await
                .map_err(|e| ByokError::Http(e.to_string()))?;
            Ok(ProviderResponse::Complete(json))
        }
    }

    fn supported_models(&self) -> Vec<String> {
        registry::iflow_models()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byokey_store::InMemoryTokenStore;

    fn make_executor() -> IFlowExecutor {
        let store = Arc::new(InMemoryTokenStore::new());
        let auth = Arc::new(AuthManager::new(store));
        IFlowExecutor::new(None, auth)
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
