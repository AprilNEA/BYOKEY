//! BYOKEY ConnectRPC protocol definitions.
//!
//! This crate contains the protobuf schemas and generated Rust code for
//! services consumed over ConnectRPC by both the byokey CLI and the
//! byokey desktop app.
//!
//! The generated code is produced at build time from `proto/*.proto` by
//! [`connectrpc-build`] and exposed under module paths that mirror the
//! proto package hierarchy.
//!
//! # Re-exports
//!
//! - [`byokey::status`] — server health, usage, rate limits
//! - [`byokey::accounts`] — provider account management
//! - [`byokey::amp`] — Amp CLI thread browsing

#![allow(
    dead_code,
    non_camel_case_types,
    unused_imports,
    clippy::all,
    clippy::pedantic
)]

include!(concat!(env!("OUT_DIR"), "/_connectrpc.rs"));
