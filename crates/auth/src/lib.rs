//! OAuth authentication flows for all supported providers.
//!
//! ## Module layout
//!
//! - [`token`]       — Shared token response parsing and `DeviceCodeResponse`.
//! - [`provider`]    — Per-provider constants, URL builders, and parameter formatters.
//! - [`flow`]        — Interactive login flow orchestration (auth-code & device-code).
//! - [`manager`]     — [`AuthManager`]: token lifecycle, refresh, cooldown.
//! - [`credentials`] — Remote OAuth app credential loader.
//! - [`callback`]    — Local HTTP callback server for redirect flows.
//! - [`pkce`]        — PKCE and random state generation utilities.

pub mod callback;
pub mod credentials;
pub mod flow;
pub mod manager;
pub mod pkce;
pub mod provider;
pub mod token;

pub use manager::AuthManager;
