//! Unified manager for OAuth token lifecycles across all providers.
//!
//! Responsibilities:
//! - Load tokens from a [`TokenStore`].
//! - Detect expiration and trigger refresh.
//! - Cooldown duration to prevent excessive refresh attempts (30 s).
//! - Background async refresh (non-blocking on the request path).
use byok_types::{ByokError, OAuthToken, ProviderId, TokenState, TokenStore, traits::Result};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const REFRESH_COOLDOWN: Duration = Duration::from_secs(30);

struct ProviderState {
    last_refresh_attempt: Option<Instant>,
}

pub struct AuthManager {
    store: Arc<dyn TokenStore>,
    state: Mutex<HashMap<ProviderId, ProviderState>>,
}

impl AuthManager {
    pub fn new(store: Arc<dyn TokenStore>) -> Self {
        Self {
            store,
            state: Mutex::new(HashMap::new()),
        }
    }

    /// Retrieve a valid token, attempting a refresh if expired.
    ///
    /// # Errors
    ///
    /// Returns an error if the token is not found, expired and cannot be refreshed, or invalid.
    pub async fn get_token(&self, provider: &ProviderId) -> Result<OAuthToken> {
        let token = self
            .store
            .load(provider)
            .await?
            .ok_or_else(|| ByokError::TokenNotFound(provider.clone()))?;

        match token.state() {
            TokenState::Valid => Ok(token),
            TokenState::Expired => self.refresh_token(provider, &token).await,
            TokenState::Invalid => Err(ByokError::TokenExpired(provider.clone())),
        }
    }

    /// Check whether the provider is authenticated (token exists and is not invalid).
    pub async fn is_authenticated(&self, provider: &ProviderId) -> bool {
        match self.store.load(provider).await {
            Ok(Some(t)) => t.state() != TokenState::Invalid,
            _ => false,
        }
    }

    /// Save a new token.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying store fails to persist the token.
    pub async fn save_token(&self, provider: &ProviderId, token: OAuthToken) -> Result<()> {
        self.store.save(provider, &token).await
    }

    /// Remove a token (logout).
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying store fails to remove the token.
    pub async fn remove_token(&self, provider: &ProviderId) -> Result<()> {
        self.store.remove(provider).await
    }

    #[allow(clippy::unused_async)]
    async fn refresh_token(
        &self,
        provider: &ProviderId,
        _token: &OAuthToken,
    ) -> Result<OAuthToken> {
        // Check cooldown period
        {
            let state = self.state.lock().unwrap();
            if let Some(ps) = state.get(provider)
                && let Some(last) = ps.last_refresh_attempt
                && last.elapsed() < REFRESH_COOLDOWN
            {
                return Err(ByokError::Auth(format!(
                    "refresh cooldown active for {provider}"
                )));
            }
        }
        // Record refresh attempt timestamp
        {
            let mut state = self.state.lock().unwrap();
            state.insert(
                provider.clone(),
                ProviderState {
                    last_refresh_attempt: Some(Instant::now()),
                },
            );
        }
        // Actual refresh is implemented by each provider module; return an error for the caller to handle
        Err(ByokError::Auth(format!(
            "token refresh not implemented for {provider}; please re-authenticate"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byok_store::InMemoryTokenStore;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_manager() -> AuthManager {
        AuthManager::new(Arc::new(InMemoryTokenStore::new()))
    }

    fn past_ts(secs: u64) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .saturating_sub(secs)
    }

    #[tokio::test]
    async fn test_get_token_not_found() {
        let m = make_manager();
        let err = m.get_token(&ProviderId::Claude).await.unwrap_err();
        assert!(matches!(err, ByokError::TokenNotFound(_)));
    }

    #[tokio::test]
    async fn test_get_valid_token() {
        let m = make_manager();
        let tok = OAuthToken::new("valid").with_expiry(3600);
        m.save_token(&ProviderId::Claude, tok).await.unwrap();
        let got = m.get_token(&ProviderId::Claude).await.unwrap();
        assert_eq!(got.access_token, "valid");
    }

    #[tokio::test]
    async fn test_get_expired_no_refresh_token() {
        let m = make_manager();
        let tok = OAuthToken {
            access_token: "old".into(),
            refresh_token: None,
            expires_at: Some(past_ts(100)),
            token_type: None,
        };
        m.save_token(&ProviderId::Gemini, tok).await.unwrap();
        let err = m.get_token(&ProviderId::Gemini).await.unwrap_err();
        assert!(matches!(err, ByokError::TokenExpired(_)));
    }

    #[tokio::test]
    async fn test_is_authenticated_false_when_missing() {
        let m = make_manager();
        assert!(!m.is_authenticated(&ProviderId::Codex).await);
    }

    #[tokio::test]
    async fn test_is_authenticated_true_when_valid() {
        let m = make_manager();
        m.save_token(&ProviderId::Codex, OAuthToken::new("tok"))
            .await
            .unwrap();
        assert!(m.is_authenticated(&ProviderId::Codex).await);
    }

    #[tokio::test]
    async fn test_remove_token() {
        let m = make_manager();
        m.save_token(&ProviderId::Kiro, OAuthToken::new("tok"))
            .await
            .unwrap();
        m.remove_token(&ProviderId::Kiro).await.unwrap();
        assert!(!m.is_authenticated(&ProviderId::Kiro).await);
    }

    #[tokio::test]
    async fn test_refresh_cooldown() {
        let m = make_manager();
        // Insert an expired token that has a refresh_token
        let tok = OAuthToken {
            access_token: "old".into(),
            refresh_token: Some("ref".into()),
            expires_at: Some(past_ts(100)),
            token_type: None,
        };
        m.save_token(&ProviderId::Copilot, tok).await.unwrap();

        // First refresh attempt (expected to fail, but not due to cooldown)
        let err1 = m.get_token(&ProviderId::Copilot).await.unwrap_err();
        assert!(matches!(err1, ByokError::Auth(_)));

        // Second attempt immediately (should hit cooldown)
        let err2 = m.get_token(&ProviderId::Copilot).await.unwrap_err();
        let msg = err2.to_string();
        assert!(
            msg.contains("cooldown"),
            "expected cooldown error, got: {msg}"
        );
    }
}
