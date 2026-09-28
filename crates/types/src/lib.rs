//! Core types and traits for the byokey workspace.
//!
//! This crate defines the shared abstractions used across all layers of the
//! byokey proxy gateway, including error types, provider identifiers, OAuth token
//! representations, and the async traits that each layer implements.

pub mod copilot;
pub mod error;
pub mod model;
pub mod provider;
pub mod token;
pub mod traits;

pub use copilot::CopilotClient;
pub use error::{ByokError, Result};
pub use model::{ClaudeFamily, ClaudeModel, ParsedId, UnknownFamily};
pub use provider::ProviderId;
pub use provider::ThinkingCapability;
pub use token::{AccountInfo, AccountToken, OAuthToken, TokenState};
pub use traits::{
    ByteStream, CLAUDE_CODE_ACCOUNT, DEFAULT_ACCOUNT, MAX_API_KEY_BYTES, TokenStore, Usage,
    UsageBucket, UsageRecord, UsageStore,
};

/// `duration` in whole milliseconds, as BYOKEY's log lines report durations
/// (`duration_ms`, `first_byte_ms`): a `u64` stays a number in every log
/// format, where the JSON one would turn a `u128` into a string.
#[must_use]
pub fn millis(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
