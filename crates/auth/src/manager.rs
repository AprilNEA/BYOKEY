//! Unified manager for OAuth token lifecycles across all providers.
//!
//! Responsibilities:
//! - Load tokens from a [`TokenStore`].
//! - Detect expiration and trigger refresh via the provider's token endpoint.
//! - Cooldown duration to prevent excessive refresh attempts (30 s).
//! - Multi-account support: save, switch, and list accounts per provider.
use byokey_types::{
    AccountInfo, ByokError, OAuthToken, ProviderId, Result, TokenState, TokenStore,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Mutex as TokioMutex;

use crate::{credentials, provider::iflow, token};

const REFRESH_COOLDOWN: Duration = Duration::from_secs(30);

struct ProviderState {
    last_refresh_attempt: Option<Instant>,
}

pub struct AuthManager {
    store: Arc<dyn TokenStore>,
    http: rquest::Client,
    state: Mutex<HashMap<ProviderId, ProviderState>>,
    /// Per-provider async locks to deduplicate concurrent refresh attempts.
    refresh_locks: Mutex<HashMap<ProviderId, Arc<TokioMutex<()>>>>,
}

impl AuthManager {
    pub fn new(store: Arc<dyn TokenStore>, http: rquest::Client) -> Self {
        Self {
            store,
            http,
            state: Mutex::new(HashMap::new()),
            refresh_locks: Mutex::new(HashMap::new()),
        }
    }

    /// Return (or create) the per-provider async mutex used for refresh dedup.
    fn get_refresh_lock(&self, provider: &ProviderId) -> Arc<TokioMutex<()>> {
        let mut locks = self.refresh_locks.lock().unwrap();
        locks
            .entry(provider.clone())
            .or_insert_with(|| Arc::new(TokioMutex::new(())))
            .clone()
    }

    // ── Active-account methods (backward-compatible) ─────────────────────

    /// Retrieve a valid token for the active account, attempting a refresh if expired.
    ///
    /// When the token is still valid but approaching expiry (within 5 minutes),
    /// a background refresh is spawned so that subsequent requests get a fresh
    /// token without blocking.
    ///
    /// # Errors
    ///
    /// Returns an error if the token is not found, expired and cannot be refreshed, or invalid.
    pub async fn get_token(self: &Arc<Self>, provider: &ProviderId) -> Result<OAuthToken> {
        let token = self
            .store
            .load(provider)
            .await?
            .ok_or_else(|| ByokError::TokenNotFound(provider.clone()))?;

        match token.state() {
            TokenState::Valid => {
                // Proactive refresh: spawn a background task if the token is
                // nearing expiry but still usable, so the next caller gets a
                // fresh token without waiting.
                if token.should_proactive_refresh() && self.should_spawn_proactive_refresh(provider)
                {
                    let this = Arc::clone(self);
                    let provider = provider.clone();
                    let token = token.clone();
                    tokio::spawn(async move {
                        if let Err(e) = this.refresh_token(&provider, &token).await {
                            tracing::debug!(%provider, %e, "proactive refresh failed (non-critical)");
                        }
                    });
                }
                Ok(token)
            }
            TokenState::Expired => self.refresh_token(provider, &token).await,
            TokenState::Invalid => Err(ByokError::TokenExpired(provider.clone())),
        }
    }

    /// Check whether the provider is authenticated (active account token exists and is not invalid).
    pub async fn is_authenticated(&self, provider: &ProviderId) -> bool {
        match self.store.load(provider).await {
            Ok(Some(t)) => t.state() != TokenState::Invalid,
            _ => false,
        }
    }

    /// Return the current [`TokenState`] for the active account.
    ///
    /// Returns [`TokenState::Invalid`] if the token is missing or the store fails.
    pub async fn token_state(&self, provider: &ProviderId) -> TokenState {
        match self.store.load(provider).await {
            Ok(Some(t)) => t.state(),
            _ => TokenState::Invalid,
        }
    }

    /// Save a new token for the active account (backward-compatible shortcut).
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying store fails to persist the token.
    pub async fn save_token(&self, provider: &ProviderId, token: OAuthToken) -> Result<()> {
        self.store.save(provider, &token).await
    }

    /// Remove the active account's token (logout).
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying store fails to remove the token.
    pub async fn remove_token(&self, provider: &ProviderId) -> Result<()> {
        self.store.remove(provider).await
    }

    // ── Multi-account methods ────────────────────────────────────────────

    /// Save a token for a specific account.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying store fails to persist the token.
    pub async fn save_token_for(
        &self,
        provider: &ProviderId,
        account_id: &str,
        label: Option<&str>,
        token: OAuthToken,
    ) -> Result<()> {
        self.store
            .save_account(provider, account_id, label, &token)
            .await
    }

    /// Retrieve a valid token for a specific account.
    ///
    /// # Errors
    ///
    /// Returns an error if the token is not found, expired, or invalid.
    pub async fn get_token_for(
        self: &Arc<Self>,
        provider: &ProviderId,
        account_id: &str,
    ) -> Result<OAuthToken> {
        let token = self
            .store
            .load_account(provider, account_id)
            .await?
            .ok_or_else(|| ByokError::TokenNotFound(provider.clone()))?;

        match token.state() {
            TokenState::Valid => {
                if token.should_proactive_refresh() && self.should_spawn_proactive_refresh(provider)
                {
                    let this = Arc::clone(self);
                    let provider = provider.clone();
                    let token = token.clone();
                    tokio::spawn(async move {
                        if let Err(e) = this.refresh_token(&provider, &token).await {
                            tracing::debug!(%provider, %e, "proactive refresh failed (non-critical)");
                        }
                    });
                }
                Ok(token)
            }
            TokenState::Expired => self.refresh_token(provider, &token).await,
            TokenState::Invalid => Err(ByokError::TokenExpired(provider.clone())),
        }
    }

    /// Remove a specific account's token.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying store fails.
    pub async fn remove_token_for(&self, provider: &ProviderId, account_id: &str) -> Result<()> {
        self.store.remove_account(provider, account_id).await
    }

    /// List all accounts for a provider.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying store fails.
    pub async fn list_accounts(&self, provider: &ProviderId) -> Result<Vec<AccountInfo>> {
        self.store.list_accounts(provider).await
    }

    /// Switch the active account for a provider.
    ///
    /// # Errors
    ///
    /// Returns an error if the account does not exist or the store fails.
    pub async fn set_active_account(&self, provider: &ProviderId, account_id: &str) -> Result<()> {
        self.store.set_active(provider, account_id).await
    }

    /// Retrieve a valid token alongside the account ID that served it.
    ///
    /// Reads the active-account list once and uses that snapshot for both the
    /// account ID and the token fetch, preventing a TOCTOU race where the
    /// active account changes between the two reads.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`get_token_for`](Self::get_token_for), or
    /// falls back to the default-account path when no named accounts exist.
    pub async fn get_token_with_account(
        self: &Arc<Self>,
        provider: &ProviderId,
    ) -> Result<(String, OAuthToken)> {
        // Single store call establishes the account snapshot. If any account
        // is marked active, use it. Otherwise, if accounts exist but none is
        // active, use the first one (logged as a warning — set_active_account
        // should have enforced the invariant). Only fall through to the
        // default-account path when the list is truly empty.
        let account_id = match self.store.list_accounts(provider).await {
            Ok(accts) if !accts.is_empty() => {
                if let Some(active) = accts.iter().find(|a| a.is_active) {
                    Some(active.account_id.clone())
                } else {
                    let fallback = accts[0].account_id.clone();
                    tracing::warn!(
                        provider = ?provider,
                        account_id = %fallback,
                        "no active account marked; defaulting to first account in the list"
                    );
                    Some(fallback)
                }
            }
            Ok(_) => None,
            Err(e) => {
                tracing::warn!(
                    provider = ?provider,
                    error = ?e,
                    "list_accounts failed in get_token_with_account; falling back to default account"
                );
                None
            }
        };

        if let Some(account_id) = account_id {
            // Load the token for the specific account we just identified —
            // no second list_accounts call, so the pair is consistent.
            let token = self.get_token_for(provider, &account_id).await?;
            Ok((account_id, token))
        } else {
            // No named accounts; fall through to the default-account path.
            let token = self.get_token(provider).await?;
            Ok((byokey_types::DEFAULT_ACCOUNT.to_string(), token))
        }
    }

    /// Load all tokens for a provider (for round-robin rotation).
    ///
    /// # Errors
    ///
    /// Returns an error if the store fails.
    pub async fn get_all_tokens(&self, provider: &ProviderId) -> Result<Vec<(String, OAuthToken)>> {
        self.store.load_all_tokens(provider).await
    }

    // ── Background refresh ────────────────────────────────────────────────

    /// Spawns a background loop that periodically checks all providers and
    /// proactively refreshes tokens approaching expiry.
    ///
    /// The loop runs every `interval` and refreshes any token that would
    /// expire within `lead_time`. Tokens without a `refresh_token`, or for
    /// providers that don't support refresh (Copilot, Kiro), are skipped.
    ///
    /// Returns a [`tokio::task::JoinHandle`] that can be used to abort the loop.
    pub fn spawn_refresh_loop(
        self: &Arc<Self>,
        interval: Duration,
        lead_time: Duration,
    ) -> tokio::task::JoinHandle<()> {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                this.refresh_due_tokens(lead_time).await;
            }
        })
    }

    /// Scans all providers and refreshes tokens expiring within `lead_time`.
    async fn refresh_due_tokens(self: &Arc<Self>, lead_time: Duration) {
        for provider in ProviderId::all() {
            // Skip providers that don't support refresh.
            if matches!(provider, ProviderId::Copilot | ProviderId::Kiro) {
                continue;
            }

            let Ok(Some(token)) = self.store.load(provider).await else {
                continue;
            };

            // Skip tokens without a refresh_token.
            if token.refresh_token.is_none() {
                continue;
            }

            // Check if token expires within lead_time.
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let expires_at = token.expires_at.unwrap_or(u64::MAX);
            if expires_at > now + lead_time.as_secs() {
                // Not yet due for refresh.
                continue;
            }
            if token.is_expired() {
                // Already expired — on-demand refresh handles this.
                continue;
            }

            tracing::debug!(%provider, "auto-refresh: token nearing expiry, refreshing");
            if let Err(e) = self.refresh_token(provider, &token).await {
                tracing::debug!(%provider, %e, "auto-refresh failed");
            }
        }
    }

    // ── Private helpers ──────────────────────────────────────────────────

    /// Check whether a proactive (background) refresh should be spawned.
    /// Returns `false` if a refresh was attempted within the cooldown period,
    /// avoiding redundant background tasks.
    fn should_spawn_proactive_refresh(&self, provider: &ProviderId) -> bool {
        let state = self.state.lock().unwrap();
        state.get(provider).is_none_or(|ps| {
            ps.last_refresh_attempt
                .is_none_or(|last| last.elapsed() >= REFRESH_COOLDOWN)
        })
    }

    async fn refresh_token(&self, provider: &ProviderId, token: &OAuthToken) -> Result<OAuthToken> {
        // Acquire the per-provider async lock so that concurrent callers
        // coalesce into a single refresh round-trip.
        let lock = self.get_refresh_lock(provider);
        let _guard = lock.lock().await;

        // Re-check: another task may have completed the refresh while we
        // were waiting for the lock.
        if let Ok(Some(current)) = self.store.load(provider).await
            && current.expires_at != token.expires_at
            && !current.is_expired()
        {
            return Ok(current);
        }

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

        let refresh_token = token
            .refresh_token
            .as_deref()
            .ok_or_else(|| ByokError::Auth(format!("no refresh_token for {provider}")))?;

        // Copilot tokens don't expire; Kiro login is not yet implemented.
        if matches!(provider, ProviderId::Copilot | ProviderId::Kiro) {
            return Err(ByokError::Auth(format!(
                "token refresh not supported for {provider}; please re-authenticate"
            )));
        }

        // Fetch credentials (client_id, client_secret, token_url) from CDN.
        let provider_name = provider.to_string();
        let creds = credentials::fetch(&provider_name, &self.http).await?;
        let token_url = creds.token_url.as_deref().ok_or_else(|| {
            ByokError::Auth(format!("no token_url in credentials for {provider}"))
        })?;

        // Build the refresh request.
        let refresh_result = if *provider == ProviderId::IFlow {
            self.refresh_iflow(&creds, token_url, refresh_token).await
        } else {
            self.refresh_standard(&creds, token_url, refresh_token)
                .await
        };

        let new_token = match refresh_result {
            Ok(t) => t,
            Err(ByokError::Auth(ref msg)) if msg.starts_with("invalid_grant:") => {
                tracing::error!(%provider, "refresh token revoked or expired — user must re-authenticate");
                if let Err(e) = self.store.remove(provider).await {
                    tracing::warn!(%provider, error = %e, "failed to remove revoked token from store");
                }
                return Err(ByokError::TokenExpired(provider.clone()));
            }
            Err(e) => return Err(e),
        };

        // Preserve the old refresh_token if the response didn't include a new one
        // (Google OAuth typically does not return a new refresh_token on refresh).
        let new_token = if new_token.refresh_token.is_none() {
            OAuthToken {
                refresh_token: token.refresh_token.clone(),
                ..new_token
            }
        } else {
            new_token
        };

        self.store.save(provider, &new_token).await?;
        tracing::info!(%provider, "token refreshed successfully");
        Ok(new_token)
    }

    /// Standard `OAuth2` refresh: POST form with `grant_type=refresh_token`.
    async fn refresh_standard(
        &self,
        creds: &credentials::OAuthCredentials,
        token_url: &str,
        refresh_token: &str,
    ) -> Result<OAuthToken> {
        let mut params = vec![
            ("grant_type", "refresh_token"),
            ("client_id", creds.client_id.as_str()),
            ("refresh_token", refresh_token),
        ];
        // Providers with a client_secret (Gemini, Antigravity) include it in the form.
        let secret_ref;
        if let Some(secret) = &creds.client_secret {
            secret_ref = secret.clone();
            params.push(("client_secret", &secret_ref));
        }

        let resp = self
            .http
            .post(token_url)
            .header("Accept", "application/json")
            .form(&params)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ByokError::Auth(format!("failed to parse refresh response: {e}")))?;

        if !status.is_success() {
            let error_code = json
                .get("error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let error_desc = json
                .get("error_description")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown error");
            if error_code == "invalid_grant" {
                return Err(ByokError::Auth(format!("invalid_grant: {error_desc}")));
            }
            return Err(ByokError::Auth(format!(
                "refresh failed ({status}): {error_desc}"
            )));
        }

        token::parse_token_response(&json)
    }

    /// iFlow-specific refresh: uses Basic Auth header and exchanges the new
    /// OAuth `access_token` for an API key via `fetch_api_key`.
    async fn refresh_iflow(
        &self,
        creds: &credentials::OAuthCredentials,
        token_url: &str,
        refresh_token: &str,
    ) -> Result<OAuthToken> {
        let client_secret = creds
            .client_secret
            .as_deref()
            .ok_or_else(|| ByokError::Auth("iflow credentials missing client_secret".into()))?;

        let params = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ];

        let resp = self
            .http
            .post(token_url)
            .header(
                "Authorization",
                iflow::basic_auth_header(&creds.client_id, client_secret),
            )
            .form(&params)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ByokError::Auth(format!("failed to parse refresh response: {e}")))?;

        if !status.is_success() {
            let error_code = json
                .get("error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let error_desc = json
                .get("error_description")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown error");
            if error_code == "invalid_grant" {
                return Err(ByokError::Auth(format!("invalid_grant: {error_desc}")));
            }
            return Err(ByokError::Auth(format!(
                "iflow refresh failed ({status}): {error_desc}"
            )));
        }

        let tok = token::parse_token_response(&json)?;

        // Exchange the new OAuth access_token for an iFlow API key.
        let api_key = iflow::fetch_api_key(&tok.access_token, &self.http).await?;
        Ok(OAuthToken {
            access_token: api_key,
            ..tok
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use byokey_store::InMemoryTokenStore;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_manager() -> Arc<AuthManager> {
        Arc::new(AuthManager::new(
            Arc::new(InMemoryTokenStore::new()),
            rquest::Client::new(),
        ))
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

    // ── Multi-account tests ──────────────────────────────────────────────

    #[tokio::test]
    async fn test_save_and_get_token_for() {
        let m = make_manager();
        m.save_token_for(
            &ProviderId::Claude,
            "work",
            Some("Work Account"),
            OAuthToken::new("work-tok").with_expiry(3600),
        )
        .await
        .unwrap();
        let tok = m.get_token_for(&ProviderId::Claude, "work").await.unwrap();
        assert_eq!(tok.access_token, "work-tok");
    }

    #[tokio::test]
    async fn test_list_accounts() {
        let m = make_manager();
        m.save_token_for(
            &ProviderId::Claude,
            "a",
            Some("Account A"),
            OAuthToken::new("a"),
        )
        .await
        .unwrap();
        m.save_token_for(&ProviderId::Claude, "b", None, OAuthToken::new("b"))
            .await
            .unwrap();
        let accounts = m.list_accounts(&ProviderId::Claude).await.unwrap();
        assert_eq!(accounts.len(), 2);
    }

    #[tokio::test]
    async fn test_set_active_account() {
        let m = make_manager();
        m.save_token_for(&ProviderId::Claude, "a", None, OAuthToken::new("tok-a"))
            .await
            .unwrap();
        m.save_token_for(&ProviderId::Claude, "b", None, OAuthToken::new("tok-b"))
            .await
            .unwrap();
        m.set_active_account(&ProviderId::Claude, "b")
            .await
            .unwrap();
        // Active-account shortcut now returns "b".
        let tok = m.get_token(&ProviderId::Claude).await.unwrap();
        assert_eq!(tok.access_token, "tok-b");
    }

    #[tokio::test]
    async fn test_remove_token_for() {
        let m = make_manager();
        m.save_token_for(&ProviderId::Claude, "work", None, OAuthToken::new("w"))
            .await
            .unwrap();
        m.remove_token_for(&ProviderId::Claude, "work")
            .await
            .unwrap();
        let err = m
            .get_token_for(&ProviderId::Claude, "work")
            .await
            .unwrap_err();
        assert!(matches!(err, ByokError::TokenNotFound(_)));
    }

    #[tokio::test]
    async fn test_get_all_tokens() {
        let m = make_manager();
        m.save_token_for(&ProviderId::Claude, "a", None, OAuthToken::new("tok-a"))
            .await
            .unwrap();
        m.save_token_for(&ProviderId::Claude, "b", None, OAuthToken::new("tok-b"))
            .await
            .unwrap();
        let all = m.get_all_tokens(&ProviderId::Claude).await.unwrap();
        assert_eq!(all.len(), 2);
    }
}
