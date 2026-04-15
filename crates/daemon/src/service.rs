//! Cross-platform OS service registration (replaces the hand-rolled autostart).
//!
//! Thin wrapper over the `service-manager` crate so the CLI talks to launchd,
//! systemd-user, and Windows SCM through one API. Each subcommand maps to one
//! method: `install`, `uninstall`, `start`, `stop`, `status`.

use std::ffi::OsString;
use std::path::PathBuf;

use service_manager::{
    ServiceInstallCtx, ServiceLabel, ServiceLevel, ServiceManager, ServiceStartCtx, ServiceStatus,
    ServiceStatusCtx, ServiceStopCtx, ServiceUninstallCtx,
};

use crate::error::{DaemonError, Result};
use crate::{SERVICE_LABEL, paths};

/// Options carried to the service unit. Mirrors `process::StartOptions` but without
/// the pid-file fields — the service is supervised by the OS, not by us.
#[derive(Debug, Default)]
pub struct ServiceOptions {
    /// Path to the byokey executable. `None` = `current_exe()`.
    pub exe: Option<PathBuf>,
    pub config: Option<PathBuf>,
    pub port: Option<u16>,
    pub host: Option<String>,
    pub db: Option<PathBuf>,
    pub log_file: Option<PathBuf>,
}

pub struct ServiceInstallResult {
    pub backend: &'static str,
    pub label: String,
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

fn build_args(opts: &ServiceOptions) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![OsString::from("serve")];
    if let Some(p) = &opts.config {
        args.push(OsString::from("--config"));
        args.push(p.clone().into_os_string());
    }
    if let Some(p) = opts.port {
        args.push(OsString::from("--port"));
        args.push(OsString::from(p.to_string()));
    }
    if let Some(h) = &opts.host {
        args.push(OsString::from("--host"));
        args.push(OsString::from(h));
    }
    if let Some(d) = &opts.db {
        args.push(OsString::from("--db"));
        args.push(d.clone().into_os_string());
    }
    if let Some(f) = &opts.log_file {
        args.push(OsString::from("--log-file"));
        args.push(f.clone().into_os_string());
    }
    args
}

#[allow(clippy::needless_pass_by_value)]
pub fn install(opts: ServiceOptions) -> Result<ServiceInstallResult> {
    let mgr = manager()?;
    let program = match opts.exe {
        Some(ref p) => p.clone(),
        None => std::env::current_exe().map_err(DaemonError::SpawnFailed)?,
    };

    if let Some(f) = &opts.log_file
        && let Some(parent) = f.parent()
    {
        std::fs::create_dir_all(parent).map_err(|e| DaemonError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }

    let args = build_args(&opts);
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
