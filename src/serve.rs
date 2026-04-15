use anyhow::Result;
use arc_swap::ArcSwap;
use byokey_auth::AuthManager;
use byokey_config::{Config, ConfigWatcher, LogConfig, LogFormat};
use byokey_proxy::AppState;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Notify;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::writer::BoxMakeWriter;

use crate::ServerArgs;
use crate::control_server::{self, ControlState};

fn init_logging(cfg: &LogConfig, log_file: Option<PathBuf>) -> Option<WorkerGuard> {
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&cfg.level));

    let path = log_file
        .map(|p| p.to_string_lossy().into_owned())
        .or_else(|| cfg.file.clone());

    let (writer, guard): (BoxMakeWriter, Option<WorkerGuard>) = if let Some(p) = &path {
        let dir = Path::new(p).parent().unwrap_or_else(|| Path::new("."));
        let name = Path::new(p)
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("byokey.log"));
        let (nb, g) = tracing_appender::non_blocking(rolling::daily(dir, name));
        (BoxMakeWriter::new(nb), Some(g))
    } else {
        (BoxMakeWriter::new(std::io::stdout), None)
    };

    let builder = tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(true)
        .with_ansi(path.is_none())
        .with_writer(writer);

    match cfg.format {
        LogFormat::Json => builder.json().init(),
        LogFormat::Text => builder.init(),
    }

    guard
}

pub async fn cmd_serve(args: ServerArgs) -> Result<()> {
    let ServerArgs {
        config: config_path,
        port,
        host,
        db,
        log_file,
    } = args;
    let effective_path = config_path.or_else(|| {
        let default = byokey_daemon::paths::config_path().ok()?;
        if default.exists() {
            Some(default)
        } else {
            None
        }
    });

    // Load config first so we can use log settings.
    let (config_arc, config_watcher): (Arc<ArcSwap<Config>>, Option<Arc<ConfigWatcher>>) =
        if let Some(ref path) = effective_path {
            let watcher = Arc::new(
                ConfigWatcher::new(path.clone())
                    .map_err(|e| anyhow::anyhow!("config error: {e}"))?,
            );
            let arc = watcher.arc();
            Arc::clone(&watcher).watch();
            (arc, Some(watcher))
        } else {
            (Arc::new(ArcSwap::from_pointee(Config::default())), None)
        };

    let snapshot = config_arc.load();

    // _log_guard must be held until server exits to flush buffered writes.
    let _log_guard = init_logging(&snapshot.log, log_file);

    // CLI overrides for listen address.
    let effective_host = host.as_deref().unwrap_or(&snapshot.host).to_owned();
    let effective_port = port.unwrap_or(snapshot.port);
    let addr = format!("{effective_host}:{effective_port}");

    let store = Arc::new(crate::open_store(db).await?);
    let auth = Arc::new(AuthManager::new(store.clone(), rquest::Client::new()));
    let usage_store: Arc<dyn byokey_types::UsageStore> = store;
    let state = AppState::new(Arc::clone(&config_arc), auth, Some(usage_store.clone()));

    // Pre-load cumulative usage from persisted records so the in-memory snapshot
    // reflects historical totals even after a restart.
    if let Ok(totals) = usage_store.totals(None, None).await {
        for bucket in &totals {
            state.usage.preload(
                &bucket.model,
                bucket.request_count,
                bucket.input_tokens,
                bucket.output_tokens,
            );
        }
    }
    let app = byokey_proxy::make_router(state);

    // ── Control socket + unified shutdown signal ───────────────────────────
    let shutdown = Arc::new(Notify::new());
    let sock_path = byokey_daemon::paths::control_sock_path()
        .map_err(|e| anyhow::anyhow!("control socket path: {e}"))?;

    // Refuse to start if another instance is already answering the socket.
    if byokey_daemon::control::is_alive() {
        return Err(anyhow::anyhow!(
            "another byokey serve is already running (control socket {} is live)",
            sock_path.display()
        ));
    }

    let ctl_state = Arc::new(ControlState {
        watcher: config_watcher,
        shutdown: Arc::clone(&shutdown),
        start: Instant::now(),
        host: effective_host.clone(),
        port: effective_port,
    });
    let ctl_handle = control_server::bind_and_serve(sock_path.clone(), ctl_state)
        .map_err(|e| anyhow::anyhow!("bind control socket {}: {e}", sock_path.display()))?;
    tracing::info!(socket = %sock_path.display(), "control socket ready");

    spawn_signal_handler(Arc::clone(&shutdown));

    // Check for TLS configuration.
    let tls_config = snapshot.tls.clone().filter(|t| t.enable);
    drop(snapshot);

    let serve_result = if let Some(tls) = tls_config {
        let rustls_config =
            axum_server::tls_rustls::RustlsConfig::from_pem_file(&tls.cert, &tls.key)
                .await
                .map_err(|e| anyhow::anyhow!("TLS config error: {e}"))?;
        let sock_addr: std::net::SocketAddr = addr
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid address: {e}"))?;
        let handle = axum_server::Handle::new();
        let handle_for_shutdown = handle.clone();
        let shutdown_for_task = Arc::clone(&shutdown);
        tokio::spawn(async move {
            shutdown_for_task.notified().await;
            handle_for_shutdown.graceful_shutdown(Some(std::time::Duration::from_secs(10)));
        });
        tracing::info!(%sock_addr, "byokey listening (TLS)");
        axum_server::bind_rustls(sock_addr, rustls_config)
            .handle(handle)
            .serve(app.into_make_service())
            .await
            .map_err(anyhow::Error::from)
    } else {
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        tracing::info!(addr = %addr, "byokey listening");
        let shutdown_for_serve = Arc::clone(&shutdown);
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                shutdown_for_serve.notified().await;
            })
            .await
            .map_err(anyhow::Error::from)
    };

    ctl_handle.cleanup();
    serve_result
}

fn spawn_signal_handler(shutdown: Arc<Notify>) {
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            let mut sigterm = match signal(SignalKind::terminate()) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(error = %e, "install SIGTERM handler failed");
                    return;
                }
            };
            let mut sigint = match signal(SignalKind::interrupt()) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(error = %e, "install SIGINT handler failed");
                    return;
                }
            };
            tokio::select! {
                _ = sigterm.recv() => tracing::info!("received SIGTERM"),
                _ = sigint.recv() => tracing::info!("received SIGINT"),
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("received Ctrl-C");
        }
        shutdown.notify_waiters();
    });
}
