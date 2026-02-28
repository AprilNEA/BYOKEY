use anyhow::Result;
use arc_swap::ArcSwap;
use byokey_auth::AuthManager;
use byokey_config::{Config, ConfigWatcher};
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

/// Common server arguments shared across serve/start/restart/autostart commands.
#[derive(clap::Args, Debug)]
struct ServerArgs {
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
}

/// Extended server arguments that include a log file path (for background/daemon modes).
#[derive(clap::Args, Debug)]
struct DaemonArgs {
    #[command(flatten)]
    server: ServerArgs,
    /// Log file path (default: ~/.byokey/server.log).
    #[arg(long, value_name = "PATH")]
    log_file: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Start the proxy server (foreground).
    Serve {
        #[command(flatten)]
        server: ServerArgs,
    },
    /// Start the proxy server in the background.
    Start {
        #[command(flatten)]
        daemon: DaemonArgs,
    },
    /// Stop the background proxy server.
    Stop,
    /// Restart the background proxy server.
    Restart {
        #[command(flatten)]
        daemon: DaemonArgs,
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
        /// Account identifier (e.g. `work`, `personal`). Defaults to `default`.
        #[arg(long, value_name = "NAME")]
        account: Option<String>,
        /// SQLite database path (default: ~/.byokey/tokens.db).
        #[arg(long, value_name = "PATH")]
        db: Option<PathBuf>,
    },
    /// Remove stored credentials for a provider.
    Logout {
        /// Provider name.
        provider: String,
        /// Account identifier. If omitted, removes the active account.
        #[arg(long, value_name = "NAME")]
        account: Option<String>,
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
    /// List all accounts for a provider.
    Accounts {
        /// Provider name.
        provider: String,
        /// SQLite database path (default: ~/.byokey/tokens.db).
        #[arg(long, value_name = "PATH")]
        db: Option<PathBuf>,
    },
    /// Switch the active account for a provider.
    Switch {
        /// Provider name.
        provider: String,
        /// Account identifier to make active.
        account: String,
        /// SQLite database path (default: ~/.byokey/tokens.db).
        #[arg(long, value_name = "PATH")]
        db: Option<PathBuf>,
    },
    /// Amp-related utilities.
    Amp {
        #[command(subcommand)]
        action: AmpAction,
    },
}

#[derive(Subcommand, Debug)]
enum AmpAction {
    /// Inject the byokey proxy URL into Amp configuration.
    Inject {
        /// The proxy URL to inject (default: http://localhost:8018).
        #[arg(long)]
        url: Option<String>,
    },
    /// Patch Amp to hide ads (preserves impression telemetry).
    DisableAds {
        /// Explicit path(s) to the bundle file. Auto-detected if omitted.
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,
        /// Restore the original bundle from backup.
        #[arg(long)]
        restore: bool,
    },
}

#[derive(Subcommand, Debug)]
enum AutostartAction {
    /// Register byokey as a boot-time service.
    Enable {
        #[command(flatten)]
        daemon: DaemonArgs,
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
        Commands::Serve { server } => {
            cmd_serve(server.config, server.port, server.host, server.db).await
        }
        Commands::Start { daemon } => cmd_start(
            daemon.server.config,
            daemon.server.port,
            daemon.server.host,
            daemon.server.db,
            daemon.log_file,
        ),
        Commands::Stop => cmd_stop(),
        Commands::Restart { daemon } => cmd_restart(
            daemon.server.config,
            daemon.server.port,
            daemon.server.host,
            daemon.server.db,
            daemon.log_file,
        ),
        Commands::Autostart { action } => match action {
            AutostartAction::Enable { daemon } => autostart_enable_impl(
                daemon.server.config,
                daemon.server.port,
                daemon.server.host,
                daemon.server.db,
                daemon.log_file,
            ),
            AutostartAction::Disable => autostart_disable_impl(),
            AutostartAction::Status => autostart_status_impl(),
        },
        Commands::Login {
            provider,
            account,
            db,
        } => cmd_login(provider, account, db).await,
        Commands::Logout {
            provider,
            account,
            db,
        } => cmd_logout(provider, account, db).await,
        Commands::Status { db } => cmd_status(db).await,
        Commands::Accounts { provider, db } => cmd_accounts(provider, db).await,
        Commands::Switch {
            provider,
            account,
            db,
        } => cmd_switch(provider, account, db).await,
        Commands::Amp { action } => match action {
            AmpAction::Inject { url } => cmd_amp_inject(url),
            AmpAction::DisableAds { paths, restore } => cmd_amp_disable_ads(paths, restore),
        },
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

    // Load config first so we can use log settings.
    let config_arc: Arc<ArcSwap<Config>> = if let Some(ref path) = effective_path {
        let watcher = Arc::new(
            ConfigWatcher::new(path.clone()).map_err(|e| anyhow::anyhow!("config error: {e}"))?,
        );
        let arc = watcher.arc();
        watcher.watch();
        arc
    } else {
        Arc::new(ArcSwap::from_pointee(Config::default()))
    };

    let snapshot = config_arc.load();

    // Initialize structured logging based on config.
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&snapshot.log.level));

