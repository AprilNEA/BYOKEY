use anyhow::Result;
use byokey_auth::AuthManager;
use byokey_config::Config;
use byokey_proxy::AppState;
use byokey_store::SqliteTokenStore;
use byokey_types::ProviderId;
use clap::{Parser, Subcommand};
use std::{path::PathBuf, process::Stdio, sync::Arc};

#[cfg(unix)]
use std::os::unix::process::CommandExt as _;

/// `launchd` service label (macOS).
#[cfg(target_os = "macos")]
const LAUNCHD_LABEL: &str = "io.byokey.server";
/// `systemd` unit name (Linux).
#[cfg(target_os = "linux")]
const SYSTEMD_UNIT: &str = "byokey";

#[derive(Parser, Debug)]
#[command(name = "byokey", about = "byokey — Bring Your Own Keys AI proxy")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Start the proxy server (foreground).
    Serve {
        /// Path to the configuration file (JSON or YAML).
        /// Defaults to ~/.config/byokey/settings.json if it exists.
        #[arg(short, long, value_name = "FILE")]
        config: Option<PathBuf>,
        /// Override the listening port (default: 8018).
        #[arg(short, long)]
        port: Option<u16>,
        /// Override the listening address (default: 127.0.0.1).
        #[arg(long)]
        host: Option<String>,
        /// SQLite database path (default: ~/.byokey/tokens.db).
        #[arg(long, value_name = "PATH")]
        db: Option<PathBuf>,
    },
    /// Start the proxy server in the background.
    Start {
        /// Path to the configuration file (JSON or YAML).
        /// Defaults to ~/.config/byokey/settings.json if it exists.
        #[arg(short, long, value_name = "FILE")]
        config: Option<PathBuf>,
        /// Override the listening port (default: 8018).
        #[arg(short, long)]
        port: Option<u16>,
        /// Override the listening address (default: 127.0.0.1).
        #[arg(long)]
        host: Option<String>,
        /// SQLite database path (default: ~/.byokey/tokens.db).
        #[arg(long, value_name = "PATH")]
        db: Option<PathBuf>,
        /// Log file path (default: ~/.byokey/server.log).
        #[arg(long, value_name = "PATH")]
        log_file: Option<PathBuf>,
    },
    /// Stop the background proxy server.
    Stop,
    /// Restart the background proxy server.
    Restart {
        /// Path to the configuration file (JSON or YAML).
        /// Defaults to ~/.config/byokey/settings.json if it exists.
        #[arg(short, long, value_name = "FILE")]
        config: Option<PathBuf>,
        /// Override the listening port (default: 8018).
        #[arg(short, long)]
        port: Option<u16>,
        /// Override the listening address (default: 127.0.0.1).
        #[arg(long)]
        host: Option<String>,
        /// SQLite database path (default: ~/.byokey/tokens.db).
        #[arg(long, value_name = "PATH")]
        db: Option<PathBuf>,
        /// Log file path (default: ~/.byokey/server.log).
        #[arg(long, value_name = "PATH")]
        log_file: Option<PathBuf>,
    },
    /// Manage auto-start on system boot.
    Autostart {
        #[command(subcommand)]
        action: AutostartAction,
    },
    /// Authenticate with a provider.
    Login {
        /// Provider name (claude / codex / copilot / gemini / qwen / kimi / iflow …).
        provider: String,
        /// SQLite database path (default: ~/.byokey/tokens.db).
        #[arg(long, value_name = "PATH")]
        db: Option<PathBuf>,
    },
    /// Remove stored credentials for a provider.
    Logout {
        /// Provider name.
        provider: String,
        /// SQLite database path (default: ~/.byokey/tokens.db).
        #[arg(long, value_name = "PATH")]
        db: Option<PathBuf>,
    },
    /// Show authentication status for all providers.
    Status {
        /// SQLite database path (default: ~/.byokey/tokens.db).
        #[arg(long, value_name = "PATH")]
        db: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
enum AutostartAction {
    /// Register byokey as a boot-time service.
    Enable {
        /// Path to the configuration file (JSON or YAML).
        /// Defaults to ~/.config/byokey/settings.json if it exists.
        #[arg(short, long, value_name = "FILE")]
        config: Option<PathBuf>,
        /// Override the listening port (default: 8018).
        #[arg(short, long)]
        port: Option<u16>,
        /// Override the listening address (default: 127.0.0.1).
        #[arg(long)]
        host: Option<String>,
        /// SQLite database path (default: ~/.byokey/tokens.db).
        #[arg(long, value_name = "PATH")]
        db: Option<PathBuf>,
        /// Log file path (default: ~/.byokey/server.log).
        #[arg(long, value_name = "PATH")]
        log_file: Option<PathBuf>,
    },
    /// Unregister the boot-time service.
    Disable,
    /// Show boot-time service registration status.
    Status,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Serve {
            config,
            port,
            host,
            db,
        } => cmd_serve(config, port, host, db).await,
        Commands::Start {
            config,
            port,
            host,
            db,
            log_file,
        } => cmd_start(config, port, host, db, log_file),
        Commands::Stop => cmd_stop(),
        Commands::Restart {
            config,
            port,
            host,
            db,
            log_file,
        } => cmd_restart(config, port, host, db, log_file),
        Commands::Autostart { action } => match action {
            AutostartAction::Enable {
                config,
                port,
                host,
                db,
                log_file,
            } => autostart_enable_impl(config, port, host, db, log_file),
            AutostartAction::Disable => autostart_disable_impl(),
            AutostartAction::Status => autostart_status_impl(),
        },
        Commands::Login { provider, db } => cmd_login(provider, db).await,
        Commands::Logout { provider, db } => cmd_logout(provider, db).await,
        Commands::Status { db } => cmd_status(db).await,
    }
}

