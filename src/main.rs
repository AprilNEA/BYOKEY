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
        action: daemon::ServiceAction,
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
    /// Amp proxy injection.
    Amp {
        #[command(subcommand)]
        action: amp::AmpAction,
    },
    /// Export the OpenAPI specification as JSON.
    Openapi,
    /// Generate shell completions.
    Completions {
        /// Shell to generate completions for.
        shell: clap_complete::Shell,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let result: Result<()> = match cli.command {
        Commands::Serve { server } => serve::cmd_serve(server).await,
        Commands::Start { daemon } => daemon::cmd_start(daemon),
        Commands::Stop => daemon::cmd_stop(),
        Commands::Restart { daemon } => daemon::cmd_restart(daemon),
        Commands::Reload => daemon::cmd_reload(),
        Commands::Service { action } => daemon::cmd_service(action),
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
        Commands::Amp { action } => amp::cmd_amp(action),
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
    };

    // Skip tokio runtime drop to avoid hanging on spawn_blocking tasks that
    // can't exit (config watcher, thread index watcher). All real work is done
    // by this point; nothing is lost by exiting immediately.
    match result {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("Error: {e:#}");
            std::process::exit(1);
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
