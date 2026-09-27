use anyhow::{Context as _, Result};
use arc_swap::ArcSwap;
use byokey_auth::AuthManager;
use byokey_config::{Config, ConfigWatcher, LogConfig, LogFormat};
use byokey_proxy::AppState;
use std::env::VarError;
use std::io::IsTerminal as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Notify;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt::writer::BoxMakeWriter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{EnvFilter, Registry, reload};

use crate::ServerArgs;
use crate::actions::telemetry;
use crate::control_server::{self, ControlState};

/// The live log filter, which a configuration reload can replace.
type FilterHandle = reload::Handle<EnvFilter, Registry>;

/// The log filter: `RUST_LOG` (`env`) when set, else the configured
/// `log.level` directives. An invalid one is an error: logging at some other
/// level would hide the lines someone asked for.
///
/// `log.level` must also set a default level: a bare word that is not one
/// (`degub`) is read as a module name, which turns every other module off.
/// `RUST_LOG` may name modules alone, as it does while developing.
fn log_filter(env: Result<String, VarError>, configured: &str) -> Result<EnvFilter> {
    let directives = match env {
        Ok(directives) => return parse_filter("RUST_LOG", &directives),
        Err(VarError::NotPresent) => configured,
        Err(e @ VarError::NotUnicode(_)) => return Err(e).context("RUST_LOG"),
    };
    if !directives
        .split(',')
        .any(|d| d.trim().parse::<LevelFilter>().is_ok())
    {
        anyhow::bail!(
            "log.level `{directives}` sets no default level such as `info`; \
             a bare word is read as a module name"
        );
    }
    parse_filter("log.level", directives)
}

fn parse_filter(source: &str, directives: &str) -> Result<EnvFilter> {
    EnvFilter::builder()
        .parse(directives)
        .with_context(|| format!("invalid {source} `{directives}`"))
}

/// Whether to colour the log: only for a person at a terminal, so a log
/// file or a service's redirected stdout gets plain text, and never with
/// `NO_COLOR` set.
fn use_color(to_file: bool) -> bool {
    !to_file
        && std::io::stdout().is_terminal()
        && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty())
}

fn init_logging(
    cfg: &LogConfig,
    log_file: Option<PathBuf>,
) -> Result<(Option<WorkerGuard>, FilterHandle)> {
    let (filter, handle) = reload::Layer::new(log_filter(std::env::var("RUST_LOG"), &cfg.level)?);

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

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_ansi(use_color(path.is_some()))
        .with_writer(writer);

    let registry = tracing_subscriber::registry()
        .with(filter)
        .with(sentry::integrations::tracing::layer());

    match cfg.format {
        LogFormat::Json => registry.with(fmt_layer.json()).init(),
        LogFormat::Text => registry.with(fmt_layer).init(),
    }

    Ok((guard, handle))
}