// ── Foreground serve ─────────────────────────────────────────────────────────

async fn cmd_serve(
    config_path: Option<PathBuf>,
    port: Option<u16>,
    host: Option<String>,
    db: Option<PathBuf>,
) -> Result<()> {
    let effective_path = config_path.or_else(|| {
        let default = default_config_path();
        if default.exists() {
            Some(default)
        } else {
            None
        }
    });
    let mut config = if let Some(path) = &effective_path {
        Config::from_file(path).map_err(|e| anyhow::anyhow!("config error: {e}"))?
    } else {
        Config::default()
    };

    if let Some(p) = port {
        config.port = p;
    }
    if let Some(h) = host {
        config.host = h;
    }

    let addr = format!("{}:{}", config.host, config.port);
    let auth = Arc::new(AuthManager::new(Arc::new(open_store(db).await?)));
    let state = AppState::new(config, auth);
    let app = byokey_proxy::make_router(state);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    eprintln!("byokey listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

// ── Background start / stop / restart ────────────────────────────────────────

fn cmd_start(
    config_path: Option<PathBuf>,
    port: Option<u16>,
    host: Option<String>,
    db: Option<PathBuf>,
    log_file: Option<PathBuf>,
) -> Result<()> {
    let pid_path = default_pid_path();

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
            anyhow::bail!("byokey is already running (pid {pid})");
        }
        // Stale PID — clean up and continue.
        let _ = std::fs::remove_file(&pid_path);
    }

    let log_path = log_file.unwrap_or_else(default_log_path);
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let log_f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let log_f2 = log_f.try_clone()?;

    let exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("serve");
    if let Some(p) = &config_path {
        cmd.args(["--config", &p.to_string_lossy()]);
    }
    if let Some(p) = port {
        cmd.args(["--port", &p.to_string()]);
    }
    if let Some(h) = &host {
        cmd.args(["--host", h]);
    }
    if let Some(d) = &db {
        cmd.args(["--db", &d.to_string_lossy()]);
    }
    cmd.stdout(log_f).stderr(log_f2).stdin(Stdio::null());
    // Detach from the terminal's process group so the child survives terminal close.
    #[cfg(unix)]
    cmd.process_group(0);

    let child = cmd.spawn()?;
    let pid = child.id();

    if let Some(parent) = pid_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&pid_path, pid.to_string())?;

    println!("byokey started (pid {pid})");
    println!("logs: {}", log_path.display());
    Ok(())
}

fn cmd_stop() -> Result<()> {
    let pid_path = default_pid_path();
    let pid_str = std::fs::read_to_string(&pid_path)
        .map_err(|_| anyhow::anyhow!("byokey is not running (PID file not found)"))?;
    let pid = pid_str.trim().to_owned();

    let ok = std::process::Command::new("kill")
        .arg(&pid)
        .status()
        .is_ok_and(|s| s.success());
    let _ = std::fs::remove_file(&pid_path);

    if ok {
        println!("byokey stopped (pid {pid})");
        Ok(())
    } else {
        Err(anyhow::anyhow!("failed to stop process {pid}"))
    }
}

