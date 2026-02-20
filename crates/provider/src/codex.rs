//! Codex (`OpenAI`) executor — `OpenAI` Chat Completions API.
//!
//! `OpenAI` format is passthrough (no translation needed).
//! Auth: `Authorization: Bearer {key_or_token}`.
use crate::registry;
use async_trait::async_trait;
use byok_auth::AuthManager;
use byok_types::{
    ByokError, ProviderId,
    traits::{ByteStream, ProviderExecutor, ProviderResponse, Result},
};
use futures_util::StreamExt as _;
use rquest::Client;
use serde_json::Value;
use std::sync::Arc;

/// `OpenAI` Chat Completions API endpoint.
const API_URL: &str = "https://api.openai.com/v1/chat/completions";

/// Executor for the `OpenAI` (Codex) API.
pub struct CodexExecutor {
    http: Client,
    api_key: Option<String>,
    auth: Arc<AuthManager>,
}

impl CodexExecutor {
    /// Creates a new Codex executor with an optional API key and auth manager.
    pub fn new(api_key: Option<String>, auth: Arc<AuthManager>) -> Self {
        Self {
            http: Client::new(),
            api_key,
            auth,
        }
    }

    /// Returns the bearer token: API key if present, otherwise fetches an OAuth token.
    async fn bearer_token(&self) -> Result<String> {
        if let Some(key) = &self.api_key {
            return Ok(key.clone());
        }
        let token = self.auth.get_token(&ProviderId::Codex).await?;
        Ok(token.access_token)
    }
}

#[async_trait]
impl ProviderExecutor for CodexExecutor {
    async fn chat_completion(&self, request: Value, stream: bool) -> Result<ProviderResponse> {
        let mut body = request;
        body["stream"] = Value::Bool(stream);

        let token = self.bearer_token().await?;
        let resp = self
            .http
            .post(API_URL)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ByokError::Http(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ByokError::Http(format!("OpenAI API {status}: {text}")));
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
        registry::codex_models()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byok_store::InMemoryTokenStore;

    fn make_executor() -> CodexExecutor {
        let store = Arc::new(InMemoryTokenStore::new());
        let auth = Arc::new(AuthManager::new(store));
        CodexExecutor::new(None, auth)
    }

    #[test]
    fn test_supported_models_non_empty() {
        let ex = make_executor();
        let models = ex.supported_models();
        assert!(!models.is_empty());
    }

    #[test]
    fn test_supported_models_contains_gpt4o() {
        let ex = make_executor();
        assert!(ex.supported_models().iter().any(|m| m == "gpt-4o"));
    }
}