    // _log_guard must be held until server exits to flush buffered writes.
    let _log_guard: Option<tracing_appender::non_blocking::WorkerGuard>;

    if let Some(ref log_path) = snapshot.log.file {
        let path = std::path::Path::new(log_path);
        let dir = path.parent().unwrap_or(std::path::Path::new("."));
        let filename = path
            .file_name()
            .unwrap_or(std::ffi::OsStr::new("byokey.log"));
        let file_appender = tracing_appender::rolling::daily(dir, filename);
        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
        _log_guard = Some(guard);

        if snapshot.log.format == "json" {
            tracing_subscriber::fmt()
                .json()
                .with_env_filter(env_filter)
                .with_target(true)
                .with_writer(non_blocking)
                .init();
        } else {
            tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .with_target(true)
                .with_writer(non_blocking)
                .init();
        }
    } else {
        _log_guard = None;
        if snapshot.log.format == "json" {
            tracing_subscriber::fmt()
                .json()
                .with_env_filter(env_filter)
                .with_target(true)
                .init();
        } else {
            tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .with_target(true)
                .init();
        }
    }

    // CLI overrides for listen address.
    let addr = format!(
        "{}:{}",
        host.as_deref().unwrap_or(&snapshot.host),
        port.unwrap_or(snapshot.port),
    );

    let auth = Arc::new(AuthManager::new(
        Arc::new(open_store(db).await?),
        rquest::Client::new(),
    ));
    let state = AppState::new(config_arc, auth);
    let app = byokey_proxy::make_router(state);

    // Check for TLS configuration.
    let tls_config = snapshot.tls.as_ref().filter(|t| t.enable);

