//! Async traits shared across all byokey crates.
//!
//! Every cross-crate abstraction is defined here so that higher layers depend
//! only on `byokey-types`, not on each other.

use crate::{AccountInfo, ByokError, ChatRequest, OAuthToken, ProviderId};
use async_trait::async_trait;
use bytes::Bytes;
use futures_core::Stream;
use serde_json::Value;
use std::pin::Pin;

/// Convenience alias used throughout the workspace.
pub type Result<T> = std::result::Result<T, ByokError>;

/// A pinned, sendable stream of SSE byte chunks.
pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes>> + Send>>;

/// Default account identifier used when no explicit account is specified.
pub const DEFAULT_ACCOUNT: &str = "default";

/// Persistent storage for OAuth tokens, keyed by `(provider, account_id)`.
///
/// The basic `load`/`save`/`remove` methods operate on the **active** account
/// for a provider, preserving backward compatibility with single-account usage.
#[async_trait]
pub trait TokenStore: Send + Sync {
    // ── Active-account shortcuts (backward-compatible) ────────────────────

    /// Load the token for the active account of the given provider.
    async fn load(&self, provider: &ProviderId) -> Result<Option<OAuthToken>>;
    /// Persist a token for the active account of the given provider.
    async fn save(&self, provider: &ProviderId, token: &OAuthToken) -> Result<()>;
    /// Remove the active account's token for the given provider.
    async fn remove(&self, provider: &ProviderId) -> Result<()>;

    // ── Multi-account operations ──────────────────────────────────────────

    /// Load a token for a specific account.
    async fn load_account(
        &self,
        provider: &ProviderId,
        account_id: &str,
    ) -> Result<Option<OAuthToken>> {
        if account_id == DEFAULT_ACCOUNT {
            return self.load(provider).await;
        }
        Err(ByokError::Storage(
            "multi-account not supported by this store".into(),
        ))
    }

    /// Persist a token for a specific account, optionally with a label.
    async fn save_account(
        &self,
        provider: &ProviderId,
        account_id: &str,
        label: Option<&str>,
        token: &OAuthToken,
    ) -> Result<()> {
        let _ = label;
        if account_id == DEFAULT_ACCOUNT {
            return self.save(provider, token).await;
        }
        Err(ByokError::Storage(
            "multi-account not supported by this store".into(),
        ))
    }

    /// Remove a specific account's token.
    async fn remove_account(&self, provider: &ProviderId, account_id: &str) -> Result<()> {
        if account_id == DEFAULT_ACCOUNT {
            return self.remove(provider).await;
        }
        Err(ByokError::Storage(
            "multi-account not supported by this store".into(),
        ))
    }

    /// List all accounts for a provider.
    async fn list_accounts(&self, _provider: &ProviderId) -> Result<Vec<AccountInfo>> {
        Ok(Vec::new())
    }

    /// Set a specific account as the active one for a provider.
    async fn set_active(&self, _provider: &ProviderId, _account_id: &str) -> Result<()> {
        Err(ByokError::Storage(
            "multi-account not supported by this store".into(),
        ))
    }

    /// Load all valid tokens for a provider (for round-robin rotation).
    async fn load_all_tokens(&self, _provider: &ProviderId) -> Result<Vec<(String, OAuthToken)>> {
        Ok(Vec::new())
    }
}

/// Acquires and refreshes OAuth tokens for a single provider.
#[async_trait]
pub trait TokenProvider: Send + Sync {
    /// Obtain a valid access token, performing an OAuth flow if necessary.
    async fn get_token(&self) -> Result<OAuthToken>;
    /// Force-refresh the current token using the stored refresh token.
    async fn refresh(&self) -> Result<OAuthToken>;
}

/// Translates an `OpenAI`-format request into a provider's native format.
///
/// Implementations must be pure (no I/O).
pub trait RequestTranslator: Send + Sync {
    /// Convert an `OpenAI`-compatible JSON request body to the provider's format.
    ///
    /// # Errors
    ///
    /// Returns [`ByokError::Translation`] if the request cannot be translated.
    fn translate_request(&self, req: Value) -> Result<Value>;
}

/// Translates a provider's native response back to `OpenAI` format.
///
/// Implementations must be pure (no I/O).
pub trait ResponseTranslator: Send + Sync {
    /// Convert a provider-native JSON response body to `OpenAI` format.
    ///
    /// # Errors
    ///
    /// Returns [`ByokError::Translation`] if the response cannot be translated.
    fn translate_response(&self, res: Value) -> Result<Value>;
}

/// The response produced by a [`ProviderExecutor`].
pub enum ProviderResponse {
    /// A complete, non-streaming JSON response.
    Complete(Value),
    /// A streaming SSE byte stream.
    Stream(ByteStream),
}

/// Executes chat-completion requests against an upstream provider.
#[async_trait]
pub trait ProviderExecutor: Send + Sync {
    /// Send a chat-completion request and return the response.
    async fn chat_completion(&self, request: ChatRequest) -> Result<ProviderResponse>;
    /// List the model identifiers supported by this provider.
    fn supported_models(&self) -> Vec<String>;
}
