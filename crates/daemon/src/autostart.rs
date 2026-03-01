use std::fmt::Write as _;
use std::path::PathBuf;

use crate::error::{DaemonError, Result};

/// Options for configuring autostart.
pub struct AutostartOptions {
    /// Path to the byokey executable. `None` = `current_exe()`.
    pub exe: Option<PathBuf>,
    pub config: Option<PathBuf>,
    pub port: Option<u16>,
    pub host: Option<String>,
    pub db: Option<PathBuf>,
    pub log_file: Option<PathBuf>,
}

pub struct AutostartEnableResult {
    pub backend: &'static str,
    pub service_file: PathBuf,
}

pub struct AutostartStatus {
    pub enabled: bool,
    pub service_file: Option<PathBuf>,
    pub service_running: bool,
    pub backend: &'static str,
}

// ── macOS (launchd) ──────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
pub fn enable(opts: AutostartOptions) -> Result<AutostartEnableResult> {
    use std::process::Stdio;

    let exe = match opts.exe {
        Some(p) => p,
        None => std::env::current_exe().map_err(DaemonError::SpawnFailed)?,
    };
    let log_path = opts.log_file.map_or_else(crate::paths::log_path, Ok)?;
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| DaemonError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }

    // Build the <array> of ProgramArguments entries.
    let mut args = format!(
        "        <string>{}</string>\n        <string>serve</string>\n",
        exe.to_string_lossy()
    );
    if let Some(p) = &opts.config {
        let _ = write!(
            args,
            "        <string>--config</string>\n        <string>{}</string>\n",
            p.to_string_lossy()
        );
    }
    if let Some(p) = opts.port {
        let _ = write!(
            args,
            "        <string>--port</string>\n        <string>{p}</string>\n"
        );
    }
    if let Some(h) = &opts.host {
        let _ = write!(
            args,
            "        <string>--host</string>\n        <string>{h}</string>\n"
        );
    }
    if let Some(d) = &opts.db {
        let _ = write!(
            args,
            "        <string>--db</string>\n        <string>{}</string>\n",
            d.to_string_lossy()
        );
    }

    let log = log_path.to_string_lossy();
    let label = crate::LAUNCHD_LABEL;
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
{args}    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{log}</string>
    <key>StandardErrorPath</key>
    <string>{log}</string>
