//! Unofficial Rust client for [Ampcode](https://ampcode.com) — Anthropic's
//! agentic coding tool.
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use ampcode::AmpcodeClient;
//!
//! # async fn example() -> ampcode::error::Result<()> {
//! let token = ampcode::secrets::load_token().await?;
//! let client = AmpcodeClient::new(token);
//! let balance = client.balance().await?;
//! println!("{}", balance.display_text);
//! # Ok(())
//! # }
//! ```
//!
//! ## Token Sources
//!
//! Tokens can come from:
//! - The Amp CLI secrets file at `~/.local/share/amp/secrets.json` via
//!   [`secrets::load_token`].
//! - An `OAuth2` access token (e.g. from BYOKEY's auth flow) — pass directly
//!   as a `String`.
//! - A custom [`TokenProvider`] impl for dynamic token refresh.
//!
//! ## Local Thread Access
//!
//! Read Amp CLI thread files from disk without network access:
//!
//! ```rust,no_run
//! # async fn example() -> ampcode::error::Result<()> {
//! let summaries = ampcode::local::list_thread_summaries().await?;
//! for s in &summaries {
//!     println!("{}: {}", s.id, s.title.as_deref().unwrap_or("(untitled)"));
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Features
//!
//! - `rustls-tls` *(default)* — use rustls for TLS.
//! - `native-tls` — use the OS native TLS stack instead.

#![allow(clippy::module_name_repetitions)]

pub mod client;
pub mod error;
pub mod local;
pub mod secrets;
pub mod types;

pub use client::AmpcodeClient;
pub use error::AmpcodeError;
pub use types::balance::{BalanceInfo, Plan};
pub use types::thread::{
    ContentBlock, Message, MessageState, Relationship, Thread, ThreadSummary, ToolRun, Usage,
};

/// Dynamic token provider for cases where tokens may be refreshed mid-session.
///
/// The simplest usage is a plain `String` (via the blanket impl). For
/// OAuth-backed setups, implement this trait on a type that provides the
/// current access token synchronously (handle refresh externally).
pub trait TokenProvider: Send + Sync {
    /// Return the current bearer token (without the `Bearer ` prefix).
    fn token(&self) -> String;
}

impl TokenProvider for String {
    fn token(&self) -> String {
        self.clone()
    }
}

impl TokenProvider for &'static str {
    fn token(&self) -> String {
        (*self).to_string()
    }
}
