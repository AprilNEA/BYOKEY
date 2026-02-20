//! GitHub Copilot executor — OpenAI-compatible API.
//!
//! Auth: device code flow -> Bearer token.
//! Format: `OpenAI` passthrough.
use crate::registry;
use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_types::{
    ByokError, ProviderId,
    traits::{ByteStream, ProviderExecutor, ProviderResponse, Result},
};
use futures_util::StreamExt as _;
use rquest::Client;
use serde_json::Value;
use std::sync::Arc;

/// GitHub Copilot Chat Completions API endpoint.
const API_URL: &str = "https://api.githubcopilot.com/chat/completions";

/// Executor for the GitHub Copilot API.
pub struct CopilotExecutor {
    http: Client,
    api_key: Option<String>,
    auth: Arc<AuthManager>,
}

impl CopilotExecutor {
    /// Creates a new Copilot executor with an optional API key and auth manager.
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
        let token = self.auth.get_token(&ProviderId::Copilot).await?;
        Ok(token.access_token)
    }
}

#[async_trait]
impl ProviderExecutor for CopilotExecutor {
    async fn chat_completion(&self, request: Value, stream: bool) -> Result<ProviderResponse> {
        let mut body = request;
        body["stream"] = Value::Bool(stream);

        let token = self.bearer_token().await?;
        let resp = self
            .http
            .post(API_URL)
            .header("authorization", format!("Bearer {token}"))
            .header("editor-version", "vscode/1.95.0")
            .header("editor-plugin-version", "copilot-chat/0.22.0")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ByokError::Http(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ByokError::Http(format!("Copilot API {status}: {text}")));
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
        registry::copilot_models()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byokey_store::InMemoryTokenStore;

    fn make_executor() -> CopilotExecutor {
        let store = Arc::new(InMemoryTokenStore::new());
        let auth = Arc::new(AuthManager::new(store));
        CopilotExecutor::new(None, auth)
    }

    #[test]
    fn test_supported_models_non_empty() {
        let ex = make_executor();
        assert!(!ex.supported_models().is_empty());
    }
}
