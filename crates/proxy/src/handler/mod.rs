//! HTTP route handlers for all proxy endpoints.
//!
//! - [`chat`] / [`messages`] / [`models`] — `OpenAI`-compatible API.
//! - [`amp`]                              — Amp CLI / `AmpCode` compatibility.
//! - [`management`]                       — BYOKEY management API (`/v0/management/*`).

pub mod amp;
pub(crate) mod chat;
pub mod management;
pub(crate) mod messages;
pub(crate) mod models;

// ── Shared header-filtering constants for proxy handlers ────────────

/// Headers that must not be forwarded (hop-by-hop per RFC 2616 §13.5.1).
pub(crate) const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailers",
    "transfer-encoding",
    "upgrade",
];

/// Authentication headers stripped from client requests in shared-proxy mode.
pub(crate) const CLIENT_AUTH_HEADERS: &[&str] = &["authorization", "x-api-key", "x-goog-api-key"];

/// Headers that can fingerprint or reveal the client's network identity.
pub(crate) const FINGERPRINT_HEADERS: &[&str] = &[
    "x-forwarded-for",
    "x-forwarded-host",
    "x-forwarded-proto",
    "x-real-ip",
    "forwarded",
    "via",
    "priority",
];
