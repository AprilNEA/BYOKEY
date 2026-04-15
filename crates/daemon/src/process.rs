use std::path::PathBuf;
use std::process::Stdio;
use std::thread;
use std::time::{Duration, Instant};

use crate::control;
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
    pub pid: Option<u32>,
}

pub enum ServerStatus {
    Running { pid: u32 },
    Stale { pid: u32 },
    Stopped,
}

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

pub fn start(opts: StartOptions) -> Result<StartResult> {
    // Authoritative liveness: control socket.
    if let Ok(info) = control::status() {
        return Err(DaemonError::AlreadyRunning { pid: info.pid });
    }

    // Clean up any stale socket/pid file from an unclean exit.
    if let Ok(sock) = paths::control_sock_path() {
        let _ = std::fs::remove_file(&sock);
    }
    let pid_path = opts.pid_file.clone().map_or_else(paths::pid_path, Ok)?;
    if pid_path.exists() {
        let _ = std::fs::remove_file(&pid_path);
    }

    let log_path = opts.log_file.clone().map_or_else(paths::log_path, Ok)?;
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
    // Preferred path: graceful shutdown over the control socket.
    if let Ok(info) = control::status() {
        let pid = info.pid;
        control::shutdown()?;
        wait_for_shutdown();
        cleanup_pid_file();
        return Ok(StopResult { pid: Some(pid) });
    }

    // Fallback: no live server behind the socket. If a pid file exists, try SIGTERM.
    let pid_path = paths::pid_path()?;
    let pid_str = std::fs::read_to_string(&pid_path).map_err(|_| DaemonError::NotRunning)?;
    let pid: u32 = pid_str
        .trim()
        .parse()
        .map_err(|_| DaemonError::MalformedPidFile {
            raw: pid_str.trim().to_owned(),
        })?;

    let ok = std::process::Command::new("kill")
        .arg(pid.to_string())
        .status()
        .is_ok_and(|s| s.success());
    let _ = std::fs::remove_file(&pid_path);

    if ok {
        Ok(StopResult { pid: Some(pid) })
    } else {
        Err(DaemonError::StopFailed { pid })
    }
}

pub fn restart(opts: StartOptions) -> Result<StartResult> {
    let _ = stop();
    // Give the socket/pid cleanup a moment (stop() already waited for graceful exit).
    thread::sleep(Duration::from_millis(100));
    start(opts)
}

pub fn status() -> Result<ServerStatus> {
    // Authoritative: is the control socket answering?
    if let Ok(info) = control::status() {
        return Ok(ServerStatus::Running { pid: info.pid });
    }

    // Non-authoritative fallback: pid file only.
    let pid_path = paths::pid_path()?;
    let Ok(pid_str) = std::fs::read_to_string(&pid_path) else {
        return Ok(ServerStatus::Stopped);
    };
    let pid: u32 = pid_str.trim().parse().unwrap_or(0);
    if pid == 0 {
        return Ok(ServerStatus::Stopped);
    }
    // A pid file exists but the socket is unreachable — treat as stale.
    Ok(ServerStatus::Stale { pid })
}

fn wait_for_shutdown() {
    let Ok(sock) = paths::control_sock_path() else {
        return;
    };
    let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
    while Instant::now() < deadline {
        if !sock.exists() && !control::is_alive() {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn cleanup_pid_file() {
    if let Ok(p) = paths::pid_path() {
        let _ = std::fs::remove_file(p);
    }
}
