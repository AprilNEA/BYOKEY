//! Kimi executor — Moonshot AI (Kimi) OpenAI-compatible API.
//!
//! Uses Kimi's OpenAI-compatible chat completions endpoint with direct passthrough.
//! Auth: `Authorization: Bearer {token}` for both OAuth and API key.
//! Model names are prefixed with `kimi-` locally and stripped before upstream dispatch.

use crate::http_util::ProviderHttp;
use crate::registry;
use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_types::{
    ChatRequest, ProviderId,
    traits::{ProviderExecutor, ProviderResponse, Result},
};
use rquest::Client;
use serde_json::Value;
use std::sync::Arc;

/// Kimi OpenAI-compatible chat completions endpoint.
const API_URL: &str = "https://api.kimi.com/coding/v1/chat/completions";

/// Executor for the Moonshot AI (Kimi) API.
pub struct KimiExecutor {
    ph: ProviderHttp,
    api_key: Option<String>,
    auth: Arc<AuthManager>,
    device_id: String,
}

impl KimiExecutor {
    /// Creates a new Kimi executor with an optional API key and auth manager.
    pub fn new(http: Client, api_key: Option<String>, auth: Arc<AuthManager>) -> Self {
        Self {
            ph: ProviderHttp::new(http),
            api_key,
            auth,
            device_id: byokey_auth::kimi::device_id(),
        }
    }

    /// Returns the Bearer token: API key if configured, otherwise OAuth access token.
    async fn bearer_token(&self) -> Result<String> {
        if let Some(key) = &self.api_key {
            return Ok(key.clone());
        }
        let token = self.auth.get_token(&ProviderId::Kimi).await?;
        Ok(token.access_token)
    }
}

/// Strip the `kimi-` prefix from a model name for the upstream API.
fn strip_kimi_prefix(model: &str) -> &str {
    model.strip_prefix("kimi-").unwrap_or(model)
}

#[async_trait]
impl ProviderExecutor for KimiExecutor {
    async fn chat_completion(&self, request: ChatRequest) -> Result<ProviderResponse> {
        let stream = request.stream;
        let mut body = request.into_body();

        if stream {
            body["stream_options"] = serde_json::json!({ "include_usage": true });
        }

        // Strip kimi- prefix for upstream API
        if let Some(model) = body.get("model").and_then(Value::as_str).map(String::from) {
            body["model"] = Value::String(strip_kimi_prefix(&model).to_string());
        }

        let token = self.bearer_token().await?;

        let accept = if stream {
            "text/event-stream"
        } else {
            "application/json"
        };

        let builder = self
            .ph
            .client()
            .post(API_URL)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .header("user-agent", "KimiCLI/1.10.6")
            .header("x-msh-platform", "kimi_cli")
            .header("x-msh-version", "1.10.6")
            .header("x-msh-device-name", byokey_auth::kimi::device_name())
            .header("x-msh-device-model", byokey_auth::kimi::DEVICE_MODEL)
            .header("x-msh-device-id", &self.device_id)
            .header("accept", accept)
            .json(&body);

        self.ph.send_passthrough(builder, stream).await
    }

    fn supported_models(&self) -> Vec<String> {
        registry::kimi_models()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byokey_store::InMemoryTokenStore;

    fn make_executor() -> KimiExecutor {
        let store = Arc::new(InMemoryTokenStore::new());
        let auth = Arc::new(AuthManager::new(store));
        KimiExecutor::new(Client::new(), None, auth)
    }

    #[test]
    fn test_supported_models_non_empty() {
        let ex = make_executor();
        assert!(!ex.supported_models().is_empty());
    }

    #[test]
    fn test_strip_kimi_prefix() {
        assert_eq!(strip_kimi_prefix("kimi-k2-0711"), "k2-0711");
        assert_eq!(strip_kimi_prefix("kimi-moonshot-v1"), "moonshot-v1");
        assert_eq!(strip_kimi_prefix("k2-0711"), "k2-0711");
    }
}
