//! Provider executor implementations and model registry.
//!
//! Each provider module implements [`ProviderExecutor`] for a specific AI backend.
//! The [`make_executor`] and [`make_executor_for_model`] functions create boxed
//! executors based on provider or model identifiers.

pub mod antigravity;
pub mod claude;
pub mod codex;
pub mod copilot;
pub mod gemini;
pub mod iflow;
pub mod kiro;
pub mod qwen;
pub mod registry;

pub use antigravity::AntigravityExecutor;
pub use claude::ClaudeExecutor;
pub use codex::CodexExecutor;
pub use copilot::CopilotExecutor;
pub use gemini::GeminiExecutor;
pub use iflow::IFlowExecutor;
pub use kiro::KiroExecutor;
pub use qwen::QwenExecutor;
pub use registry::resolve_provider;

use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_config::ProviderConfig;
use byokey_types::{
    ByokError, ProviderId,
    traits::{ProviderExecutor, ProviderResponse, Result as ProviderResult},
};
use std::sync::Arc;

/// Wraps a primary executor with a fallback: if the primary fails, the fallback is tried.
struct FallbackExecutor {
    primary: Box<dyn ProviderExecutor>,
    fallback: Box<dyn ProviderExecutor>,
}

#[async_trait]
impl ProviderExecutor for FallbackExecutor {
    async fn chat_completion(
        &self,
        request: serde_json::Value,
        stream: bool,
    ) -> ProviderResult<ProviderResponse> {
        match self.primary.chat_completion(request.clone(), stream).await {
            Ok(resp) => Ok(resp),
            Err(err) => {
                eprintln!("[byokey] primary provider failed ({err}), falling back to secondary");
                self.fallback.chat_completion(request, stream).await
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
    auth: Arc<AuthManager>,
) -> Option<Box<dyn ProviderExecutor>> {
    match provider {
        ProviderId::Claude => Some(Box::new(ClaudeExecutor::new(api_key, auth))),
        ProviderId::Codex => Some(Box::new(CodexExecutor::new(api_key, auth))),
        ProviderId::Gemini => Some(Box::new(GeminiExecutor::new(api_key, auth))),
        ProviderId::Kiro => Some(Box::new(KiroExecutor::new(api_key, auth))),
        ProviderId::Copilot => Some(Box::new(CopilotExecutor::new(api_key, auth))),
        ProviderId::Antigravity => Some(Box::new(AntigravityExecutor::new(api_key, auth))),
        ProviderId::Qwen => Some(Box::new(QwenExecutor::new(api_key, auth))),
        ProviderId::IFlow => Some(Box::new(IFlowExecutor::new(api_key, auth))),
        ProviderId::Kimi => None, // executor not yet implemented; auth flow is ready
    }
}

/// Create an executor by resolving the model string to its provider.
///
/// Respects `ProviderConfig::backend` (always route to another provider) and
/// `ProviderConfig::fallback` (wrap with a fallback executor).
///
/// # Errors
///
/// Returns [`ByokError::UnsupportedModel`] if the model string is not recognised
/// or if the resolved provider does not have an executor implemented yet.
pub fn make_executor_for_model(
    model: &str,
    config_fn: impl Fn(&ProviderId) -> Option<ProviderConfig>,
    auth: Arc<AuthManager>,
) -> Result<Box<dyn ProviderExecutor>, ByokError> {
    let provider = registry::resolve_provider(model)
        .ok_or_else(|| ByokError::UnsupportedModel(model.to_string()))?;

    let config = config_fn(&provider).unwrap_or_default();

    // If a backend override is set, route entirely to that provider.
    if let Some(backend_id) = &config.backend {
        let backend_config = config_fn(backend_id).unwrap_or_default();
        return make_executor(backend_id, backend_config.api_key, auth)
            .ok_or_else(|| ByokError::UnsupportedModel(model.to_string()));
    }

    // Build the primary executor.
    let primary = make_executor(&provider, config.api_key, Arc::clone(&auth))
        .ok_or_else(|| ByokError::UnsupportedModel(model.to_string()))?;

    // If a fallback is configured, wrap in FallbackExecutor.
    if let Some(fallback_id) = &config.fallback {
        let fallback_config = config_fn(fallback_id).unwrap_or_default();
        if let Some(fallback) = make_executor(fallback_id, fallback_config.api_key, auth) {
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
        Arc::new(AuthManager::new(Arc::new(InMemoryTokenStore::new())))
    }

    #[test]
    fn test_make_executor_claude() {
        let auth = make_auth();
        let ex = make_executor(&ProviderId::Claude, None, auth);
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
        let ex = make_executor(&ProviderId::Codex, Some("sk-test".into()), auth);
        assert!(ex.is_some());
    }

    #[test]
    fn test_make_executor_gemini() {
        let auth = make_auth();
        let ex = make_executor(&ProviderId::Gemini, None, auth);
        assert!(ex.is_some());
    }

    #[test]
    fn test_make_executor_copilot() {
        let auth = make_auth();
        let ex = make_executor(&ProviderId::Copilot, None, auth);
        assert!(ex.is_some());
    }

    #[test]
    fn test_make_executor_antigravity() {
        let auth = make_auth();
        let ex = make_executor(&ProviderId::Antigravity, None, auth);
        assert!(ex.is_some());
        assert!(
            ex.unwrap()
                .supported_models()
                .iter()
                .all(|m| m.starts_with("ag-"))
        );
    }

    #[test]
    fn test_make_executor_for_model_claude() {
        let auth = make_auth();
        let ex = make_executor_for_model("claude-opus-4-6", |_| None, auth);
        assert!(ex.is_ok());
    }

    #[test]
    fn test_make_executor_for_model_unknown() {
        let auth = make_auth();
        let result = make_executor_for_model("nonexistent-model", |_| None, auth);
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
            auth,
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
            auth,
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
            auth,
        );
        assert!(ex.is_ok());
        // FallbackExecutor delegates supported_models to primary (Gemini)
        let models = ex.unwrap().supported_models();
        assert!(models.iter().any(|m| m.starts_with("gemini-")));
    }
}
