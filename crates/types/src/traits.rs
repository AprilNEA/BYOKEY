//! Async traits shared across all byokey crates.
//!
//! Every cross-crate abstraction is defined here so that higher layers depend
//! only on `byokey-types`, not on each other.

pub use crate::error::Result;
use crate::{AccountInfo, ByokError, OAuthToken, ProviderId};
use async_trait::async_trait;
use bytes::Bytes;
use futures_core::Stream;
use std::pin::Pin;

/// A pinned, sendable stream of SSE byte chunks.
pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes>> + Send>>;

/// Default account identifier used when no explicit account is specified.
pub const DEFAULT_ACCOUNT: &str = "default";

/// Default account identifier for credentials imported from the local
/// Claude Code CLI (see `byokey_auth::provider::claude_code`).
pub const CLAUDE_CODE_ACCOUNT: &str = "claude-code";

/// Maximum byte length accepted by the `AddApiKey` RPC / CLI command.
/// Real API keys are well under 1KB; rejecting larger values guards against
/// oversized strings ending up in every outgoing `Authorization` header.
pub const MAX_API_KEY_BYTES: usize = 4096;

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

/// A single request's usage record for persistence.
#[derive(Debug, Clone)]
pub struct UsageRecord {
    pub model: String,
    pub provider: String,
    /// Account identifier. Use [`DEFAULT_ACCOUNT`] for API-key flows or when
    /// the caller can't determine the specific OAuth account that served the
    /// request.
    pub account_id: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub success: bool,
}

/// Per-model usage totals.
#[derive(Debug, Clone)]
pub struct UsageBucket {
    pub model: String,
    pub request_count: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Persistent storage for usage statistics.
#[async_trait]
pub trait UsageStore: Send + Sync {
    /// Record a single request's usage.
    async fn record(&self, record: &UsageRecord) -> Result<()>;

    /// Get cumulative totals, optionally within a time range.
    async fn totals(&self, from: Option<i64>, to: Option<i64>) -> Result<Vec<UsageBucket>>;
}
