use std::path::PathBuf;

/// Daemon-specific error type.
#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("byokey is already running (pid {pid})")]
    AlreadyRunning { pid: u32 },

    #[error("byokey is not running (PID file not found)")]
    NotRunning,

    #[error("failed to stop process {pid}")]
    StopFailed { pid: u32 },

    #[error("failed to spawn daemon process")]
    SpawnFailed(#[source] std::io::Error),

    #[error("I/O error on {path}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("cannot determine home directory")]
    NoHomeDir,

    #[error("autostart is not enabled")]
    AutostartNotEnabled,

    #[error("{tool} failed")]
    ServiceToolFailed { tool: &'static str },

    #[error("autostart is not supported on this platform")]
    PlatformUnsupported,
}

pub type Result<T> = std::result::Result<T, DaemonError>;
