//! HTTP route handlers.
//!
//! - `messages` / `count_tokens` / `models` — the Anthropic Messages API
//!   Claude Code and Claude Desktop speak, served from Anthropic, Copilot
//!   (`copilot_messages`) or Cursor (`cursor_messages`). `normalize`
//!   shapes request bodies; `forward` relays the upstream's answer.
//! - [`management`] — BYOKEY's `ConnectRPC` management API.

mod copilot_messages;
pub(crate) mod count_tokens;
pub(crate) mod cursor_messages;
mod forward;
pub mod management;
pub(crate) mod messages;
pub(crate) mod models;
mod normalize;