fn cmd_restart(
    config_path: Option<PathBuf>,
    port: Option<u16>,
    host: Option<String>,
    db: Option<PathBuf>,
    log_file: Option<PathBuf>,
) -> Result<()> {
    if let Err(e) = cmd_stop() {
        eprintln!("stop: {e}");
    }
    std::thread::sleep(std::time::Duration::from_millis(500));
    cmd_start(config_path, port, host, db, log_file)
}

// ── Autostart — macOS (launchd) ───────────────────────────────────────────────

#[cfg(target_os = "macos")]
fn launchd_plist_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{LAUNCHD_LABEL}.plist"))
}

#[cfg(target_os = "macos")]
fn autostart_enable_impl(
    config_path: Option<PathBuf>,
    port: Option<u16>,
    host: Option<String>,
    db: Option<PathBuf>,
    log_file: Option<PathBuf>,
) -> Result<()> {
    let exe = std::env::current_exe()?;
    let log_path = log_file.unwrap_or_else(default_log_path);
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Build the <array> of ProgramArguments entries.
    let mut args = format!(
        "        <string>{}</string>\n        <string>serve</string>\n",
        exe.to_string_lossy()
    );
    if let Some(p) = &config_path {
        args.push_str(&format!(
            "        <string>--config</string>\n        <string>{}</string>\n",
            p.to_string_lossy()
        ));
    }
    if let Some(p) = port {
        args.push_str(&format!(
            "        <string>--port</string>\n        <string>{p}</string>\n"
        ));
    }
    if let Some(h) = &host {
        args.push_str(&format!(
            "        <string>--host</string>\n        <string>{h}</string>\n"
        ));
    }
    if let Some(d) = &db {
        args.push_str(&format!(
            "        <string>--db</string>\n        <string>{}</string>\n",
            d.to_string_lossy()
        ));
    }

    let log = log_path.to_string_lossy();
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LAUNCHD_LABEL}</string>
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

    let plist_path = launchd_plist_path();
    if let Some(parent) = plist_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&plist_path, plist)?;

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
        println!("autostart enabled (launchd)");
        println!("plist: {}", plist_path.display());
        Ok(())
    } else {
        Err(anyhow::anyhow!("launchctl load failed"))
    }
}

#[cfg(target_os = "macos")]
fn autostart_disable_impl() -> Result<()> {
    let plist_path = launchd_plist_path();
    if !plist_path.exists() {
        anyhow::bail!("autostart is not enabled");
    }
    let _ = std::process::Command::new("launchctl")
        .args(["unload", &plist_path.to_string_lossy()])
        .status();
    std::fs::remove_file(&plist_path)?;
    println!("autostart disabled");
    Ok(())
}

#[cfg(target_os = "macos")]
fn autostart_status_impl() -> Result<()> {
    let plist_path = launchd_plist_path();
    if !plist_path.exists() {
        println!("autostart: disabled");
        return Ok(());
    }
    println!("autostart: enabled");
    println!("plist:     {}", plist_path.display());
    let running = std::process::Command::new("launchctl")
        .args(["list", LAUNCHD_LABEL])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    println!(
        "service:   {}",
        if running { "running" } else { "not running" }
    );
    Ok(())
}

// ── Autostart — Linux (systemd user) ─────────────────────────────────────────

#[cfg(target_os = "linux")]
fn systemd_unit_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("systemd")
        .join("user")
        .join(format!("{SYSTEMD_UNIT}.service"))
}

#[cfg(target_os = "linux")]
fn autostart_enable_impl(
    config_path: Option<PathBuf>,
    port: Option<u16>,
    host: Option<String>,
    db: Option<PathBuf>,
    log_file: Option<PathBuf>,
) -> Result<()> {
    let exe = std::env::current_exe()?;
    let log_path = log_file.unwrap_or_else(default_log_path);
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut exec_args = vec![exe.to_string_lossy().into_owned(), "serve".to_owned()];
    if let Some(p) = &config_path {
        exec_args.extend(["--config".to_owned(), p.to_string_lossy().into_owned()]);
    }
    if let Some(p) = port {
        exec_args.extend(["--port".to_owned(), p.to_string()]);
    }
    if let Some(h) = &host {
        exec_args.extend(["--host".to_owned(), h.clone()]);
    }
    if let Some(d) = &db {
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

    let unit_path = systemd_unit_path();
    if let Some(parent) = unit_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&unit_path, unit)?;

    let _ = std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();

    let ok = std::process::Command::new("systemctl")
        .args(["--user", "enable", "--now", SYSTEMD_UNIT])
        .status()
        .is_ok_and(|s| s.success());

    if ok {
        println!("autostart enabled (systemd user)");
        println!("unit: {}", unit_path.display());
        Ok(())
    } else {
        Err(anyhow::anyhow!("systemctl enable failed"))
    }
}

