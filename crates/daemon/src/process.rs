use std::path::PathBuf;
use std::process::Stdio;

use crate::error::{DaemonError, Result};
use crate::paths;

/// Options for starting the daemon.
pub struct StartOptions {
    /// Path to the byokey executable. `None` = `current_exe()`.
    pub exe: Option<PathBuf>,
    pub config: Option<PathBuf>,
    pub port: Option<u16>,
    pub host: Option<String>,
    pub db: Option<PathBuf>,
    pub log_file: Option<PathBuf>,
    pub pid_file: Option<PathBuf>,
}

pub struct StartResult {
    pub pid: u32,
    pub log_path: PathBuf,
}

pub struct StopResult {
    pub pid: u32,
}

pub enum ServerStatus {
    Running { pid: u32 },
    Stale { pid: u32 },
    Stopped,
}

pub fn start(opts: StartOptions) -> Result<StartResult> {
    let pid_path = opts.pid_file.map_or_else(paths::pid_path, Ok)?;

    // Detect stale or live PID file.
    if let Ok(pid_str) = std::fs::read_to_string(&pid_path) {
        let pid = pid_str.trim().to_owned();
        let alive = std::process::Command::new("kill")
            .args(["-0", &pid])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if alive {
            return Err(DaemonError::AlreadyRunning {
                pid: pid.parse().unwrap_or(0),
            });
        }
        // Stale PID — clean up and continue.
        let _ = std::fs::remove_file(&pid_path);
    }

    let log_path = opts.log_file.map_or_else(paths::log_path, Ok)?;
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| DaemonError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    let log_f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| DaemonError::Io {
            path: log_path.clone(),
            source: e,
        })?;
    let log_f2 = log_f.try_clone().map_err(|e| DaemonError::Io {
        path: log_path.clone(),
        source: e,
    })?;

    let exe = match opts.exe {
        Some(p) => p,
        None => std::env::current_exe().map_err(DaemonError::SpawnFailed)?,
    };
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("serve");
    if let Some(p) = &opts.config {
        cmd.args(["--config", &p.to_string_lossy()]);
    }
    if let Some(p) = opts.port {
        cmd.args(["--port", &p.to_string()]);
    }
    if let Some(h) = &opts.host {
        cmd.args(["--host", h]);
    }
    if let Some(d) = &opts.db {
        cmd.args(["--db", &d.to_string_lossy()]);
    }
    cmd.stdout(log_f).stderr(log_f2).stdin(Stdio::null());
    // Detach from the terminal's process group so the child survives terminal close.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        cmd.process_group(0);
    }

    let child = cmd.spawn().map_err(DaemonError::SpawnFailed)?;
    let pid = child.id();

    if let Some(parent) = pid_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| DaemonError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    std::fs::write(&pid_path, pid.to_string()).map_err(|e| DaemonError::Io {
        path: pid_path,
        source: e,
    })?;

    Ok(StartResult { pid, log_path })
}

pub fn stop() -> Result<StopResult> {
    let pid_path = paths::pid_path()?;
    let pid_str = std::fs::read_to_string(&pid_path).map_err(|_| DaemonError::NotRunning)?;
    let pid = pid_str.trim().to_owned();

    let ok = std::process::Command::new("kill")
        .arg(&pid)
        .status()
        .is_ok_and(|s| s.success());
    let _ = std::fs::remove_file(&pid_path);

    let pid_num = pid.parse().unwrap_or(0);
    if ok {
        Ok(StopResult { pid: pid_num })
    } else {
        Err(DaemonError::StopFailed { pid: pid_num })
    }
}

pub fn restart(opts: StartOptions) -> Result<StartResult> {
    // Try to stop — ignore errors (may not be running).
    let _ = stop();
    std::thread::sleep(std::time::Duration::from_millis(500));
    start(opts)
}

pub fn status() -> Result<ServerStatus> {
    let pid_path = paths::pid_path()?;
    let Ok(pid_str) = std::fs::read_to_string(&pid_path) else {
        return Ok(ServerStatus::Stopped);
    };
    let pid: u32 = pid_str.trim().parse().unwrap_or(0);
    let alive = std::process::Command::new("kill")
        .args(["-0", pid_str.trim()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if alive {
        Ok(ServerStatus::Running { pid })
    } else {
        Ok(ServerStatus::Stale { pid })
    }
}