    if let Some(tls) = tls_config {
        let rustls_config =
            axum_server::tls_rustls::RustlsConfig::from_pem_file(&tls.cert, &tls.key)
                .await
                .map_err(|e| anyhow::anyhow!("TLS config error: {e}"))?;
        let addr: std::net::SocketAddr = addr
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid address: {e}"))?;
        tracing::info!(%addr, "byokey listening (TLS)");
        axum_server::bind_rustls(addr, rustls_config)
            .serve(app.into_make_service())
            .await?;
    } else {
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        tracing::info!(addr = %addr, "byokey listening");
        axum::serve(listener, app).await?;
    }
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

async fn cmd_login(
    provider_str: String,
    account: Option<String>,
    db: Option<PathBuf>,
) -> Result<()> {
    let provider = provider_str
        .parse::<ProviderId>()
        .map_err(|e| anyhow::anyhow!("unknown provider '{provider_str}': {e}"))?;
    let auth = AuthManager::new(Arc::new(open_store(db).await?), rquest::Client::new());
    byokey_auth::flow::login(&provider, &auth, account.as_deref())
        .await
        .map_err(|e| anyhow::anyhow!("login failed: {e}"))?;
    Ok(())
}

async fn cmd_logout(
    provider_str: String,
    account: Option<String>,
    db: Option<PathBuf>,
) -> Result<()> {
    let provider = provider_str
        .parse::<ProviderId>()
        .map_err(|e| anyhow::anyhow!("unknown provider '{provider_str}': {e}"))?;
    let auth = AuthManager::new(Arc::new(open_store(db).await?), rquest::Client::new());
    if let Some(account_id) = &account {
        auth.remove_token_for(&provider, account_id)
            .await
            .map_err(|e| anyhow::anyhow!("logout failed: {e}"))?;
        println!("{provider_str} account '{account_id}' logged out");
    } else {
        auth.remove_token(&provider)
            .await
            .map_err(|e| anyhow::anyhow!("logout failed: {e}"))?;
        println!("{provider_str} logged out");
    }
    Ok(())
}

async fn cmd_status(db: Option<PathBuf>) -> Result<()> {
    let auth = AuthManager::new(Arc::new(open_store(db).await?), rquest::Client::new());
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
        ProviderId::Factory,
    ];
    for provider in &providers {
        let accounts = auth.list_accounts(provider).await.unwrap_or_default();
        if accounts.is_empty() {
            println!("{provider}: not authenticated");
        } else if accounts.len() == 1 {
            let status = if auth.is_authenticated(provider).await {
                "authenticated"
            } else {
                "expired"
            };
            println!("{provider}: {status}");
        } else {
            let active = accounts.iter().find(|a| a.is_active);
            let label = active
                .and_then(|a| a.label.as_deref())
                .unwrap_or_else(|| active.map_or("?", |a| a.account_id.as_str()));
            println!("{provider}: {} account(s), active: {label}", accounts.len());
        }
    }
    Ok(())
}

async fn cmd_accounts(provider_str: String, db: Option<PathBuf>) -> Result<()> {
    let provider = provider_str
        .parse::<ProviderId>()
        .map_err(|e| anyhow::anyhow!("unknown provider '{provider_str}': {e}"))?;
    let auth = AuthManager::new(Arc::new(open_store(db).await?), rquest::Client::new());
    let accounts = auth
        .list_accounts(&provider)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    if accounts.is_empty() {
        println!("{provider}: no accounts");
    } else {
        for a in &accounts {
            let marker = if a.is_active { " (active)" } else { "" };
            let label = a
                .label
                .as_deref()
                .map_or(String::new(), |l| format!(" [{l}]"));
            println!("  {}{label}{marker}", a.account_id);
        }
    }
    Ok(())
}

async fn cmd_switch(provider_str: String, account: String, db: Option<PathBuf>) -> Result<()> {
    let provider = provider_str
        .parse::<ProviderId>()
        .map_err(|e| anyhow::anyhow!("unknown provider '{provider_str}': {e}"))?;
    let auth = AuthManager::new(Arc::new(open_store(db).await?), rquest::Client::new());
    auth.set_active_account(&provider, &account)
        .await
        .map_err(|e| anyhow::anyhow!("switch failed: {e}"))?;
    println!("{provider_str}: switched to account '{account}'");
    Ok(())
}

// ── Amp commands ─────────────────────────────────────────────────────────────

fn cmd_amp_inject(url: Option<String>) -> Result<()> {
    let url = url.unwrap_or_else(|| "http://localhost:8018/amp".to_string());
    let settings_path = amp_settings_path();

    if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Read existing settings or start with an empty object.
    let mut map: serde_json::Map<String, serde_json::Value> = if settings_path.exists() {
        let content = std::fs::read_to_string(&settings_path)?;
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        serde_json::Map::new()
    };

    map.insert(
        "amp.url".to_string(),
        serde_json::Value::String(url.clone()),
    );

    let json = serde_json::to_string_pretty(&serde_json::Value::Object(map))?;
    std::fs::write(&settings_path, format!("{json}\n"))?;
    println!("amp.url set to {url}");
    println!("config: {}", settings_path.display());
    Ok(())
}

