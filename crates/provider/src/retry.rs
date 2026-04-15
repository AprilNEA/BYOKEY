//! Retry executor — wraps a provider with multi-key rotation on retryable errors.
//!
//! When a provider has multiple API keys configured, the `RetryExecutor`
//! tries each key in round-robin order (using [`CredentialRouter`]) until
//! a request succeeds or all keys are exhausted / in cooldown.

use crate::routing::{CredentialRouter, RoutingStrategy};
use crate::versions::VersionStore;
use async_trait::async_trait;
use byokey_auth::AuthManager;
use byokey_config::KeyRoutingStrategy;
use byokey_types::{
    ChatRequest, ProviderId, RateLimitStore,
    traits::{ProviderExecutor, ProviderResponse, Result},
};
use rquest::Client;
use std::collections::HashMap;
use std::{sync::Arc, time::Duration};

/// Default cooldown duration for a key after a retryable error.
const COOLDOWN_DURATION: Duration = Duration::from_secs(30);

/// Wraps a provider with multi-key retry: on retryable errors, marks the
/// current key as cooled down and retries with the next available key.
///
/// Each key can have its own `base_url`, enabling parallel use of
/// official and third-party endpoints for the same provider type.
pub struct RetryExecutor {
    provider: ProviderId,
    router: Arc<CredentialRouter>,
    /// Per-key base URL overrides.
    base_urls: HashMap<String, Option<String>>,
    auth: Arc<AuthManager>,
    http: Client,
    models: Vec<String>,
    ratelimit: Option<Arc<RateLimitStore>>,
    versions: VersionStore,
}

impl RetryExecutor {
    /// Creates a new retry executor.
    ///
    /// `credentials` contains `(api_key, base_url)` pairs. Each key can
    /// optionally target a different endpoint.
    ///
    /// `strategy` controls how keys are selected:
    /// - `RoundRobin`: rotate evenly across all keys.
    /// - `Priority`: always prefer the first ready key, only try later keys on failure.
    ///
    /// # Panics
    ///
    /// Panics if `credentials` is empty (propagated from [`CredentialRouter::new`]).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        provider: ProviderId,
        credentials: Vec<(String, Option<String>)>,
        strategy: KeyRoutingStrategy,
        auth: Arc<AuthManager>,
        http: Client,
        models: Vec<String>,
        ratelimit: Option<Arc<RateLimitStore>>,
        versions: VersionStore,
    ) -> Self {
        let keys: Vec<String> = credentials.iter().map(|(k, _)| k.clone()).collect();
        let base_urls: HashMap<String, Option<String>> = credentials.into_iter().collect();
        let routing_strategy = match strategy {
            KeyRoutingStrategy::RoundRobin => RoutingStrategy::RoundRobin,
            KeyRoutingStrategy::Priority => RoutingStrategy::FillFirst,
        };
        Self {
            provider,
            router: Arc::new(
                CredentialRouter::new(keys, COOLDOWN_DURATION).with_strategy(routing_strategy),
            ),
            base_urls,
            auth,
            http,
            models,
            ratelimit,
            versions,
        }
    }
}

#[async_trait]
impl ProviderExecutor for RetryExecutor {
    async fn chat_completion(&self, request: ChatRequest) -> Result<ProviderResponse> {
        let max_attempts = self
            .router
            .max_retry()
            .unwrap_or_else(|| self.router.len().min(3));
        let mut last_err = None;

        for _ in 0..max_attempts {
            let key = match self.router.next_key() {
                Some(k) => k.to_string(),
                None => break, // all keys in cooldown
            };

            let base_url = self.base_urls.get(&key).cloned().flatten();
            let executor = crate::factory::make_executor(
                &self.provider,
                Some(key.clone()),
                base_url,
                Arc::clone(&self.auth),
                self.http.clone(),
                self.ratelimit.clone(),
                &self.versions,
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
                    if let Some(delay) = e.retry_after() {
                        self.router.mark_error_with_delay(&key, delay);
                    } else {
                        self.router.mark_error(&key);
                    }
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
            vec![("key-1".into(), None)],
            KeyRoutingStrategy::default(),
            make_auth(),
            Client::new(),
            vec!["claude-opus-4-5".into()],
            None,
            VersionStore::empty(),
        );
        assert_eq!(exec.supported_models(), vec!["claude-opus-4-5"]);
    }

    #[test]
    fn test_retry_executor_multiple_keys() {
        let exec = RetryExecutor::new(
            ProviderId::Claude,
            vec![
                ("key-1".into(), None),
                ("key-2".into(), None),
                ("key-3".into(), None),
            ],
            KeyRoutingStrategy::default(),
            make_auth(),
            Client::new(),
            vec!["claude-opus-4-5".into()],
            None,
            VersionStore::empty(),
        );
        assert_eq!(exec.supported_models().len(), 1);
    }
}
