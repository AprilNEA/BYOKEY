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
    ChatRequest, ProviderId, RateLimitStore,
    traits::{ProviderExecutor, ProviderResponse, Result},
};
use rquest::Client;
use serde_json::Value;
use std::sync::Arc;

/// Default Kimi API base URL.
const DEFAULT_BASE_URL: &str = "https://api.kimi.com";
/// Chat completions API path.
const API_PATH: &str = "/coding/v1/chat/completions";

/// Executor for the Moonshot AI (Kimi) API.
pub struct KimiExecutor {
    ph: ProviderHttp,
    api_key: Option<String>,
    api_url: String,
    auth: Arc<AuthManager>,
    device_id: String,
}

#[bon::bon]
impl KimiExecutor {
    /// Creates a new Kimi executor.
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
            ph = ph.with_ratelimit(store, ProviderId::Kimi);
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
            device_id: byokey_auth::provider::kimi::device_id(),
        }
    }

    /// Returns the Bearer token: API key if configured, otherwise OAuth access token.
    async fn bearer_token(&self) -> Result<String> {
        crate::http_util::resolve_bearer_token(
            self.api_key.as_deref(),
            &self.auth,
            &ProviderId::Kimi,
        )
        .await
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

        crate::http_util::ensure_stream_options(&mut body, stream);

        // Strip kimi- prefix for upstream API
        if let Some(model) = body.get("model").and_then(Value::as_str).map(String::from) {
            body["model"] = Value::String(strip_kimi_prefix(&model).to_string());
        }

        let token = self.bearer_token().await?;
        let accept = crate::http_util::accept_for_stream(stream);

        let builder = self
            .ph
            .client()
            .post(&self.api_url)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .header("user-agent", "KimiCLI/1.10.6")
            .header("x-msh-platform", "kimi_cli")
            .header("x-msh-version", "1.10.6")
            .header(
                "x-msh-device-name",
                byokey_auth::provider::kimi::device_name(),
            )
            .header(
                "x-msh-device-model",
                byokey_auth::provider::kimi::DEVICE_MODEL,
            )
            .header("x-msh-device-id", &self.device_id)
            .header("accept", accept)
            .json(&body);

        self.ph.send_passthrough(builder, stream).await
    }

    fn supported_models(&self) -> Vec<String> {
        registry::models_for_provider(&ProviderId::Kimi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_executor() -> KimiExecutor {
        let (client, auth) = crate::http_util::test_auth();
        KimiExecutor::builder().http(client).auth(auth).build()
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
