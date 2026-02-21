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

use byokey_auth::AuthManager;
use byokey_types::{ByokError, ProviderId, traits::ProviderExecutor};
use std::sync::Arc;

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
/// # Errors
///
/// Returns [`ByokError::UnsupportedModel`] if the model string is not recognised
/// or if the resolved provider does not have an executor implemented yet.
pub fn make_executor_for_model(
    model: &str,
    api_key_fn: impl Fn(&ProviderId) -> Option<String>,
    auth: Arc<AuthManager>,
) -> Result<Box<dyn ProviderExecutor>, ByokError> {
    let provider = registry::resolve_provider(model)
        .ok_or_else(|| ByokError::UnsupportedModel(model.to_string()))?;
    make_executor(&provider, api_key_fn(&provider), auth)
        .ok_or_else(|| ByokError::UnsupportedModel(model.to_string()))
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
                ProviderId::Codex => Some("sk-test".into()),
                _ => None,
            },
            auth,
        );
        assert!(ex.is_ok());
    }
}
