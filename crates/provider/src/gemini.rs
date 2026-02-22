//! Gemini executor — Google Generative Language API.
//!
//! Uses Gemini's OpenAI-compatible endpoint for simplicity.
//! Auth: `Authorization: Bearer {token}` for OAuth, `x-goog-api-key` for API key.
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

/// Gemini OpenAI-compatible endpoint
const API_URL: &str = "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions";

/// Executor for the Google Gemini API.
pub struct GeminiExecutor {
    http: Client,
    api_key: Option<String>,
    auth: Arc<AuthManager>,
}

impl GeminiExecutor {
    /// Creates a new Gemini executor with an optional API key and auth manager.
    pub fn new(api_key: Option<String>, auth: Arc<AuthManager>) -> Self {
        Self {
            http: Client::new(),
            api_key,
            auth,
        }
    }

    /// Returns the auth header: `x-goog-api-key` for API keys, `Authorization: Bearer` for OAuth.
    async fn auth_header(&self) -> Result<(&'static str, String)> {
        if let Some(key) = &self.api_key {
            return Ok(("x-goog-api-key", key.clone()));
        }
        let token = self.auth.get_token(&ProviderId::Gemini).await?;
        Ok(("authorization", format!("Bearer {}", token.access_token)))
    }
}

#[async_trait]
impl ProviderExecutor for GeminiExecutor {
    async fn chat_completion(&self, request: Value, stream: bool) -> Result<ProviderResponse> {
        let mut body = request;
        body["stream"] = Value::Bool(stream);

        let (header_name, header_value) = self.auth_header().await?;

        let resp = self
            .http
            .post(API_URL)
            .header(header_name, header_value)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(ByokError::Http(format!("Gemini API {status}: {text}")));
        }

        if stream {
            let byte_stream: ByteStream = Box::pin(
                resp.bytes_stream()
                    .map(|r| r.map_err(ByokError::from)),
            );
            Ok(ProviderResponse::Stream(byte_stream))
        } else {
            let json: Value = resp.json().await?;
            Ok(ProviderResponse::Complete(json))
        }
    }

    fn supported_models(&self) -> Vec<String> {
        registry::gemini_models()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byokey_store::InMemoryTokenStore;

    fn make_executor() -> GeminiExecutor {
        let store = Arc::new(InMemoryTokenStore::new());
        let auth = Arc::new(AuthManager::new(store));
        GeminiExecutor::new(None, auth)
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
}
