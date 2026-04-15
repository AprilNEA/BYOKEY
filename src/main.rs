mod amp;
mod auth;
mod control_server;
mod daemon;
mod serve;

use anyhow::Result;
use byokey_store::SqliteTokenStore;
use byokey_types::ProviderId;
use clap::{CommandFactory, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "byokey",
    about = "BYOKEY — Bring Your Own Keys AI proxy",
    version
)]
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
    /// Log file path. If set, logs are written to this file with daily rotation.
    #[arg(long, value_name = "PATH")]
    log_file: Option<PathBuf>,
}

/// Extended server arguments for background/daemon modes.
#[derive(clap::Args, Debug)]
struct DaemonArgs {
    #[command(flatten)]
    server: ServerArgs,
}

/// Shared arguments for commands that access the token store.
#[derive(clap::Args, Debug)]
struct StoreArgs {
    /// SQLite database path (default: ~/.byokey/tokens.db).
    #[arg(long, value_name = "PATH")]
    db: Option<PathBuf>,
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
    /// Reload the running server's configuration without restarting.
    Reload,
    /// Manage OS-level service registration (launchd / systemd / Windows SCM).
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
    /// Authenticate with a provider.
    Login {
        /// Provider name.
        provider: ProviderId,
        /// Account identifier (e.g. `work`, `personal`). Defaults to `default`.
        #[arg(long, value_name = "NAME")]
        account: Option<String>,
        #[command(flatten)]
        store: StoreArgs,
    },
    /// Remove stored credentials for a provider.
    Logout {
        /// Provider name.
        provider: ProviderId,
        /// Account identifier. If omitted, removes the active account.
        #[arg(long, value_name = "NAME")]
        account: Option<String>,
        #[command(flatten)]
        store: StoreArgs,
    },
    /// Show authentication status for all providers.
    Status {
        #[command(flatten)]
        store: StoreArgs,
    },
    /// List all accounts for a provider.
    Accounts {
        /// Provider name.
        provider: ProviderId,
        #[command(flatten)]
        store: StoreArgs,
    },
    /// Switch the active account for a provider.
    Switch {
        /// Provider name.
        provider: ProviderId,
        /// Account identifier to make active.
        account: String,
        #[command(flatten)]
        store: StoreArgs,
    },
    /// Amp-related utilities.
    Amp {
        #[command(subcommand)]
        action: AmpAction,
    },
    /// Export the OpenAPI specification as JSON.
    Openapi,
    /// Generate shell completions.
    Completions {
        /// Shell to generate completions for.
        shell: clap_complete::Shell,
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
    /// Manage Amp ads patching.
    Ads {
        #[command(subcommand)]
        action: AdsAction,
    },
}

#[derive(Subcommand, Debug)]
enum AdsAction {
    /// Patch Amp to hide ads (preserves impression telemetry).
    Disable {
        /// Explicit path(s) to the bundle file. Auto-detected if omitted.
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,
        /// Also patch editor extensions (VS Code, Cursor, Windsurf).
        #[arg(long, conflicts_with = "all")]
        extension: bool,
        /// Patch both CLI binary and editor extensions.
        #[arg(long, conflicts_with = "extension")]
        all: bool,
    },
    /// Restore original Amp files (re-enable ads).
    Enable {
        /// Explicit path(s) to the bundle file. Auto-detected if omitted.
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
enum ServiceAction {
    /// Install byokey as an OS-managed service and start it.
    Install {
        #[command(flatten)]
        daemon: DaemonArgs,
    },
    /// Uninstall the OS-managed service.
    Uninstall,
    /// Start the installed service.
    Start,
    /// Stop the installed service.
    Stop,
    /// Show the OS-managed service's registration and running state.
    Status,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Serve { server } => serve::cmd_serve(server).await,
        Commands::Start { daemon } => daemon::cmd_start(daemon),
        Commands::Stop => daemon::cmd_stop(),
        Commands::Restart { daemon } => daemon::cmd_restart(daemon),
        Commands::Reload => daemon::cmd_reload(),
        Commands::Service { action } => match action {
            ServiceAction::Install { daemon } => daemon::cmd_service_install(daemon),
            ServiceAction::Uninstall => daemon::cmd_service_uninstall(),
            ServiceAction::Start => daemon::cmd_service_start(),
            ServiceAction::Stop => daemon::cmd_service_stop(),
            ServiceAction::Status => daemon::cmd_service_status(),
        },
        Commands::Login {
            provider,
            account,
            store,
        } => auth::cmd_login(provider, account, store.db).await,
        Commands::Logout {
            provider,
            account,
            store,
        } => auth::cmd_logout(provider, account, store.db).await,
        Commands::Status { store } => auth::cmd_status(store.db).await,
        Commands::Accounts { provider, store } => auth::cmd_accounts(provider, store.db).await,
        Commands::Switch {
            provider,
            account,
            store,
        } => auth::cmd_switch(provider, account, store.db).await,
        Commands::Amp { action } => match action {
            AmpAction::Inject { url } => amp::cmd_amp_inject(url),
            AmpAction::Ads { action } => match action {
                AdsAction::Disable {
                    paths,
                    extension,
                    all,
                } => amp::cmd_ads_disable(paths, extension || all),
                AdsAction::Enable { paths } => amp::cmd_ads_enable(paths),
            },
        },
        Commands::Openapi => {
            use utoipa::OpenApi as _;
            let spec = byokey_proxy::ApiDoc::openapi()
                .to_json()
                .expect("OpenAPI spec serialization failed");
            println!("{spec}");
            Ok(())
        }
        Commands::Completions { shell } => {
            clap_complete::generate(shell, &mut Cli::command(), "byokey", &mut std::io::stdout());
            Ok(())
        }
    }
}

pub(crate) async fn open_store(db: Option<PathBuf>) -> Result<SqliteTokenStore> {
    let path = match db {
        Some(p) => p,
        None => byokey_daemon::paths::db_path()?,
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let url = format!("sqlite://{}?mode=rwc", path.display());
    SqliteTokenStore::new(&url)
        .await
        .map_err(|e| anyhow::anyhow!("database error: {e}"))
}
