use anyhow::Result;
use byokey_auth::AuthManager;
use byokey_config::Config;
use byokey_proxy::AppState;
use byokey_store::SqliteTokenStore;
use byokey_types::ProviderId;
use clap::{Parser, Subcommand};
use std::{path::PathBuf, sync::Arc};

#[derive(Parser, Debug)]
#[command(name = "byokey", about = "byokey — Bring Your Own Keys AI proxy")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Start the proxy server.
    Serve {
        /// Path to the YAML configuration file.
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
        Commands::Login { provider, db } => cmd_login(provider, db).await,
        Commands::Logout { provider, db } => cmd_logout(provider, db).await,
        Commands::Status { db } => cmd_status(db).await,
    }
}

async fn cmd_serve(
    config_path: Option<PathBuf>,
    port: Option<u16>,
    host: Option<String>,
    db: Option<PathBuf>,
) -> Result<()> {
    let mut config = if let Some(path) = &config_path {
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

fn default_db_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".byokey").join("tokens.db")
}
