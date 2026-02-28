//! Unified manager for OAuth token lifecycles across all providers.
//!
//! Responsibilities:
//! - Load tokens from a [`TokenStore`].
//! - Detect expiration and trigger refresh via the provider's token endpoint.
//! - Cooldown duration to prevent excessive refresh attempts (30 s).
//! - Multi-account support: save, switch, and list accounts per provider.
use byokey_types::{
    AccountInfo, ByokError, OAuthToken, ProviderId, TokenState, TokenStore, traits::Result,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use crate::{credentials, iflow};

const REFRESH_COOLDOWN: Duration = Duration::from_secs(30);

struct ProviderState {
    last_refresh_attempt: Option<Instant>,
}

pub struct AuthManager {
    store: Arc<dyn TokenStore>,
    http: rquest::Client,
    state: Mutex<HashMap<ProviderId, ProviderState>>,
}

impl AuthManager {
    pub fn new(store: Arc<dyn TokenStore>, http: rquest::Client) -> Self {
        Self {
            store,
            http,
            state: Mutex::new(HashMap::new()),
        }
    }

    // ── Active-account methods (backward-compatible) ─────────────────────

    /// Retrieve a valid token for the active account, attempting a refresh if expired.
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
        &self,
        provider: &ProviderId,
        account_id: &str,
    ) -> Result<OAuthToken> {
        let token = self
            .store
            .load_account(provider, account_id)
            .await?
            .ok_or_else(|| ByokError::TokenNotFound(provider.clone()))?;

        match token.state() {
            TokenState::Valid => Ok(token),
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

    /// Load all tokens for a provider (for round-robin rotation).
    ///
    /// # Errors
    ///
    /// Returns an error if the store fails.
    pub async fn get_all_tokens(&self, provider: &ProviderId) -> Result<Vec<(String, OAuthToken)>> {
        self.store.load_all_tokens(provider).await
    }

    // ── Private helpers ──────────────────────────────────────────────────

    async fn refresh_token(&self, provider: &ProviderId, token: &OAuthToken) -> Result<OAuthToken> {
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

        // Factory uses WorkOS token endpoint directly (no CDN credentials needed).
        if *provider == ProviderId::Factory {
            return self.refresh_factory(token, refresh_token).await;
        }

        // Fetch credentials (client_id, client_secret, token_url) from CDN.
        let provider_name = provider.to_string();
        let creds = credentials::fetch(&provider_name, &self.http).await?;
        let token_url = creds.token_url.as_deref().ok_or_else(|| {
            ByokError::Auth(format!("no token_url in credentials for {provider}"))
        })?;

        // Build the refresh request.
        let new_token = if *provider == ProviderId::IFlow {
            self.refresh_iflow(&creds, token_url, refresh_token).await?
        } else {
            self.refresh_standard(&creds, token_url, refresh_token)
                .await?
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
            let error_desc = json
                .get("error_description")
                .or_else(|| json.get("error"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown error");
            return Err(ByokError::Auth(format!(
                "refresh failed ({status}): {error_desc}"
            )));
        }

        parse_token_response(&json)
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
            let error_desc = json
                .get("error_description")
                .or_else(|| json.get("error"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown error");
            return Err(ByokError::Auth(format!(
                "iflow refresh failed ({status}): {error_desc}"
            )));
        }

        let token = parse_token_response(&json)?;

        // Exchange the new OAuth access_token for an iFlow API key.
        let api_key = iflow::fetch_api_key(&token.access_token, &self.http).await?;
        Ok(OAuthToken {
            access_token: api_key,
            ..token
        })
    }

    /// Factory-specific refresh via `WorkOS`: POST form with `grant_type=refresh_token`
    /// and the Factory `CLIENT_ID`. No `client_secret` is needed.
    async fn refresh_factory(&self, token: &OAuthToken, refresh_token: &str) -> Result<OAuthToken> {
        use crate::factory;

        let params = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", factory::CLIENT_ID),
        ];

        let resp = self
            .http
            .post(factory::TOKEN_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&params)
            .send()
            .await?;

        let status = resp.status();
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ByokError::Auth(format!("failed to parse refresh response: {e}")))?;

        if !status.is_success() {
            let error_desc = json
                .get("error_description")
                .or_else(|| json.get("error"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown error");
            return Err(ByokError::Auth(format!(
                "Factory refresh failed ({status}): {error_desc}"
            )));
        }

        let mut new_token = factory::parse_token_response(&json)?;

        // Preserve the old refresh_token if the response didn't include a new one.
        if new_token.refresh_token.is_none() {
            new_token.refresh_token = token.refresh_token.clone();
        }

        self.store.save(&ProviderId::Factory, &new_token).await?;
        tracing::info!("Factory token refreshed successfully");
        Ok(new_token)
    }
}

/// Parse a standard OAuth token response JSON into an [`OAuthToken`].
fn parse_token_response(json: &serde_json::Value) -> Result<OAuthToken> {
    let access_token = json
        .get("access_token")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ByokError::Auth("missing access_token in refresh response".into()))?
        .to_string();

    let mut token = OAuthToken::new(access_token);
    if let Some(r) = json
        .get("refresh_token")
        .and_then(serde_json::Value::as_str)
    {
        token = token.with_refresh(r);
    }
    if let Some(exp) = json.get("expires_in").and_then(serde_json::Value::as_u64) {
        token = token.with_expiry(exp);
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use byokey_store::InMemoryTokenStore;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_manager() -> AuthManager {
        AuthManager::new(Arc::new(InMemoryTokenStore::new()), rquest::Client::new())
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
