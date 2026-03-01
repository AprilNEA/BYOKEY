#![allow(clippy::missing_errors_doc)]

pub mod autostart;
pub mod error;
pub mod paths;
pub mod process;

/// `launchd` service label (macOS).
#[cfg(target_os = "macos")]
pub const LAUNCHD_LABEL: &str = "io.byokey.server";

/// `systemd` unit name (Linux).
#[cfg(target_os = "linux")]
pub const SYSTEMD_UNIT: &str = "byokey";
