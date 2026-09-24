//! HTTP route handlers for all proxy endpoints.
//!
//! - [`chat`] / [`messages`] / [`models`] / [`responses`] — `OpenAI`- and
//!   Anthropic-compatible API.
//! - [`management`] — BYOKEY `ConnectRPC` management API.

pub(crate) mod chat;
pub mod management;
pub(crate) mod messages;
pub(crate) mod models;
pub(crate) mod responses;
