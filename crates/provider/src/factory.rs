//! Factory executor — backend-only provider routing through Factory.ai proxy.
//!
//! Factory is a backend provider: it does not own any models directly.
//! Configure other providers with `backend: factory` to route through it.
//!
//! Auth: device code flow → `WorkOS` token → org-scoped token (refresh managed by `AuthManager`).
//! Format: `OpenAI` passthrough (no translation needed).

use crate::http_util::ProviderHttp;
use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_types::{
    ChatRequest, ProviderId,
    traits::{ProviderExecutor, ProviderResponse, Result},
};
use rquest::Client;
use std::sync::Arc;

/// Factory LLM proxy base URL (OpenAI-compatible endpoint).
const API_BASE_URL: &str = "https://api.factory.ai/api/llm/o";

/// User-Agent header matching the Factory CLI.
const USER_AGENT: &str = "factory-cli/0.62.1";

/// Executor for the Factory.ai API.
pub struct FactoryExecutor {
    ph: ProviderHttp,
    api_key: Option<String>,
    auth: Arc<AuthManager>,
}

impl FactoryExecutor {
    /// Creates a new Factory executor with an optional API key and auth manager.
    pub fn new(http: Client, api_key: Option<String>, auth: Arc<AuthManager>) -> Self {
        Self {
            ph: ProviderHttp::new(http),
            api_key,
            auth,
        }
    }

    /// Obtains the Bearer token for Factory API requests.
    ///
    /// Uses the configured `api_key` if present, otherwise retrieves the
    /// OAuth token from the auth manager (with automatic refresh).
    async fn bearer_token(&self) -> Result<String> {
        if let Some(key) = &self.api_key {
            return Ok(key.clone());
        }
        let token = self.auth.get_token(&ProviderId::Factory).await?;
        Ok(token.access_token)
    }
}

#[async_trait]
impl ProviderExecutor for FactoryExecutor {
    async fn chat_completion(&self, request: ChatRequest) -> Result<ProviderResponse> {
        let stream = request.stream;
        let body = request.into_body();

        let token = self.bearer_token().await?;

        let builder = self
            .ph
            .client()
            .post(format!("{API_BASE_URL}/v1/chat/completions"))
            .header("authorization", format!("Bearer {token}"))
            .header("user-agent", USER_AGENT)
            .header("x-api-provider", "openai")
            .header("x-session-id", uuid::Uuid::new_v4().to_string())
            .header("x-assistant-message-id", uuid::Uuid::new_v4().to_string())
            .header("content-type", "application/json")
            .json(&body);

        self.ph.send_passthrough(builder, stream).await
    }

    fn supported_models(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byokey_store::InMemoryTokenStore;

    fn make_executor() -> FactoryExecutor {
        let store = Arc::new(InMemoryTokenStore::new());
        let auth = Arc::new(AuthManager::new(store, rquest::Client::new()));
        FactoryExecutor::new(Client::new(), None, auth)
    }

    #[test]
    fn test_supported_models_empty() {
        let ex = make_executor();
        assert!(ex.supported_models().is_empty());
    }
}