fn cmd_amp_disable_ads(paths: Vec<PathBuf>, restore: bool) -> Result<()> {
    let bundles = if paths.is_empty() {
        println!("searching for amp bundle...");
        let found = find_amp_bundles();
        if found.is_empty() {
            println!("no amp bundle found — falling back to hide_free_tier");
            set_hide_free_tier(true)?;
            return Ok(());
        }
        for p in &found {
            println!("  found: {}", p.display());
        }
        found
    } else {
        paths
    };

    if restore {
        for bundle_path in &bundles {
            println!("\nrestoring: {}", bundle_path.display());
            amp_restore(bundle_path)?;
        }
        println!("\nrestart amp / reload editor window to apply.");
        return Ok(());
    }

    let mut any_patched = false;
    let mut all_failed = true;

    for bundle_path in &bundles {
        println!("\npatching: {}", bundle_path.display());

        let data = match std::fs::read(bundle_path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("  ERROR reading file: {e}");
                continue;
            }
        };
        println!("  size: {} bytes", data.len());

        match amp_patch(&data) {
            Ok(Some(patched)) => {
                let bak = bundle_path.with_extension("js.bak");
                if !bak.exists() {
                    std::fs::copy(bundle_path, &bak)?;
                    println!("  backup saved: {}", bak.display());
                }
                std::fs::write(bundle_path, patched)?;
                println!("  patched successfully");
                resign_adhoc(bundle_path);
                any_patched = true;
                all_failed = false;
            }
            Ok(None) => {
                println!("  already patched — skipping");
                all_failed = false;
            }
            Err(e) => eprintln!("  ERROR: {e}"),
        }
    }

    if all_failed {
        println!("\nbinary patch failed — enabling hide_free_tier as fallback");
        set_hide_free_tier(true)?;
    } else if any_patched {
        println!("\nrestart amp / reload editor window to apply.");
    }

    Ok(())
}

/// Enable or disable `amp.hide_free_tier` in the byokey config file.
fn set_hide_free_tier(enabled: bool) -> Result<()> {
    let config_path = default_config_path();

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut map: serde_json::Map<String, serde_json::Value> = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)?;
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        serde_json::Map::new()
    };

    // Ensure `amp` object exists.
    let amp = map
        .entry("amp")
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    if let Some(obj) = amp.as_object_mut() {
        obj.insert(
            "hide_free_tier".to_string(),
            serde_json::Value::Bool(enabled),
        );
    }

    let json = serde_json::to_string_pretty(&serde_json::Value::Object(map))?;
    std::fs::write(&config_path, format!("{json}\n"))?;
    println!(
        "  amp.hide_free_tier = {enabled} in {}",
        config_path.display()
    );
    Ok(())
}

// ── Amp ad-patch helpers ─────────────────────────────────────────────────────

const AMP_PATCH_MARKER: &[u8] = b"/*ampatch*";

/// Discover patchable Amp bundles: CLI binary on PATH + editor extensions.
fn find_amp_bundles() -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();

    // 1. Resolve `amp` from PATH.
    if let Ok(output) = std::process::Command::new("which")
        .arg("amp")
        .stderr(Stdio::null())
        .output()
        && let Ok(raw) = std::str::from_utf8(&output.stdout)
    {
        let raw = raw.trim();
        if !raw.is_empty()
            && let Ok(real) = std::fs::canonicalize(raw)
            // Skip native binaries (Mach-O / ELF) — only patch JS text files.
            && !is_native_binary(&real)
            && has_ad_code(&real)
            && seen.insert(real.clone())
        {
            result.push(real);
        }
    }

    // 2. VS Code / Cursor / Windsurf extensions.
    if let Ok(home) = std::env::var("HOME") {
        let home = PathBuf::from(home);
        for editor_dir in &[".vscode", ".vscode-insiders", ".cursor", ".windsurf"] {
            let ext_root = home.join(editor_dir).join("extensions");
            if !ext_root.is_dir() {
                continue;
            }
            if let Ok(entries) = std::fs::read_dir(&ext_root) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if !name_str.starts_with("sourcegraph.amp-") {
                        continue;
                    }
                    if let Ok(walker) = glob_walk(&entry.path()) {
                        for js_file in walker {
                            if let Ok(meta) = js_file.metadata()
                                && meta.len() > 1_000_000
                                && let Ok(real) = std::fs::canonicalize(&js_file)
                                && has_ad_code(&real)
                                && seen.insert(real.clone())
                            {
                                result.push(real);
                            }
                        }
                    }
                }
            }
        }
    }

    result
}

