//! In-memory token store backed by a `HashMap` behind a `Mutex`.
//!
//! Supports multi-account storage with `(ProviderId, account_id)` composite keys.

use async_trait::async_trait;
use byokey_types::{AccountInfo, AccountToken, OAuthToken, ProviderId, Result, TokenStore};
use std::collections::HashMap;
use std::sync::Mutex;

/// Key for the in-memory store: `(provider, account_id)`.
type AccountKey = (ProviderId, String);

/// Per-account entry.
struct AccountEntry {
    token: OAuthToken,
    label: Option<String>,
    is_active: bool,
}

/// An in-memory [`TokenStore`] implementation for testing and ephemeral use.
pub struct InMemoryTokenStore {
    data: Mutex<HashMap<AccountKey, AccountEntry>>,
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
    // ── Active-account shortcuts ──────────────────────────────────────────

    async fn load(&self, provider: ProviderId) -> Result<Option<OAuthToken>> {
        let data = self.data.lock().unwrap();
        Ok(data
            .iter()
            .find(|((p, _), e)| *p == provider && e.is_active)
            .map(|(_, e)| e.token.clone()))
    }

    async fn save(&self, provider: ProviderId, token: &OAuthToken) -> Result<()> {
        // Resolve the active account_id so refreshes update the row that
        // `load()` reads, rather than orphaning a second non-active row.
        let active_id = {
            let data = self.data.lock().unwrap();
            data.iter()
                .find(|((p, _), e)| *p == provider && e.is_active)
                .map(|((_, id), _)| id.clone())
        };
        let account_id = active_id.as_deref().unwrap_or("default");
        self.save_account(provider, account_id, None, token).await
    }

    async fn remove(&self, provider: ProviderId) -> Result<()> {
        let mut data = self.data.lock().unwrap();
        // Find and remove the active account.
        let active_key = data
            .iter()
            .find(|((p, _), e)| *p == provider && e.is_active)
            .map(|(k, _)| k.clone());
        if let Some(key) = active_key {
            data.remove(&key);
        }
        Ok(())
    }

    // ── Multi-account operations ──────────────────────────────────────────

    async fn load_account(
        &self,
        provider: ProviderId,
        account_id: &str,
    ) -> Result<Option<OAuthToken>> {
        let data = self.data.lock().unwrap();
        let key = (provider, account_id.to_string());
        Ok(data.get(&key).map(|e| e.token.clone()))
    }

    async fn save_account(
        &self,
        provider: ProviderId,
        account_id: &str,
        label: Option<&str>,
        token: &OAuthToken,
    ) -> Result<()> {
        let mut data = self.data.lock().unwrap();
        let key = (provider, account_id.to_string());

        // Check if any active account exists for this provider.
        let has_active = data.iter().any(|((p, _), e)| *p == provider && e.is_active);

        if let Some(entry) = data.get_mut(&key) {
            entry.token = token.clone();
            if let Some(l) = label {
                entry.label = Some(l.to_string());
            }
        } else {
            data.insert(
                key,
                AccountEntry {
                    token: token.clone(),
                    label: label.map(String::from),
                    is_active: !has_active,
                },
            );
        }
        Ok(())
    }

    async fn remove_account(&self, provider: ProviderId, account_id: &str) -> Result<()> {
        let mut data = self.data.lock().unwrap();
        let key = (provider, account_id.to_string());
        data.remove(&key);
        Ok(())
    }

    async fn list_accounts(&self, provider: ProviderId) -> Result<Vec<AccountInfo>> {
        let data = self.data.lock().unwrap();
        let mut accounts: Vec<AccountInfo> = data
            .iter()
            .filter(|((p, _), _)| *p == provider)
            .map(|((_, id), e)| AccountInfo {
                account_id: id.clone(),
                label: e.label.clone(),
                is_active: e.is_active,
            })
            .collect();
        // Active first, then alphabetical.
        accounts.sort_by(|a, b| {
            b.is_active
                .cmp(&a.is_active)
                .then(a.account_id.cmp(&b.account_id))
        });
        Ok(accounts)
    }

    async fn set_active(&self, provider: ProviderId, account_id: &str) -> Result<()> {
        let mut data = self.data.lock().unwrap();
        let target_key = (provider, account_id.to_string());
        if !data.contains_key(&target_key) {
            return Err(byokey_types::ByokError::Storage(format!(
                "account '{account_id}' not found for provider {provider}"
            )));
        }
        // Deactivate all, then activate the target.
        for ((p, _), entry) in data.iter_mut() {
            if *p == provider {
                entry.is_active = false;
            }
        }
        data.get_mut(&target_key).unwrap().is_active = true;
        Ok(())
    }

