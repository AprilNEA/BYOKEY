//! HTTP route handlers.
//!
//! - `messages` / `count_tokens` / `models` — the Anthropic Messages API
//!   Claude Code and Claude Desktop speak, served from Anthropic, Copilot
//!   (`messages`) or Cursor (`cursor_messages`).
//! - [`management`] — BYOKEY's `ConnectRPC` management API.

pub(crate) mod count_tokens;
pub(crate) mod cursor_messages;
pub mod management;
pub(crate) mod messages;
pub(crate) mod models;
