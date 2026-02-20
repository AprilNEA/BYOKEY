//! Claude executor — Anthropic Messages API.
//!
//! Auth: `x-api-key` for raw API keys, `Authorization: Bearer` for OAuth tokens.
//! Format: `OpenAI` -> Anthropic (translate), Anthropic -> `OpenAI` (translate).
use crate::registry;
use async_trait::async_trait;
use byok_auth::AuthManager;
use byok_translate::{ClaudeToOpenAI, OpenAIToClaude};
use byok_types::{
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

/// Anthropic Messages API endpoint.
const API_URL: &str = "https://api.anthropic.com/v1/messages";

/// Required Anthropic API version header value.
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Authentication mode for the Claude API.
enum AuthMode {
    /// Raw API key sent via `x-api-key` header.
    ApiKey(String),
    /// OAuth token sent via `Authorization: Bearer` header.
    Bearer(String),
}

/// Executor for the Anthropic Claude API.
pub struct ClaudeExecutor {
    http: Client,
    api_key: Option<String>,
    auth: Arc<AuthManager>,
}

impl ClaudeExecutor {
    /// Creates a new Claude executor with an optional API key and auth manager.
    pub fn new(api_key: Option<String>, auth: Arc<AuthManager>) -> Self {
        Self {
            http: Client::new(),
            api_key,
            auth,
        }
    }

    /// Resolves the authentication mode: API key if present, otherwise OAuth token.
    async fn get_auth(&self) -> Result<AuthMode> {
        if let Some(key) = &self.api_key {
            return Ok(AuthMode::ApiKey(key.clone()));
        }
        let token = self.auth.get_token(&ProviderId::Claude).await?;
        Ok(AuthMode::Bearer(token.access_token))
    }
}

#[async_trait]
impl ProviderExecutor for ClaudeExecutor {
    async fn chat_completion(&self, request: Value, stream: bool) -> Result<ProviderResponse> {
        let mut body = OpenAIToClaude.translate_request(request)?;
        body["stream"] = Value::Bool(stream);

        let auth = self.get_auth().await?;

        let builder = self
            .http
            .post(API_URL)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json");

        let builder = match &auth {
            AuthMode::ApiKey(key) => builder.header("x-api-key", key.as_str()),
            AuthMode::Bearer(tok) => builder.header("authorization", format!("Bearer {tok}")),
        };

        let resp = builder
            .json(&body)
            .send()
            .await
            .map_err(|e| ByokError::Http(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ByokError::Http(format!("Claude API {status}: {text}")));
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
            let translated = ClaudeToOpenAI.translate_response(json)?;
            Ok(ProviderResponse::Complete(translated))
        }
    }

    fn supported_models(&self) -> Vec<String> {
        registry::claude_models()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byok_store::InMemoryTokenStore;

    fn make_executor() -> ClaudeExecutor {
        let store = Arc::new(InMemoryTokenStore::new());
        let auth = Arc::new(AuthManager::new(store));
        ClaudeExecutor::new(None, auth)
    }

    #[test]
    fn test_supported_models_non_empty() {
        let ex = make_executor();
        let models = ex.supported_models();
        assert!(!models.is_empty());
        assert!(models.iter().all(|m| m.starts_with("claude-")));
    }

    #[test]
    fn test_supported_models_with_api_key() {
        let store = Arc::new(InMemoryTokenStore::new());
        let auth = Arc::new(AuthManager::new(store));
        let ex = ClaudeExecutor::new(Some("sk-ant-test".into()), auth);
        assert!(!ex.supported_models().is_empty());
    }
}