/// Recursively yield `.js` files under `dir`.
fn glob_walk(dir: &std::path::Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if let Ok(sub) = glob_walk(&path) {
                files.extend(sub);
            }
        } else if path.extension().is_some_and(|e| e == "js") {
            files.push(path);
        }
    }
    Ok(files)
}

/// Ad-hoc re-sign a binary after patching (macOS only).
/// Silently skips on non-macOS or if `codesign` is unavailable.
#[cfg(target_os = "macos")]
fn resign_adhoc(path: &std::path::Path) {
    let status = std::process::Command::new("codesign")
        .args(["--sign", "-", "--force", "--preserve-metadata=entitlements"])
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match status {
        Ok(s) if s.success() => println!("  re-signed (ad-hoc)"),
        Ok(s) => eprintln!("  WARNING: codesign exited with {s}"),
        Err(e) => eprintln!("  WARNING: codesign not available: {e}"),
    }
}

#[cfg(not(target_os = "macos"))]
fn resign_adhoc(_path: &std::path::Path) {}

/// Return `true` if the file starts with a Mach-O or ELF magic number.
fn is_native_binary(path: &std::path::Path) -> bool {
    use std::io::Read as _;
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut magic = [0u8; 4];
    if f.read_exact(&mut magic).is_err() {
        return false;
    }
    let m = u32::from_be_bytes(magic);
    // Mach-O: fat (0xcafebabe), 64-bit LE (0xcffaedfe), 32-bit BE (0xfeedface),
    //         64-bit BE (0xfeedfacf), 32-bit LE (0xcefaedfe)
    matches!(
        m,
        0xcafe_babe | 0xcffa_edfe | 0xfeed_face | 0xfeed_facf | 0xcefa_edfe
    ) || magic == [0x7f, b'E', b'L', b'F'] // ELF
}

fn has_ad_code(path: &std::path::Path) -> bool {
    std::fs::read(path)
        .map(|data| {
            data.windows(b"fireImpressionIfNeeded".len())
                .any(|w| w == b"fireImpressionIfNeeded")
        })
        .unwrap_or(false)
}

