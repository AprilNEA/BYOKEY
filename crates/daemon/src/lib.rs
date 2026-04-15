#![allow(clippy::missing_errors_doc)]

pub mod control;
pub mod error;
pub mod paths;
pub mod process;
pub mod service;

/// Cross-platform service label used by launchd / systemd / Windows SCM.
pub const SERVICE_LABEL: &str = "io.byokey.server";