#[cfg(target_os = "linux")]
fn autostart_disable_impl() -> Result<()> {
    let unit_path = systemd_unit_path();
    if !unit_path.exists() {
        anyhow::bail!("autostart is not enabled");
    }
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "disable", "--now", SYSTEMD_UNIT])
        .status();
    std::fs::remove_file(&unit_path)?;
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();
    println!("autostart disabled");
    Ok(())
}

#[cfg(target_os = "linux")]
fn autostart_status_impl() -> Result<()> {
    let unit_path = systemd_unit_path();
    if !unit_path.exists() {
        println!("autostart: disabled");
        return Ok(());
    }
    println!("autostart: enabled");
    println!("unit:      {}", unit_path.display());
    let running = std::process::Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", SYSTEMD_UNIT])
        .status()
        .is_ok_and(|s| s.success());
    println!(
        "service:   {}",
        if running { "running" } else { "not running" }
    );
    Ok(())
}

// ── Autostart — unsupported platforms ────────────────────────────────────────

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn autostart_enable_impl(
    _config_path: Option<PathBuf>,
    _port: Option<u16>,
    _host: Option<String>,
    _db: Option<PathBuf>,
    _log_file: Option<PathBuf>,
) -> Result<()> {
    Err(anyhow::anyhow!(
        "autostart is not supported on this platform"
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn autostart_disable_impl() -> Result<()> {
    Err(anyhow::anyhow!(
        "autostart is not supported on this platform"
    ))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn autostart_status_impl() -> Result<()> {
    Err(anyhow::anyhow!(
        "autostart is not supported on this platform"
    ))
}

// ── Auth commands ─────────────────────────────────────────────────────────────

async fn cmd_login(provider_str: String, db: Option<PathBuf>) -> Result<()> {
    let provider = provider_str
        .parse::<ProviderId>()
        .map_err(|e| anyhow::anyhow!("unknown provider '{provider_str}': {e}"))?;
    let auth = AuthManager::new(Arc::new(open_store(db).await?));
    byokey_auth::flow::login(&provider, &auth)
        .await
        .map_err(|e| anyhow::anyhow!("login failed: {e}"))?;
    Ok(())
}

async fn cmd_logout(provider_str: String, db: Option<PathBuf>) -> Result<()> {
    let provider = provider_str
        .parse::<ProviderId>()
        .map_err(|e| anyhow::anyhow!("unknown provider '{provider_str}': {e}"))?;
    let auth = AuthManager::new(Arc::new(open_store(db).await?));
    auth.remove_token(&provider)
        .await
        .map_err(|e| anyhow::anyhow!("logout failed: {e}"))?;
    eprintln!("{provider_str} logged out");
    Ok(())
}

async fn cmd_status(db: Option<PathBuf>) -> Result<()> {
    let auth = AuthManager::new(Arc::new(open_store(db).await?));
    let providers = [
        ProviderId::Claude,
        ProviderId::Codex,
        ProviderId::Copilot,
        ProviderId::Gemini,
        ProviderId::Kiro,
        ProviderId::Antigravity,
        ProviderId::Qwen,
        ProviderId::Kimi,
        ProviderId::IFlow,
    ];
    for provider in &providers {
        let status = if auth.is_authenticated(provider).await {
            "authenticated"
        } else {
            "not authenticated"
        };
        println!("{provider}: {status}");
    }
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

async fn open_store(db: Option<PathBuf>) -> Result<SqliteTokenStore> {
    let path = db.unwrap_or_else(default_db_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let url = format!("sqlite://{}", path.display());
    SqliteTokenStore::new(&url)
        .await
        .map_err(|e| anyhow::anyhow!("database error: {e}"))
}

fn default_config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("byokey")
        .join("settings.json")
}

fn default_db_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".byokey").join("tokens.db")
}

fn default_pid_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".byokey").join("byokey.pid")
}

fn default_log_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".byokey").join("server.log")
}
