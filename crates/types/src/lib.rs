//! Core types and traits for the byokey workspace.
//!
//! This crate defines the shared abstractions used across all layers of the
//! byokey proxy gateway, including error types, provider identifiers, OAuth token
//! representations, and the async traits that each layer implements.

pub mod chat;
pub mod error;
pub mod provider;
pub mod token;
pub mod traits;

pub use chat::ChatRequest;
pub use error::ByokError;
pub use provider::{ProtocolFormat, ProviderId};
pub use token::{AccountInfo, OAuthToken, TokenState};
pub use traits::{
    ByteStream, ProviderExecutor, ProviderResponse, RequestTranslator, ResponseTranslator,
    TokenStore,
};
