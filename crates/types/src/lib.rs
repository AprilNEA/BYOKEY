//! Core types and traits for the byokey workspace.
//!
//! This crate defines the shared abstractions used across all layers of the
//! byokey proxy gateway, including error types, provider identifiers, OAuth token
//! representations, and the async traits that each layer implements.

pub mod chat;
pub mod error;
pub mod provider;
pub mod ratelimit;
pub mod token;
pub mod traits;

pub use chat::ChatRequest;
pub use error::{ByokError, Result};
pub use provider::ProviderId;
pub use provider::ThinkingCapability;
pub use ratelimit::{RateLimitSnapshot, RateLimitStore};
pub use token::{AccountInfo, OAuthToken, TokenState};
pub use traits::{
    AccountUsageTotal, ByteStream, CLAUDE_CODE_ACCOUNT, CODEX_CLI_ACCOUNT, ChatHistoryStore,
    ConversationSummary, DEFAULT_ACCOUNT, MAX_API_KEY_BYTES, MessageRecord, ProviderExecutor,
    ProviderResponse, RequestTranslator, ResponseTranslator, TokenStore, UsageBucket, UsageRecord,
    UsageStore,
};
