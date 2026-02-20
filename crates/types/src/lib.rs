//! Core types and traits for the byok workspace.
//!
//! This crate defines the shared abstractions used across all layers of the
//! byok proxy gateway, including error types, provider identifiers, OAuth token
//! representations, and the async traits that each layer implements.

pub mod error;
pub mod provider;
pub mod token;
pub mod traits;

pub use error::ByokError;
pub use provider::{ProtocolFormat, ProviderId};
pub use token::{OAuthToken, TokenState};
pub use traits::{
    ByteStream, ProviderExecutor, ProviderResponse, RequestTranslator, ResponseTranslator,
    TokenProvider, TokenStore,
};
