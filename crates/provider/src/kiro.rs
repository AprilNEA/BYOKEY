//! Kiro executor — Anthropic-compatible API served by Kiro.
//!
//! Kiro exposes an Anthropic Messages API at its own endpoint.
//! Format: `OpenAI` -> Anthropic (translate), Anthropic -> `OpenAI` (translate).
use crate::registry;
use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_translate::{ClaudeToOpenAI, OpenAIToClaude};
use byokey_types::{
    ByokError, ProviderId,
    traits::{
        ByteStream, ProviderExecutor, ProviderResponse, RequestTranslator, ResponseTranslator,
        Result,
    },
};
use futures_util::StreamExt as _;
use rquest::Client;
use serde_json::Value;
use std::sync::Arc;

/// Kiro Messages API endpoint.
const KIRO_API_URL: &str = "https://api.kiro.dev/v1/messages";

/// Required Anthropic API version header value.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Executor for the Kiro API (Anthropic-compatible).
pub struct KiroExecutor {
    http: Client,
    api_key: Option<String>,
    auth: Arc<AuthManager>,
}

impl KiroExecutor {
    /// Creates a new Kiro executor with an optional API key and auth manager.
    pub fn new(http: Client, api_key: Option<String>, auth: Arc<AuthManager>) -> Self {
        Self {
            http,
            api_key,
            auth,
        }
    }

    /// Returns the bearer token: API key if present, otherwise fetches an OAuth token.
    async fn bearer_token(&self) -> Result<String> {
        if let Some(key) = &self.api_key {
            return Ok(key.clone());
        }
        let token = self.auth.get_token(&ProviderId::Kiro).await?;
        Ok(token.access_token)
    }
}

#[async_trait]
impl ProviderExecutor for KiroExecutor {
    async fn chat_completion(&self, request: Value, stream: bool) -> Result<ProviderResponse> {
        let mut body = OpenAIToClaude.translate_request(request)?;
        body["stream"] = Value::Bool(stream);

        let token = self.bearer_token().await?;
        let resp = self
            .http
            .post(KIRO_API_URL)
            .header("authorization", format!("Bearer {token}"))
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ByokError::Upstream {
                status: status.as_u16(),
                body: text,
            });
        }

        if stream {
            let byte_stream: ByteStream =
                Box::pin(resp.bytes_stream().map(|r| r.map_err(ByokError::from)));
            Ok(ProviderResponse::Stream(byte_stream))
        } else {
            let json: Value = resp.json().await?;
            let translated = ClaudeToOpenAI.translate_response(json)?;
            Ok(ProviderResponse::Complete(translated))
        }
    }

    fn supported_models(&self) -> Vec<String> {
        registry::kiro_models()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byokey_store::InMemoryTokenStore;

    fn make_executor() -> KiroExecutor {
        let store = Arc::new(InMemoryTokenStore::new());
        let auth = Arc::new(AuthManager::new(store));
        KiroExecutor::new(Client::new(), None, auth)
    }

    #[test]
    fn test_supported_models_non_empty() {
        let ex = make_executor();
        assert!(!ex.supported_models().is_empty());
    }
}
