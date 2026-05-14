//! Provider-specific OAuth configuration and parameter formatting.
//!
//! Each sub-module defines constants (endpoints, ports, scopes) and
//! provides URL building / parameter formatting functions for its provider.
//! Token response parsing is handled by [`crate::token::parse_token_response`].

pub mod amp;
pub mod antigravity;
pub mod claude;
pub mod claude_code;
pub mod codex;
pub mod codex_cli;
pub mod copilot;
pub mod gemini;
pub mod iflow;
pub mod kimi;
pub mod kiro;
pub mod qwen;
