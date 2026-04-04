//! Executor factory — creates boxed [`ProviderExecutor`] instances from
//! provider identifiers, model strings, and configuration.
//!
//! Supports API key passthrough, backend overrides, fallback wrapping, and
//! multi-key retry via [`RetryExecutor`](crate::retry::RetryExecutor).

use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_config::ProviderConfig;
use byokey_types::{
    ByokError, ChatRequest, ProviderId, RateLimitStore,
    traits::{ProviderExecutor, ProviderResponse, Result as ProviderResult},
};
use rquest::Client;
use std::collections::HashSet;
use std::hash::BuildHasher;
use std::sync::Arc;

use crate::device_profile::DeviceProfileCache;
use crate::executor::{
    AntigravityExecutor, ClaudeExecutor, CodexExecutor, CodexWsExecutor, CopilotExecutor,
    GeminiExecutor, IFlowExecutor, KimiExecutor, KiroExecutor, QwenExecutor,
};
use crate::{registry, retry};

/// Wraps a primary executor with a fallback: if the primary fails, the fallback is tried.
struct FallbackExecutor {
    primary: Box<dyn ProviderExecutor>,
    fallback: Box<dyn ProviderExecutor>,
}

#[async_trait]
impl ProviderExecutor for FallbackExecutor {
    async fn chat_completion(&self, request: ChatRequest) -> ProviderResult<ProviderResponse> {
        match self.primary.chat_completion(request.clone()).await {
            Ok(resp) => Ok(resp),
            Err(err) => {
                tracing::warn!(error = %err, "primary provider failed, falling back");
                self.fallback.chat_completion(request).await
            }
        }
    }

    fn supported_models(&self) -> Vec<String> {
        self.primary.supported_models()
    }
}

/// Create a boxed executor for the given provider.
///
/// Returns `None` if the provider is not supported.
pub fn make_executor(
    provider: &ProviderId,
    api_key: Option<String>,
    base_url: Option<String>,
    auth: Arc<AuthManager>,
    http: Client,
    ratelimit: Option<Arc<RateLimitStore>>,
) -> Option<Box<dyn ProviderExecutor>> {
    make_executor_with_cache(provider, api_key, base_url, auth, http, ratelimit, None)
}

/// Like [`make_executor`] but accepts an optional [`DeviceProfileCache`]
/// that is forwarded to providers supporting fingerprint stabilisation
/// (currently only Claude).
pub fn make_executor_with_cache(
    provider: &ProviderId,
    api_key: Option<String>,
    base_url: Option<String>,
    auth: Arc<AuthManager>,
    http: Client,
    ratelimit: Option<Arc<RateLimitStore>>,
    profile_cache: Option<Arc<DeviceProfileCache>>,
) -> Option<Box<dyn ProviderExecutor>> {
    match provider {
        ProviderId::Claude => Some(Box::new(
            ClaudeExecutor::builder()
                .http(http)
                .auth(auth)
                .maybe_api_key(api_key)
                .maybe_base_url(base_url)
                .maybe_ratelimit(ratelimit)
                .maybe_profile_cache(profile_cache)
                .build(),
        )),
        ProviderId::Codex => Some(Box::new(
            CodexExecutor::builder()
                .http(http)
                .auth(auth)
                .maybe_api_key(api_key)
                .maybe_base_url(base_url)
                .maybe_ratelimit(ratelimit)
                .build(),
        )),
        ProviderId::Gemini => Some(Box::new(
            GeminiExecutor::builder()
                .http(http)
                .auth(auth)
                .maybe_api_key(api_key)
                .maybe_base_url(base_url)
                .maybe_ratelimit(ratelimit)
                .build(),
        )),
        ProviderId::Kiro => Some(Box::new(
            KiroExecutor::builder()
                .http(http)
                .auth(auth)
                .maybe_api_key(api_key)
                .maybe_base_url(base_url)
                .maybe_ratelimit(ratelimit)
                .build(),
        )),
        ProviderId::Copilot => Some(Box::new(
            CopilotExecutor::builder()
                .http(http)
                .auth(auth)
                .maybe_api_key(api_key)
                .maybe_base_url(base_url)
                .maybe_ratelimit(ratelimit)
                .build(),
        )),
        ProviderId::Antigravity => Some(Box::new(
            AntigravityExecutor::builder()
                .http(http)
                .auth(auth)
                .maybe_api_key(api_key)
                .maybe_base_url(base_url)
                .maybe_ratelimit(ratelimit)
                .build(),
        )),
        ProviderId::Qwen => Some(Box::new(
            QwenExecutor::builder()
                .http(http)
                .auth(auth)
                .maybe_api_key(api_key)
                .maybe_base_url(base_url)
                .maybe_ratelimit(ratelimit)
                .build(),
        )),
        ProviderId::IFlow => Some(Box::new(
            IFlowExecutor::builder()
                .http(http)
                .auth(auth)
                .maybe_api_key(api_key)
                .maybe_base_url(base_url)
                .maybe_ratelimit(ratelimit)
                .build(),
        )),
        ProviderId::Kimi => Some(Box::new(
            KimiExecutor::builder()
                .http(http)
                .auth(auth)
                .maybe_api_key(api_key)
                .maybe_base_url(base_url)
                .maybe_ratelimit(ratelimit)
                .build(),
        )),
        ProviderId::Amp => None, // Amp is not a model provider
    }
}

