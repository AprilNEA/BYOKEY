//! In-memory token store backed by a `HashMap` behind a `Mutex`.

use async_trait::async_trait;
use byokey_types::{OAuthToken, ProviderId, TokenStore, traits::Result};
use std::collections::HashMap;
use std::sync::Mutex;

/// An in-memory [`TokenStore`] implementation for testing and ephemeral use.
pub struct InMemoryTokenStore {
    /// Provider-keyed token map.
    data: Mutex<HashMap<ProviderId, OAuthToken>>,
}

impl InMemoryTokenStore {
    /// Creates a new empty in-memory token store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryTokenStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl TokenStore for InMemoryTokenStore {
    /// Loads the token for the given provider, if present.
    async fn load(&self, provider: &ProviderId) -> Result<Option<OAuthToken>> {
        Ok(self.data.lock().unwrap().get(provider).cloned())
    }

    /// Saves (or overwrites) the token for the given provider.
    async fn save(&self, provider: &ProviderId, token: &OAuthToken) -> Result<()> {
        self.data
            .lock()
            .unwrap()
            .insert(provider.clone(), token.clone());
        Ok(())
    }

    /// Removes the token for the given provider.
    async fn remove(&self, provider: &ProviderId) -> Result<()> {
        self.data.lock().unwrap().remove(provider);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_save_and_load() {
        let store = InMemoryTokenStore::new();
        let token = OAuthToken::new("test-access");
        store.save(&ProviderId::Claude, &token).await.unwrap();
        let loaded = store.load(&ProviderId::Claude).await.unwrap().unwrap();
        assert_eq!(loaded.access_token, "test-access");
    }

    #[tokio::test]
    async fn test_load_missing() {
        let store = InMemoryTokenStore::new();
        assert!(store.load(&ProviderId::Gemini).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_remove() {
        let store = InMemoryTokenStore::new();
        store
            .save(&ProviderId::Codex, &OAuthToken::new("tok"))
            .await
            .unwrap();
        store.remove(&ProviderId::Codex).await.unwrap();
        assert!(store.load(&ProviderId::Codex).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_overwrite() {
        let store = InMemoryTokenStore::new();
        store
            .save(&ProviderId::Claude, &OAuthToken::new("first"))
            .await
            .unwrap();
        store
            .save(&ProviderId::Claude, &OAuthToken::new("second"))
            .await
            .unwrap();
        let loaded = store.load(&ProviderId::Claude).await.unwrap().unwrap();
        assert_eq!(loaded.access_token, "second");
    }

    #[tokio::test]
    async fn test_multiple_providers() {
        let store = InMemoryTokenStore::new();
        store
            .save(&ProviderId::Claude, &OAuthToken::new("claude-tok"))
            .await
            .unwrap();
        store
            .save(&ProviderId::Gemini, &OAuthToken::new("gemini-tok"))
            .await
            .unwrap();
        assert_eq!(
            store
                .load(&ProviderId::Claude)
                .await
                .unwrap()
                .unwrap()
                .access_token,
            "claude-tok"
        );
        assert_eq!(
            store
                .load(&ProviderId::Gemini)
                .await
                .unwrap()
                .unwrap()
                .access_token,
            "gemini-tok"
        );
    }
}
