use std::path::PathBuf;

use crate::error::{DaemonError, Result};

/// Return the user's home directory, or error if unset.
pub fn home_dir() -> Result<PathBuf> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| DaemonError::NoHomeDir)
}

pub fn pid_path() -> Result<PathBuf> {
    Ok(home_dir()?.join(".byokey").join("byokey.pid"))
}

pub fn log_path() -> Result<PathBuf> {
    Ok(home_dir()?.join(".byokey").join("server.log"))
}

pub fn config_path() -> Result<PathBuf> {
    Ok(home_dir()?
        .join(".config")
        .join("byokey")
        .join("settings.json"))
}

pub fn db_path() -> Result<PathBuf> {
    Ok(home_dir()?.join(".byokey").join("tokens.db"))
}

#[cfg(target_os = "macos")]
pub fn launchd_plist_path() -> Result<PathBuf> {
    Ok(home_dir()?
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{}.plist", crate::LAUNCHD_LABEL)))
}

#[cfg(target_os = "linux")]
pub fn systemd_unit_path() -> Result<PathBuf> {
    Ok(home_dir()?
        .join(".config")
        .join("systemd")
        .join("user")
        .join(format!("{}.service", crate::SYSTEMD_UNIT)))
}