/// Create an executor by resolving the model string to its provider.
///
/// Respects `ProviderConfig::backend` (always route to another provider),
/// `ProviderConfig::fallback` (wrap with a fallback executor), and
/// `ProviderConfig::api_keys` (multi-key retry with [`retry::RetryExecutor`]).
///
/// # Errors
///
/// Returns [`ByokError::UnsupportedModel`] if the model string is not recognised
/// or if the resolved provider does not have an executor implemented yet.
pub fn make_executor_for_model<S: BuildHasher>(
    model: &str,
    config_fn: impl Fn(&ProviderId) -> Option<ProviderConfig>,
    oauth_providers: &HashSet<ProviderId, S>,
    provider_hint: Option<&ProviderId>,
    auth: Arc<AuthManager>,
    http: Client,
    ratelimit: Option<Arc<RateLimitStore>>,
) -> Result<Box<dyn ProviderExecutor>, ByokError> {
    let provider = if let Some(p) = provider_hint {
        p.clone()
    } else {
        registry::resolve_provider_with(model, |p| {
            config_fn(p)
                .as_ref()
                .is_some_and(|c| c.api_key.is_some() || !c.api_keys.is_empty())
                || oauth_providers.contains(p)
        })
        .or_else(|| registry::resolve_provider(model))
        .ok_or_else(|| ByokError::UnsupportedModel(model.to_string()))?
    };

    let config = config_fn(&provider).unwrap_or_default();

    // If a backend override is set, route entirely to that provider.
    if let Some(backend_id) = &config.backend {
        let backend_config = config_fn(backend_id).unwrap_or_default();
        return make_executor(
            backend_id,
            backend_config.api_key,
            backend_config.base_url,
            auth,
            http,
            ratelimit,
        )
        .ok_or_else(|| ByokError::UnsupportedModel(model.to_string()));
    }

    // If multiple API keys are configured, use RetryExecutor for key rotation.
    let all_keys_with_urls = config.all_api_keys_with_base_url();
    if all_keys_with_urls.len() > 1 {
        let credentials: Vec<(String, Option<String>)> = all_keys_with_urls
            .into_iter()
            .map(|(k, u)| (k.to_string(), u.map(String::from)))
            .collect();
        // Need supported_models from a temporary executor to pass to RetryExecutor.
        let models = make_executor(
            &provider,
            None,
            config.base_url.clone(),
            Arc::clone(&auth),
            http.clone(),
            None,
        )
        .map(|e| e.supported_models())
        .unwrap_or_default();
        let primary: Box<dyn ProviderExecutor> = Box::new(retry::RetryExecutor::new(
            provider.clone(),
            credentials,
            config.routing,
            Arc::clone(&auth),
            http.clone(),
            models,
            ratelimit.clone(),
        ));

        // Wrap with fallback if configured.
        if let Some(fallback_id) = &config.fallback {
            let fallback_config = config_fn(fallback_id).unwrap_or_default();
            if let Some(fallback) = make_executor(
                fallback_id,
                fallback_config.api_key,
                fallback_config.base_url,
                auth,
                http,
                ratelimit,
            ) {
                return Ok(Box::new(FallbackExecutor { primary, fallback }));
            }
        }
        return Ok(primary);
    }

    // Build the primary executor (single key or OAuth).
    // For Codex with `websocket: true` and no API key, use WebSocket transport.
    let primary: Box<dyn ProviderExecutor> =
        if provider == ProviderId::Codex && config.websocket && config.api_key.is_none() {
            Box::new(CodexWsExecutor::new(Arc::clone(&auth)))
        } else {
            make_executor(
                &provider,
                config.api_key,
                config.base_url,
                Arc::clone(&auth),
                http.clone(),
                ratelimit.clone(),
            )
            .ok_or_else(|| ByokError::UnsupportedModel(model.to_string()))?
        };

    // If a fallback is configured, wrap in FallbackExecutor.
    if let Some(fallback_id) = &config.fallback {
        let fallback_config = config_fn(fallback_id).unwrap_or_default();
        if let Some(fallback) = make_executor(
            fallback_id,
            fallback_config.api_key,
            fallback_config.base_url,
            auth,
            http,
            ratelimit,
        ) {
            return Ok(Box::new(FallbackExecutor { primary, fallback }));
        }
    }

    Ok(primary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use byokey_store::InMemoryTokenStore;

    fn make_auth() -> Arc<AuthManager> {
        Arc::new(AuthManager::new(
            Arc::new(InMemoryTokenStore::new()),
            rquest::Client::new(),
        ))
    }

    fn make_http() -> Client {
        Client::new()
    }

    fn empty_oauth() -> HashSet<ProviderId> {
        HashSet::new()
    }

    #[test]
    fn test_make_executor_claude() {
        let auth = make_auth();
        let ex = make_executor(&ProviderId::Claude, None, None, auth, make_http(), None);
        assert!(ex.is_some());
        assert!(
            ex.unwrap()
                .supported_models()
                .iter()
                .any(|m| m.starts_with("claude-"))
        );
    }

    #[test]
    fn test_make_executor_codex() {
        let auth = make_auth();
        let ex = make_executor(
            &ProviderId::Codex,
            Some("sk-test".into()),
            None,
            auth,
            make_http(),
            None,
        );
        assert!(ex.is_some());
    }

    #[test]
    fn test_make_executor_gemini() {
        let auth = make_auth();
        let ex = make_executor(&ProviderId::Gemini, None, None, auth, make_http(), None);
        assert!(ex.is_some());
    }

    #[test]
    fn test_make_executor_copilot() {
        let auth = make_auth();
        let ex = make_executor(&ProviderId::Copilot, None, None, auth, make_http(), None);
        assert!(ex.is_some());
    }

    #[test]
    fn test_make_executor_antigravity() {
        let auth = make_auth();
        let ex = make_executor(
            &ProviderId::Antigravity,
            None,
            None,
            auth,
            make_http(),
            None,
        );
        assert!(ex.is_some());
        assert!(
            ex.unwrap()
                .supported_models()
                .iter()
                .any(|m| m.starts_with("ag-"))
        );
    }

    #[test]
    fn test_make_executor_kimi() {
        let auth = make_auth();
        let ex = make_executor(&ProviderId::Kimi, None, None, auth, make_http(), None);
        assert!(ex.is_some());
        assert!(
            ex.unwrap()
                .supported_models()
                .iter()
                .any(|m| m.starts_with("kimi-"))
        );
    }

    #[test]
    fn test_make_executor_for_model_claude() {
        let auth = make_auth();
        let ex = make_executor_for_model(
            "claude-opus-4-6",
            |_| None,
            &empty_oauth(),
            None,
            auth,
            make_http(),
            None,
        );
        assert!(ex.is_ok());
    }

    #[test]
    fn test_make_executor_for_model_unknown() {
        let auth = make_auth();
        let result = make_executor_for_model(
            "nonexistent-model",
            |_| None,
            &empty_oauth(),
            None,
            auth,
            make_http(),
            None,
        );
        assert!(matches!(result, Err(ByokError::UnsupportedModel(_))));
    }

    #[test]
    fn test_make_executor_for_model_passes_api_key() {
        let auth = make_auth();
        let ex = make_executor_for_model(
            "gpt-4o",
            |p| match p {
                ProviderId::Copilot => Some(ProviderConfig {
                    api_key: Some("sk-test".into()),
                    ..Default::default()
                }),
                _ => None,
            },
            &empty_oauth(),
            None,
            auth,
            make_http(),
            None,
        );
        assert!(ex.is_ok());
    }

    #[test]
    fn test_make_executor_for_model_backend_override() {
        let auth = make_auth();
        // gemini model with backend: copilot → should create a Copilot executor
        let ex = make_executor_for_model(
            "gemini-2.0-flash",
            |p| match p {
                ProviderId::Gemini => Some(ProviderConfig {
                    backend: Some(ProviderId::Copilot),
                    ..Default::default()
                }),
                _ => None,
            },
            &empty_oauth(),
            None,
            auth,
            make_http(),
            None,
        );
        assert!(ex.is_ok());
    }

    #[test]
    fn test_make_executor_for_model_fallback() {
        let auth = make_auth();
        // gemini model with fallback: copilot → should create a FallbackExecutor
        let ex = make_executor_for_model(
            "gemini-2.0-flash",
            |p| match p {
                ProviderId::Gemini => Some(ProviderConfig {
                    fallback: Some(ProviderId::Copilot),
                    ..Default::default()
                }),
                _ => None,
            },
            &empty_oauth(),
            None,
            auth,
            make_http(),
            None,
        );
        assert!(ex.is_ok());
        // FallbackExecutor delegates supported_models to primary (Gemini)
        let models = ex.unwrap().supported_models();
        assert!(models.iter().any(|m| m.starts_with("gemini-")));
    }

    #[test]
    fn test_make_executor_for_model_multi_key_retry() {
        use byokey_config::ApiKeyEntry;

        let auth = make_auth();
        let ex = make_executor_for_model(
            "claude-opus-4-6",
            |p| match p {
                ProviderId::Claude => Some(ProviderConfig {
                    api_keys: vec![
                        ApiKeyEntry {
                            api_key: "sk-key-1".into(),
                            label: None,
                            base_url: None,
                        },
                        ApiKeyEntry {
                            api_key: "sk-key-2".into(),
                            label: None,
                            base_url: None,
                        },
                    ],
                    ..Default::default()
                }),
                _ => None,
            },
            &empty_oauth(),
            None,
            auth,
            make_http(),
            None,
        );
        assert!(ex.is_ok());
        // RetryExecutor delegates supported_models from the provider
        let models = ex.unwrap().supported_models();
        assert!(models.iter().any(|m| m.starts_with("claude-")));
    }

    #[test]
    fn test_make_executor_for_model_single_api_key_no_retry() {
        let auth = make_auth();
        // Single api_key → no RetryExecutor, direct executor
        let ex = make_executor_for_model(
            "claude-opus-4-6",
            |p| match p {
                ProviderId::Claude => Some(ProviderConfig {
                    api_key: Some("sk-single".into()),
                    ..Default::default()
                }),
                _ => None,
            },
            &empty_oauth(),
            None,
            auth,
            make_http(),
            None,
        );
        assert!(ex.is_ok());
    }
}
