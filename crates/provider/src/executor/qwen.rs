//! Qwen executor — Alibaba Qwen (Tongyi Qianwen) API.
//!
//! Uses Qwen's OpenAI-compatible endpoint with direct passthrough.
//! Auth: `Authorization: Bearer {token}` for both OAuth and API key.
use crate::http_util::ProviderHttp;
use crate::registry;
use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_types::{
    ChatRequest, ProviderId, RateLimitStore,
    traits::{ProviderExecutor, ProviderResponse, Result},
};
use rquest::Client;
use std::sync::Arc;

/// Default Qwen API base URL.
const DEFAULT_BASE_URL: &str = "https://portal.qwen.ai";
/// Chat completions API path.
const API_PATH: &str = "/v1/chat/completions";

/// Executor for the Alibaba Qwen API.
pub struct QwenExecutor {
    ph: ProviderHttp,
    api_key: Option<String>,
    api_url: String,
    auth: Arc<AuthManager>,
}

#[bon::bon]
impl QwenExecutor {
    /// Creates a new Qwen executor.
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
            ph = ph.with_ratelimit(store, ProviderId::Qwen);
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
        }
    }

    /// Returns the Bearer token: API key if configured, otherwise OAuth access token.
    async fn bearer_token(&self) -> Result<String> {
        crate::http_util::resolve_bearer_token(
            self.api_key.as_deref(),
            &self.auth,
            &ProviderId::Qwen,
        )
        .await
    }
}

#[async_trait]
impl ProviderExecutor for QwenExecutor {
    async fn chat_completion(&self, request: ChatRequest) -> Result<ProviderResponse> {
        let stream = request.stream;
        let mut body = request.into_body();

        crate::http_util::ensure_stream_options(&mut body, stream);

        let token = self.bearer_token().await?;
        let accept = crate::http_util::accept_for_stream(stream);

        let builder = self
            .ph
            .client()
            .post(&self.api_url)
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
            .header("x-stainless-runtime", "node")
            .header("accept", accept);

        self.ph.send_passthrough(builder.json(&body), stream).await
    }

    fn supported_models(&self) -> Vec<String> {
        registry::models_for_provider(&ProviderId::Qwen)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_executor() -> QwenExecutor {
        let (client, auth) = crate::http_util::test_auth();
        QwenExecutor::builder().http(client).auth(auth).build()
    }

    #[test]
    fn test_supported_models_non_empty() {
        let ex = make_executor();
        assert!(!ex.supported_models().is_empty());
    }
}