/// Apply `log.level` each time the configuration reloads, so the log can
/// be turned up without restarting (and losing what the server learned).
/// `RUST_LOG` fixes the filter for the life of the process.
fn follow_log_level(watcher: Arc<ConfigWatcher>, filter: FilterHandle) {
    if std::env::var_os("RUST_LOG").is_some() {
        return;
    }
    let mut reloads = watcher.subscribe();
    let mut applied = watcher.load().log.level.clone();
    tokio::spawn(async move {
        while reloads.changed().await.is_ok() {
            let level = watcher.load().log.level.clone();
            if level == applied {
                continue;
            }
            match log_filter(Err(VarError::NotPresent), &level)
                .and_then(|new| filter.reload(new).context("replacing the log filter"))
            {
                Ok(()) => {
                    tracing::info!(%level, "applied the new log.level");
                    applied = level;
                }
                Err(e) => tracing::warn!(error = %format!("{e:#}"), "kept the current log.level"),
            }
        }
    });
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

    // _sentry_guard must be held for the entire process lifetime so pending
    // events are flushed when it is dropped. Initialization happens before
    // the tracing subscriber so the sentry-tracing layer sees a bound client.
    let _sentry_guard = telemetry::init(&snapshot.telemetry);
    let sentry_enabled = _sentry_guard.is_some();

    // _log_guard must be held until server exits to flush buffered writes.
    let (_log_guard, log_filter) = init_logging(&snapshot.log, log_file)?;
    if let Some(watcher) = &config_watcher {
        follow_log_level(Arc::clone(watcher), log_filter);
    }

    if sentry_enabled {
        tracing::info!("sentry enabled");
    }

    // CLI overrides for listen address.
    let effective_host = host.as_deref().unwrap_or(&snapshot.host).to_owned();
    let effective_port = port.unwrap_or(snapshot.port);
    let addr = format!("{effective_host}:{effective_port}");

    let store = Arc::new(crate::open_store(db).await?);
    // One client for every upstream: token refreshes, catalog and version
    // fetches, and the proxied requests themselves.
    let http = byokey_proxy::upstream_client(snapshot.proxy_url.as_deref())?;
    let auth = Arc::new(AuthManager::new(store.clone(), http.clone()));

    // Background token refresh: check every 60s, refresh tokens within 5 min of expiry.
    let _refresh_handle = auth.spawn_refresh_loop(
        std::time::Duration::from_secs(60),
        std::time::Duration::from_secs(300),
    );

    let usage_store: Arc<dyn byokey_types::UsageStore> = store;
    let state = AppState::new(
        Arc::clone(&config_arc),
        auth,
        http,
        Some(usage_store.clone()),
    );
    let _identity_handle = state.spawn_copilot_identity_fetch();

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
    let app = byokey_proxy::make_router(Arc::clone(&state));

    // Acquire the HTTP listener. Prefer a pre-opened fd from systemfd /
    // systemd / launchd socket activation (no rebind on restart, no
    // EADDRINUSE in dev loops). Fall back to a fresh sync bind so that
    // EADDRINUSE surfaces immediately — `tokio::net::TcpListener::bind`
    // routes through async DNS and can hang in this process's runtime.
    let listener = match listenfd::ListenFd::from_env().take_tcp_listener(0) {
        Ok(Some(l)) => {
            tracing::info!("using inherited TCP listener from environment");
            l.set_nonblocking(true)
                .map_err(|e| anyhow::anyhow!("set_nonblocking: {e}"))?;
            tokio::net::TcpListener::from_std(l).map_err(|e| anyhow::anyhow!("from_std: {e}"))?
        }
        _ => {
            let parsed: std::net::SocketAddr = addr
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid address {addr}: {e}"))?;
            let std_listener = std::net::TcpListener::bind(parsed)
                .map_err(|e| anyhow::anyhow!("bind {addr}: {e}"))?;
            std_listener
                .set_nonblocking(true)
                .map_err(|e| anyhow::anyhow!("set_nonblocking: {e}"))?;
            tokio::net::TcpListener::from_std(std_listener)
                .map_err(|e| anyhow::anyhow!("from_std: {e}"))?
        }
    };

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
    drop(snapshot);
    tracing::info!(addr = %addr, "byokey listening");

    let shutdown_for_serve = Arc::clone(&shutdown);
    let serve_result = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_for_serve.notified().await;
        })
        .await
        .map_err(anyhow::Error::from);

    ctl_handle.cleanup();

    // Aux tokio tasks (config watcher, thread index watcher, control listener)
    // keep the runtime alive after axum::serve returns. Since the HTTP side has
    // already drained via graceful_shutdown and the socket is cleaned up, exit
    // the process explicitly to avoid a hang on shutdown.
    if serve_result.is_ok() {
        std::process::exit(0);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_invalid_log_filter_is_an_error_naming_where_it_came_from() {
        assert!(log_filter(Err(VarError::NotPresent), "info,tarpc=warn").is_ok());
        assert!(
            log_filter(Ok("debug".into()), "not a level").is_ok(),
            "RUST_LOG wins"
        );

        assert!(
            log_filter(Ok("byokey_proxy=debug".into()), "info").is_ok(),
            "RUST_LOG may name modules alone"
        );

        let err = log_filter(Err(VarError::NotPresent), "info,byokey_proxy=loud").unwrap_err();
        assert!(
            format!("{err:#}").starts_with("invalid log.level `info,byokey_proxy=loud`"),
            "{err:#}"
        );
        let err = log_filter(Ok("byokey=verbose".into()), "info").unwrap_err();
        assert!(
            format!("{err:#}").starts_with("invalid RUST_LOG `byokey=verbose`"),
            "{err:#}"
        );
        let err = log_filter(Err(VarError::NotPresent), "degub").unwrap_err();
        assert!(
            err.to_string().contains("sets no default level"),
            "a typo would turn logging off: {err}"
        );
    }
}
