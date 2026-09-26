//! Provider-specific OAuth configuration and parameter formatting.
//!
//! Each sub-module defines constants (endpoints, ports, scopes) and
//! provides URL building / parameter formatting functions for its provider.
//! Token response parsing is handled by [`crate::token::parse_token_response`].

pub mod claude;
pub mod claude_code;
pub mod copilot;
pub mod cursor;
