//! Qwen executor — Alibaba Qwen (Tongyi Qianwen) API.
//!
//! Uses Qwen's OpenAI-compatible endpoint with direct passthrough.
//! Auth: `Authorization: Bearer {token}` for both OAuth and API key.
use crate::http_util::ProviderHttp;
use crate::registry;
use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_types::{
    ChatRequest, ProviderId,
    traits::{ProviderExecutor, ProviderResponse, Result},
};
use rquest::Client;
use std::sync::Arc;

/// Qwen OpenAI-compatible endpoint
const API_URL: &str = "https://portal.qwen.ai/v1/chat/completions";

/// Executor for the Alibaba Qwen API.
pub struct QwenExecutor {
    ph: ProviderHttp,
    api_key: Option<String>,
    auth: Arc<AuthManager>,
}

impl QwenExecutor {
    /// Creates a new Qwen executor with an optional API key and auth manager.
    pub fn new(http: Client, api_key: Option<String>, auth: Arc<AuthManager>) -> Self {
        Self {
            ph: ProviderHttp::new(http),
            api_key,
            auth,
        }
    }

    /// Returns the Bearer token: API key if configured, otherwise OAuth access token.
    async fn bearer_token(&self) -> Result<String> {
        if let Some(key) = &self.api_key {
            return Ok(key.clone());
        }
        let token = self.auth.get_token(&ProviderId::Qwen).await?;
        Ok(token.access_token)
    }
}

#[async_trait]
impl ProviderExecutor for QwenExecutor {
    async fn chat_completion(&self, request: ChatRequest) -> Result<ProviderResponse> {
        let stream = request.stream;
        let mut body = request.into_body();

        if stream {
            body["stream_options"] = serde_json::json!({ "include_usage": true });
        }

        let token = self.bearer_token().await?;

        let mut builder = self
            .ph
            .client()
            .post(API_URL)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .header("user-agent", "QwenCode/0.10.3 (darwin; arm64)")
            .header("x-dashscope-useragent", "QwenCode/0.10.3 (darwin; arm64)")
            .header("x-dashscope-authtype", "qwen-oauth")
            .header("x-stainless-runtime-version", "v22.17.0")
            .header("x-stainless-lang", "js")
            .header("x-stainless-arch", "arm64")
            .header("x-stainless-package-version", "5.11.0")
            .header("x-dashscope-cachecontrol", "enable")
            .header("x-stainless-retry-count", "0")
            .header("x-stainless-os", "MacOS")
            .header("x-stainless-runtime", "node");

        if stream {
            builder = builder.header("accept", "text/event-stream");
        } else {
            builder = builder.header("accept", "application/json");
        }

        self.ph.send_passthrough(builder.json(&body), stream).await
    }

    fn supported_models(&self) -> Vec<String> {
        registry::qwen_models()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byokey_store::InMemoryTokenStore;

    fn make_executor() -> QwenExecutor {
        let store = Arc::new(InMemoryTokenStore::new());
        let auth = Arc::new(AuthManager::new(store));
        QwenExecutor::new(Client::new(), None, auth)
    }

    #[test]
    fn test_supported_models_non_empty() {
        let ex = make_executor();
        assert!(!ex.supported_models().is_empty());
    }
}
