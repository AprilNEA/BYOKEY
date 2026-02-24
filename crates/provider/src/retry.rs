//! Retry executor — wraps a provider with multi-key rotation on retryable errors.
//!
//! When a provider has multiple API keys configured, the `RetryExecutor`
//! tries each key in round-robin order (using [`CredentialRouter`]) until
//! a request succeeds or all keys are exhausted / in cooldown.

use crate::routing::CredentialRouter;
use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_types::{
    ChatRequest, ProviderId,
    traits::{ProviderExecutor, ProviderResponse, Result},
};
use rquest::Client;
use std::{sync::Arc, time::Duration};

/// Default cooldown duration for a key after a retryable error.
const COOLDOWN_DURATION: Duration = Duration::from_secs(30);

/// Wraps a provider with multi-key retry: on retryable errors, marks the
/// current key as cooled down and retries with the next available key.
pub struct RetryExecutor {
    provider: ProviderId,
    router: Arc<CredentialRouter>,
    auth: Arc<AuthManager>,
    http: Client,
    models: Vec<String>,
}

impl RetryExecutor {
    /// Creates a new retry executor.
    ///
    /// `keys` must contain at least one key.
    ///
    /// # Panics
    ///
    /// Panics if `keys` is empty (propagated from [`CredentialRouter::new`]).
    pub fn new(
        provider: ProviderId,
        keys: Vec<String>,
        auth: Arc<AuthManager>,
        http: Client,
        models: Vec<String>,
    ) -> Self {
        Self {
            provider,
            router: Arc::new(CredentialRouter::new(keys, COOLDOWN_DURATION)),
            auth,
            http,
            models,
        }
    }
}

#[async_trait]
impl ProviderExecutor for RetryExecutor {
    async fn chat_completion(&self, request: ChatRequest) -> Result<ProviderResponse> {
        let max_attempts = self.router.len().min(3);
        let mut last_err = None;

        for _ in 0..max_attempts {
            let key = match self.router.next_key() {
                Some(k) => k.to_string(),
                None => break, // all keys in cooldown
            };

            let executor = crate::make_executor(
                &self.provider,
                Some(key.clone()),
                Arc::clone(&self.auth),
                self.http.clone(),
            );

            let Some(executor) = executor else {
                break;
            };

            match executor.chat_completion(request.clone()).await {
                Ok(resp) => return Ok(resp),
                Err(e) if e.is_retryable() => {
                    tracing::warn!(
                        provider = %self.provider,
                        error = %e,
                        "retryable error, rotating key"
                    );
                    self.router.mark_error(&key);
                    last_err = Some(e);
                }
                Err(e) => return Err(e),
            }
        }

        Err(last_err.unwrap_or_else(|| {
            byokey_types::ByokError::Http(format!(
                "{}: all API keys exhausted or in cooldown",
                self.provider
            ))
        }))
    }

    fn supported_models(&self) -> Vec<String> {
        self.models.clone()
    }
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

    #[test]
    fn test_retry_executor_models() {
        let exec = RetryExecutor::new(
            ProviderId::Claude,
            vec!["key-1".into()],
            make_auth(),
            Client::new(),
            vec!["claude-opus-4-5".into()],
        );
        assert_eq!(exec.supported_models(), vec!["claude-opus-4-5"]);
    }

    #[test]
    fn test_retry_executor_multiple_keys() {
        let exec = RetryExecutor::new(
            ProviderId::Claude,
            vec!["key-1".into(), "key-2".into(), "key-3".into()],
            make_auth(),
            Client::new(),
            vec!["claude-opus-4-5".into()],
        );
        assert_eq!(exec.supported_models().len(), 1);
    }
}
