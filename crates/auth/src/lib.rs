//! OAuth authentication flows for all supported providers.
//!
//! Each sub-module implements provider-specific URL building, token exchange
//! parameters, and response parsing. The [`AuthManager`] coordinates token
//! lifecycle across providers.

pub mod antigravity;
pub mod callback;
pub mod claude;
pub mod credentials;
pub mod codex;
pub mod copilot;
pub mod flow;
pub mod gemini;
pub mod iflow;
pub mod kimi;
pub mod kiro;
pub mod manager;
pub mod pkce;
pub mod qwen;

pub use manager::AuthManager;
