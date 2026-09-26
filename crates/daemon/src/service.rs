//! Cross-platform OS service registration (replaces the hand-rolled autostart).
//!
//! Thin wrapper over the `service-manager` crate so the CLI talks to launchd,
//! systemd-user, and Windows SCM through one API. Each subcommand maps to one
//! method: `install`, `uninstall`, `start`, `stop`, `status`.

use std::path::PathBuf;

use service_manager::{
    ServiceInstallCtx, ServiceLabel, ServiceLevel, ServiceManager, ServiceStartCtx, ServiceStatus,
    ServiceStatusCtx, ServiceStopCtx, ServiceUninstallCtx,
};

use crate::error::{DaemonError, Result};
use crate::{SERVICE_LABEL, ServeOptions, paths};

pub struct ServiceInstallResult {
    pub backend: &'static str,
    pub label: String,
    pub log_path: PathBuf,
}

pub struct ServiceStatusInfo {
    pub backend: &'static str,
    pub installed: bool,
    pub running: bool,
}

fn label() -> ServiceLabel {
    SERVICE_LABEL
        .parse()
        .expect("SERVICE_LABEL is a valid service label")
}

fn manager() -> Result<Box<dyn ServiceManager>> {
    let mut mgr = <dyn ServiceManager>::native().map_err(|_| DaemonError::PlatformUnsupported)?;
    mgr.set_level(ServiceLevel::User)
        .map_err(|_| DaemonError::ServiceToolFailed {
            tool: "set_level(User)",
        })?;
    Ok(mgr)
}

fn backend_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "launchd (user)"
    } else if cfg!(target_os = "linux") {
        "systemd (user)"
    } else if cfg!(target_os = "windows") {
        "Windows SCM"
    } else {
        "unknown"
    }
}

/// Register `byokey serve` with `opts` as a user service that starts at
/// login. It logs to `opts.log_file`, or [`paths::log_path`] like
/// `byokey start`, so a service that dies leaves a trace.
pub fn install(opts: &ServeOptions) -> Result<ServiceInstallResult> {
    let mgr = manager()?;
    let program = std::env::current_exe().map_err(DaemonError::SpawnFailed)?;

    let log_path = match opts.log_file.clone() {
        Some(p) => p,
        None => paths::log_path()?,
    };
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| DaemonError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }

    let mut args = opts.args();
    args.extend(["--log-file".into(), log_path.clone().into()]);
    let ctx = ServiceInstallCtx {
        label: label(),
        program,
        args,
        contents: None,
        username: None,
        working_directory: paths::home_dir().ok(),
        environment: None,
        autostart: true,
        restart_policy: service_manager::RestartPolicy::default(),
    };
    mgr.install(ctx)
        .map_err(|_| DaemonError::ServiceToolFailed { tool: "install" })?;

    Ok(ServiceInstallResult {
        backend: backend_name(),
        label: SERVICE_LABEL.to_owned(),
        log_path,
    })
}

pub fn uninstall() -> Result<()> {
    let mgr = manager()?;
    let _ = mgr.stop(ServiceStopCtx { label: label() });
    mgr.uninstall(ServiceUninstallCtx { label: label() })
        .map_err(|_| DaemonError::ServiceToolFailed { tool: "uninstall" })
}

pub fn start() -> Result<()> {
    let mgr = manager()?;
    mgr.start(ServiceStartCtx { label: label() })
        .map_err(|_| DaemonError::ServiceToolFailed { tool: "start" })
}

pub fn stop() -> Result<()> {
    let mgr = manager()?;
    mgr.stop(ServiceStopCtx { label: label() })
        .map_err(|_| DaemonError::ServiceToolFailed { tool: "stop" })
}

pub fn status() -> Result<ServiceStatusInfo> {
    let mgr = manager()?;
    let s = mgr
        .status(ServiceStatusCtx { label: label() })
        .map_err(|_| DaemonError::ServiceToolFailed { tool: "status" })?;
    let (installed, running) = match s {
        ServiceStatus::NotInstalled => (false, false),
        ServiceStatus::Stopped(_) => (true, false),
        ServiceStatus::Running => (true, true),
    };
    Ok(ServiceStatusInfo {
        backend: backend_name(),
        installed,
        running,
    })
}
