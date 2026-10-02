//! HTTP route handlers.
//!
//! - `messages` / `count_tokens` / `models` — the Anthropic Messages API
//!   Claude Code and Claude Desktop speak, served from Anthropic, Copilot
//!   (`copilot_messages`) or Cursor (`cursor_messages`). `normalize`
//!   shapes request bodies; `forward` relays the upstream's answer.
//! - [`management`] — BYOKEY's `ConnectRPC` management API.

use serde_json::Value;

pub(crate) mod catalog;
mod copilot;
mod copilot_messages;
pub(crate) mod count_tokens;
pub(crate) mod cursor_messages;
mod forward;
pub mod management;
pub(crate) mod messages;
pub(crate) mod models;
mod normalize;
pub(crate) mod responses;

/// Name the model a Messages-format request asks for on the request's span
/// (the router's `http` span, which a handler runs in), so every line of
/// the request carries it. Returns the span for more fields.
fn record_model(body: &Value) -> tracing::Span {
    let span = tracing::Span::current();
    if let Some(model) = body.get("model").and_then(Value::as_str) {
        span.record("model", model);
    }
    span
}