</dict>
</plist>
"#
    );

    let plist_path = crate::paths::launchd_plist_path()?;
    if let Some(parent) = plist_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| DaemonError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    std::fs::write(&plist_path, plist).map_err(|e| DaemonError::Io {
        path: plist_path.clone(),
        source: e,
    })?;

    // Unload first in case a previous version is already loaded.
    let _ = std::process::Command::new("launchctl")
        .args(["unload", &plist_path.to_string_lossy()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    let ok = std::process::Command::new("launchctl")
        .args(["load", &plist_path.to_string_lossy()])
        .status()
        .is_ok_and(|s| s.success());

    if ok {
        Ok(AutostartEnableResult {
            backend: "launchd",
            service_file: plist_path,
        })
    } else {
        Err(DaemonError::ServiceToolFailed {
            tool: "launchctl load",
        })
    }
}

#[cfg(target_os = "macos")]
pub fn disable() -> Result<()> {
    let plist_path = crate::paths::launchd_plist_path()?;
    if !plist_path.exists() {
        return Err(DaemonError::AutostartNotEnabled);
    }
    let _ = std::process::Command::new("launchctl")
        .args(["unload", &plist_path.to_string_lossy()])
        .status();
    std::fs::remove_file(&plist_path).map_err(|e| DaemonError::Io {
        path: plist_path,
        source: e,
    })?;
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn status() -> Result<AutostartStatus> {
    use std::process::Stdio;

    let plist_path = crate::paths::launchd_plist_path()?;
    if !plist_path.exists() {
        return Ok(AutostartStatus {
            enabled: false,
            service_file: None,
            service_running: false,
            backend: "launchd",
        });
    }
    let running = std::process::Command::new("launchctl")
        .args(["list", crate::LAUNCHD_LABEL])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    Ok(AutostartStatus {
        enabled: true,
        service_file: Some(plist_path),
        service_running: running,
        backend: "launchd",
    })
}

// ── Linux (systemd user) ─────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
pub fn enable(opts: AutostartOptions) -> Result<AutostartEnableResult> {
    let exe = match opts.exe {
        Some(p) => p,
        None => std::env::current_exe().map_err(DaemonError::SpawnFailed)?,
    };
    let log_path = opts.log_file.map_or_else(crate::paths::log_path, Ok)?;
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| DaemonError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }

    let mut exec_args = vec![exe.to_string_lossy().into_owned(), "serve".to_owned()];
    if let Some(p) = &opts.config {
        exec_args.extend(["--config".to_owned(), p.to_string_lossy().into_owned()]);
    }
    if let Some(p) = opts.port {
        exec_args.extend(["--port".to_owned(), p.to_string()]);
    }
    if let Some(h) = &opts.host {
        exec_args.extend(["--host".to_owned(), h.clone()]);
    }
    if let Some(d) = &opts.db {
        exec_args.extend(["--db".to_owned(), d.to_string_lossy().into_owned()]);
    }

    let log = log_path.to_string_lossy();
    let unit = format!(
        "[Unit]\n\
         Description=byokey AI proxy gateway\n\
         After=network.target\n\
         \n\
         [Service]\n\
         ExecStart={exec}\n\
         Restart=on-failure\n\
         StandardOutput=append:{log}\n\
         StandardError=append:{log}\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        exec = exec_args.join(" "),
    );

    let unit_path = crate::paths::systemd_unit_path()?;
    if let Some(parent) = unit_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| DaemonError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    std::fs::write(&unit_path, unit).map_err(|e| DaemonError::Io {
        path: unit_path.clone(),
        source: e,
    })?;

    let _ = std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();

    let ok = std::process::Command::new("systemctl")
        .args(["--user", "enable", "--now", crate::SYSTEMD_UNIT])
        .status()
        .is_ok_and(|s| s.success());

    if ok {
        Ok(AutostartEnableResult {
            backend: "systemd user",
            service_file: unit_path,
        })
    } else {
        Err(DaemonError::ServiceToolFailed {
            tool: "systemctl enable",
        })
    }
}

#[cfg(target_os = "linux")]
pub fn disable() -> Result<()> {
    let unit_path = crate::paths::systemd_unit_path()?;
    if !unit_path.exists() {
        return Err(DaemonError::AutostartNotEnabled);
    }
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "disable", "--now", crate::SYSTEMD_UNIT])
        .status();
    std::fs::remove_file(&unit_path).map_err(|e| DaemonError::Io {
        path: unit_path,
        source: e,
    })?;
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn status() -> Result<AutostartStatus> {
    let unit_path = crate::paths::systemd_unit_path()?;
    if !unit_path.exists() {
        return Ok(AutostartStatus {
            enabled: false,
            service_file: None,
            service_running: false,
            backend: "systemd user",
        });
    }
    let running = std::process::Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", crate::SYSTEMD_UNIT])
        .status()
        .is_ok_and(|s| s.success());
    Ok(AutostartStatus {
        enabled: true,
        service_file: Some(unit_path),
        service_running: running,
        backend: "systemd user",
    })
}

// ── Unsupported platforms ────────────────────────────────────────────────────

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn enable(_opts: AutostartOptions) -> Result<AutostartEnableResult> {
    Err(DaemonError::PlatformUnsupported)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn disable() -> Result<()> {
    Err(DaemonError::PlatformUnsupported)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn status() -> Result<AutostartStatus> {
    Err(DaemonError::PlatformUnsupported)
}
