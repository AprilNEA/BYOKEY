//! BYOKEY ConnectRPC protocol definitions.
//!
//! This crate contains the protobuf schemas and generated Rust code for
//! services consumed over ConnectRPC by the byokey CLI and TUI.
//!
//! The generated code is produced at build time from `proto/*.proto` by
//! `connectrpc-build` and exposed under module paths that mirror the
//! proto package hierarchy.
//!
//! # Re-exports
//!
//! - [`byokey::status`] — server health and usage
//! - [`byokey::accounts`] — provider accounts
//! - [`client`] — optional management API client wrapper (`client` feature)

#![allow(
    dead_code,
    non_camel_case_types,
    unused_imports,
    clippy::all,
    clippy::pedantic
)]

#[cfg(feature = "client")]
pub mod client;

/// The management API revision this schema describes, reported in
/// `ServerInfo.api_version`.
///
/// Bump it when a client built against the previous schema would misbehave
/// against this one: a removed or retyped RPC or field, or new semantics for
/// an existing one. Purely additive changes don't need a bump.
///
/// - 1: `GetStatus`, `GetUsage`, `ListAccounts`
/// - 2: account writes (`RemoveAccount`, `ActivateAccount`, `AddApiKey`,
///   `ImportClaudeCode`), streaming `Login`, `ServerInfo` version fields
pub const API_VERSION: u32 = 2;

include!(concat!(env!("OUT_DIR"), "/_connectrpc.rs"));
