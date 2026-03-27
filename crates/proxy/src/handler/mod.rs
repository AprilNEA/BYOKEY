//! HTTP route handlers for all proxy endpoints.
//!
//! - [`chat`] / [`messages`] / [`models`] — `OpenAI`-compatible API.
//! - [`amp`] / [`amp_provider`]           — Amp CLI / `AmpCode` compatibility.
//! - [`accounts`] / [`status`] / [`ratelimits`] — Management API.

pub mod accounts;
pub(crate) mod amp;
pub(crate) mod amp_provider;
pub(crate) mod chat;
pub(crate) mod messages;
pub(crate) mod models;
pub mod ratelimits;
pub mod status;