/// Patch the ad widget `build()` to return a zero-height spacer.
/// Returns `Ok(Some(patched))` on success, `Ok(None)` if already patched.
fn amp_patch(data: &[u8]) -> Result<Option<Vec<u8>>> {
    if data
        .windows(AMP_PATCH_MARKER.len())
        .any(|w| w == AMP_PATCH_MARKER)
    {
        return Ok(None);
    }

    // 1. Find spacer widget constructor: `new <Name>({height:0})`
    let spacer_re = regex::bytes::Regex::new(r"new (\w+)\(\{height:0\}\)").expect("valid regex");
    let spacer_match = spacer_re.captures(data).ok_or_else(|| {
        anyhow::anyhow!("cannot find spacer widget pattern  new <X>({{height:0}})")
    })?;
    let spacer = &spacer_match[1];
    println!(
        "  spacer widget constructor: {}",
        std::str::from_utf8(spacer).unwrap_or("?")
    );

    // 2. Anchor on `fireImpressionIfNeeded(){`
    let anchor_re = regex::bytes::Regex::new(r"fireImpressionIfNeeded\(\)\{").expect("valid regex");
    let anchor = anchor_re
        .find(data)
        .ok_or_else(|| anyhow::anyhow!("cannot find fireImpressionIfNeeded(){{"))?;
    println!(
        "  anchor: fireImpressionIfNeeded() at byte {}",
        anchor.start()
    );

    let fire_body_end = find_brace_match(data, anchor.end() - 1)?;

    // 3. Locate `build(<arg>){` immediately after.
    let search_window = 300;
    let search_end = (fire_body_end + search_window).min(data.len());
    let build_re = regex::bytes::Regex::new(r"build\(\w{1,4}\)\{").expect("valid regex");
    let build_match = build_re
        .find(&data[fire_body_end..search_end])
        .ok_or_else(|| {
            anyhow::anyhow!(
                "cannot find build() within {search_window} bytes after \
                 fireImpressionIfNeeded (byte {fire_body_end})"
            )
        })?;

    let body_open = fire_body_end + build_match.end() - 1; // position of `{`
    let body_close = find_brace_match(data, body_open)?; // matching `}`
    let body_len = body_close - body_open - 1; // bytes between { and }

    println!(
        "  build() body: {body_len} bytes  [{}..{body_close})",
        body_open + 1
    );

    // 4. Build same-length replacement: `return new <Spacer>({height:0})/*ampatch*<pad>*/`
    let mut ret_stmt = Vec::new();
    ret_stmt.extend_from_slice(b"return new ");
    ret_stmt.extend_from_slice(spacer);
    ret_stmt.extend_from_slice(b"({height:0})");
    ret_stmt.extend_from_slice(AMP_PATCH_MARKER);

    let suffix = b"*/";
    let pad = body_len
        .checked_sub(ret_stmt.len() + suffix.len())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "replacement ({} bytes) exceeds original body ({body_len} bytes)",
                ret_stmt.len() + suffix.len()
            )
        })?;

    let mut replacement = ret_stmt;
    replacement.resize(replacement.len() + pad, b' ');
    replacement.extend_from_slice(suffix);
    assert_eq!(replacement.len(), body_len);

    // 5. Splice.
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&data[..body_open + 1]);
    out.extend_from_slice(&replacement);
    out.extend_from_slice(&data[body_close..]);
    assert_eq!(out.len(), data.len(), "file length must not change");

    let preview_len = 80.min(replacement.len());
    let preview = String::from_utf8_lossy(&replacement[..preview_len]);
    let ellipsis = if replacement.len() > 80 { "..." } else { "" };
    println!("  injected: {preview}{ellipsis}");

    Ok(Some(out))
}

/// Match the `}` that balances the `{` at `start`, respecting string literals.
fn find_brace_match(data: &[u8], start: usize) -> Result<usize> {
    if data.get(start) != Some(&b'{') {
        anyhow::bail!("expected '{{' at byte {start}");
    }

    let mut depth: usize = 1;
    let mut pos = start + 1;
    let mut in_str: u8 = 0; // 0 = not in string; otherwise the quote char
    let mut esc = false;

    while pos < data.len() && depth > 0 {
        let b = data[pos];
        if esc {
            esc = false;
        } else if in_str != 0 {
            if b == b'\\' {
                esc = true;
            } else if b == in_str {
                in_str = 0;
            }
        } else {
            match b {
                b'"' | b'\'' | b'`' => in_str = b,
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
        }
        pos += 1;
    }

    if depth != 0 {
        anyhow::bail!("unmatched brace starting at byte {start}");
    }
    Ok(pos - 1)
}

/// Restore a bundle from its `.bak` backup.
fn amp_restore(bundle_path: &std::path::Path) -> Result<()> {
    let bak = bundle_path.with_extension("js.bak");
    if !bak.exists() {
        // Try the Python-style `.bak` extension too (appended, not replaced).
        let bak_alt = PathBuf::from(format!("{}.bak", bundle_path.display()));
        if bak_alt.exists() {
            std::fs::copy(&bak_alt, bundle_path)?;
            println!("  restored from {}", bak_alt.display());
            return Ok(());
        }
        anyhow::bail!("no backup found at {}", bak.display());
    }
    std::fs::copy(&bak, bundle_path)?;
    println!("  restored from {}", bak.display());
    Ok(())
}

fn amp_settings_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".amp").join("settings.json")
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
