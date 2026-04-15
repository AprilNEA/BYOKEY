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

pub fn control_sock_path() -> Result<PathBuf> {
    Ok(home_dir()?.join(".byokey").join("control.sock"))
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
