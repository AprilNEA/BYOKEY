//! HTTP route handlers for all proxy endpoints.
//!
//! - [`chat`] / [`messages`] / [`models`] — `OpenAI`-compatible API.
//! - [`amp`]                              — Amp CLI / `AmpCode` proxy.
//! - [`management`]                       — BYOKEY management API (`/v0/management/*`).

pub mod amp;
pub(crate) mod chat;
pub mod management;
pub(crate) mod messages;
pub(crate) mod models;
