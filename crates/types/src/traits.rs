//! Async traits shared across all byokey crates.
//!
//! Every cross-crate abstraction is defined here so that higher layers depend
//! only on `byokey-types`, not on each other.

pub use crate::error::Result;
use crate::{AccountInfo, ByokError, ChatRequest, OAuthToken, ProviderId};
use async_trait::async_trait;
use bytes::Bytes;
use futures_core::Stream;
use serde_json::Value;
use std::pin::Pin;

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

/// Summary of a stored conversation.
#[derive(Debug, Clone)]
pub struct ConversationSummary {
    pub id: String,
    pub title: Option<String>,
    pub model: String,
    pub provider: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A single message record for persistence.
#[derive(Debug, Clone)]
pub struct MessageRecord {
    pub id: String,
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub model: Option<String>,
    pub finish_reason: Option<String>,
    pub duration_ms: Option<u64>,
    pub extra: Option<Value>,
    pub created_at: i64,
}

/// Persistent storage for chat conversation history.
#[async_trait]
pub trait ChatHistoryStore: Send + Sync {
    /// Create a new conversation, returning its ID.
    async fn create_conversation(
        &self,
        id: &str,
        model: &str,
        provider: &str,
        title: Option<&str>,
    ) -> Result<()>;

    /// Append a message to an existing conversation.
    async fn append_message(&self, msg: &MessageRecord) -> Result<()>;

    /// List recent conversations, newest first.
    async fn list_conversations(&self, limit: u64, offset: u64)
    -> Result<Vec<ConversationSummary>>;

    /// Load all messages for a conversation, ordered by `created_at`.
    async fn get_messages(&self, conversation_id: &str) -> Result<Vec<MessageRecord>>;

    /// Delete a conversation and its messages.
    async fn delete_conversation(&self, conversation_id: &str) -> Result<()>;
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

/// Time-bucketed usage aggregation result.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct UsageBucket {
    pub period_start: i64,
    pub model: String,
    pub request_count: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Per-(provider, account, model) cumulative usage for a time range.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct AccountUsageTotal {
    pub provider: String,
    pub account_id: String,
    pub model: String,
    pub request_count: u64,
    pub success_count: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Persistent storage for usage statistics.
#[async_trait]
pub trait UsageStore: Send + Sync {
    /// Record a single request's usage.
    async fn record(&self, record: &UsageRecord) -> Result<()>;

    /// Query aggregated usage within a time range, optionally filtered by model.
    /// Results are grouped into time buckets whose size is determined by the range.
    async fn query(
        &self,
        from: i64,
        to: i64,
        model: Option<&str>,
        bucket_secs: i64,
    ) -> Result<Vec<UsageBucket>>;

    /// Get cumulative totals, optionally within a time range.
    async fn totals(&self, from: Option<i64>, to: Option<i64>) -> Result<Vec<UsageBucket>>;

    /// Cumulative usage grouped by (provider, `account_id`, model), optionally
    /// within a time range.
    ///
    /// Returns an empty vector by default; backends that track `account_id`
    /// override this.
    async fn totals_by_account(
        &self,
        _from: Option<i64>,
        _to: Option<i64>,
    ) -> Result<Vec<AccountUsageTotal>> {
        Ok(Vec::new())
    }
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
