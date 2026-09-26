//! Core types and traits for the byokey workspace.
//!
//! This crate defines the shared abstractions used across all layers of the
//! byokey proxy gateway, including error types, provider identifiers, OAuth token
//! representations, and the async traits that each layer implements.

pub mod copilot;
pub mod error;
pub mod provider;
pub mod token;
pub mod traits;

pub use copilot::CopilotClient;
pub use error::{ByokError, Result};
pub use provider::ProviderId;
pub use provider::ThinkingCapability;
pub use token::{AccountInfo, OAuthToken, TokenState};
pub use traits::{
    ByteStream, CLAUDE_CODE_ACCOUNT, DEFAULT_ACCOUNT, MAX_API_KEY_BYTES, TokenStore, UsageBucket,
    UsageRecord, UsageStore,
};