    async fn load_all_tokens(&self, provider: ProviderId) -> Result<Vec<AccountToken>> {
        let data = self.data.lock().unwrap();
        let mut entries: Vec<(&String, &AccountEntry)> = data
            .iter()
            .filter(|((p, _), _)| *p == provider)
            .map(|((_, id), e)| (id, e))
            .collect();
        // Active first, then alphabetical.
        entries.sort_by(|(a_id, a), (b_id, b)| b.is_active.cmp(&a.is_active).then(a_id.cmp(b_id)));
        Ok(entries
            .into_iter()
            .map(|(id, e)| AccountToken {
                account_id: id.clone(),
                token: e.token.clone(),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_save_and_load() {
        let store = InMemoryTokenStore::new();
        let token = OAuthToken::new("test-access");
        store.save(ProviderId::Claude, &token).await.unwrap();
        let loaded = store.load(ProviderId::Claude).await.unwrap().unwrap();
        assert_eq!(loaded.access_token, "test-access");
    }

    #[tokio::test]
    async fn test_load_missing() {
        let store = InMemoryTokenStore::new();
        assert!(store.load(ProviderId::Cursor).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_remove() {
        let store = InMemoryTokenStore::new();
        store
            .save(ProviderId::Copilot, &OAuthToken::new("tok"))
            .await
            .unwrap();
        store.remove(ProviderId::Copilot).await.unwrap();
        assert!(store.load(ProviderId::Copilot).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_overwrite() {
        let store = InMemoryTokenStore::new();
        store
            .save(ProviderId::Claude, &OAuthToken::new("first"))
            .await
            .unwrap();
        store
            .save(ProviderId::Claude, &OAuthToken::new("second"))
            .await
            .unwrap();
        let loaded = store.load(ProviderId::Claude).await.unwrap().unwrap();
        assert_eq!(loaded.access_token, "second");
    }

    #[tokio::test]
    async fn test_multiple_providers() {
        let store = InMemoryTokenStore::new();
        store
            .save(ProviderId::Claude, &OAuthToken::new("claude-tok"))
            .await
            .unwrap();
        store
            .save(ProviderId::Cursor, &OAuthToken::new("cursor-tok"))
            .await
            .unwrap();
        assert_eq!(
            store
                .load(ProviderId::Claude)
                .await
                .unwrap()
                .unwrap()
                .access_token,
            "claude-tok"
        );
        assert_eq!(
            store
                .load(ProviderId::Cursor)
                .await
                .unwrap()
                .unwrap()
                .access_token,
            "cursor-tok"
        );
    }

    #[tokio::test]
    async fn save_updates_non_default_active_account() {
        // Regression: save() used to hardcode account_id="default", which
        // stranded refreshes when the active account was named otherwise
        // (e.g. "claude-code" from import-claude-code). Now save() must
        // overwrite the active row.
        let store = InMemoryTokenStore::new();
        store
            .save_account(
                ProviderId::Claude,
                "claude-code",
                Some("Claude Code"),
                &OAuthToken::new("imported"),
            )
            .await
            .unwrap();
        store
            .save(ProviderId::Claude, &OAuthToken::new("refreshed"))
            .await
            .unwrap();
        let accounts = store.list_accounts(ProviderId::Claude).await.unwrap();
        assert_eq!(accounts.len(), 1, "save must not create a second account");
        assert_eq!(accounts[0].account_id, "claude-code");
        assert!(accounts[0].is_active);
        assert_eq!(
            store
                .load(ProviderId::Claude)
                .await
                .unwrap()
                .unwrap()
                .access_token,
            "refreshed"
        );
    }

    // ── Multi-account tests ──────────────────────────────────────────────

    #[tokio::test]
    async fn test_multi_account_first_active() {
        let store = InMemoryTokenStore::new();
        store
            .save_account(
                ProviderId::Claude,
                "work",
                Some("Work"),
                &OAuthToken::new("w"),
            )
            .await
            .unwrap();
        store
            .save_account(ProviderId::Claude, "personal", None, &OAuthToken::new("p"))
            .await
            .unwrap();
        // First account is active.
        let loaded = store.load(ProviderId::Claude).await.unwrap().unwrap();
        assert_eq!(loaded.access_token, "w");
    }

    #[tokio::test]
    async fn test_set_active() {
        let store = InMemoryTokenStore::new();
        store
            .save_account(ProviderId::Claude, "a", None, &OAuthToken::new("tok-a"))
            .await
            .unwrap();
        store
            .save_account(ProviderId::Claude, "b", None, &OAuthToken::new("tok-b"))
            .await
            .unwrap();
        store.set_active(ProviderId::Claude, "b").await.unwrap();
        let loaded = store.load(ProviderId::Claude).await.unwrap().unwrap();
        assert_eq!(loaded.access_token, "tok-b");
    }

    #[tokio::test]
    async fn test_list_accounts() {
        let store = InMemoryTokenStore::new();
        store
            .save_account(
                ProviderId::Claude,
                "work",
                Some("Work"),
                &OAuthToken::new("w"),
            )
            .await
            .unwrap();
        store
            .save_account(
                ProviderId::Claude,
                "personal",
                Some("Personal"),
                &OAuthToken::new("p"),
            )
            .await
            .unwrap();
        let accounts = store.list_accounts(ProviderId::Claude).await.unwrap();
        assert_eq!(accounts.len(), 2);
        assert!(accounts[0].is_active);
    }

    #[tokio::test]
    async fn test_load_all_tokens() {
        let store = InMemoryTokenStore::new();
        store
            .save_account(ProviderId::Claude, "a", None, &OAuthToken::new("tok-a"))
            .await
            .unwrap();
        store
            .save_account(ProviderId::Claude, "b", None, &OAuthToken::new("tok-b"))
            .await
            .unwrap();
        let all = store.load_all_tokens(ProviderId::Claude).await.unwrap();
        assert_eq!(all.len(), 2);
    }
}
