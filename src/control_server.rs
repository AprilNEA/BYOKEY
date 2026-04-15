//! Control socket server: accepts status/shutdown/reload requests from the CLI.
//!
//! Bound to `~/.byokey/control.sock` at server startup. Acts as the authoritative
//! liveness signal — if you can connect, the server is up. Shutdown is triggered
//! via a shared `Notify`; the HTTP server awaits that same `Notify` for graceful
//! shutdown.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use byokey_config::ConfigWatcher;
use byokey_daemon::control::{Request, Response, StatusData};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Notify;

pub struct ControlState {
    pub watcher: Option<Arc<ConfigWatcher>>,
    pub shutdown: Arc<Notify>,
    pub start: Instant,
    pub host: String,
    pub port: u16,
}

pub struct ControlHandle {
    pub sock_path: PathBuf,
}

impl ControlHandle {
    pub fn cleanup(&self) {
        let _ = std::fs::remove_file(&self.sock_path);
    }
}

pub fn bind_and_serve(
    sock_path: PathBuf,
    state: Arc<ControlState>,
) -> std::io::Result<ControlHandle> {
    if let Some(parent) = sock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // A lingering socket file from an unclean exit blocks bind — remove it.
    // `control::is_alive()` has already proven no live server is answering.
    let _ = std::fs::remove_file(&sock_path);

    let listener = UnixListener::bind(&sock_path)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let perms = std::fs::Permissions::from_mode(0o600);
        let _ = std::fs::set_permissions(&sock_path, perms);
    }

    let handle = ControlHandle {
        sock_path: sock_path.clone(),
    };

    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let state = Arc::clone(&state);
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, state).await {
                            tracing::debug!(error = %e, "control connection error");
                        }
                    });
                }
                Err(e) => {
                    tracing::warn!(error = %e, "control accept failed, exiting listener");
                    break;
                }
            }
        }
    });

    Ok(handle)
}

async fn handle_connection(stream: UnixStream, state: Arc<ControlState>) -> std::io::Result<()> {
    let (r, mut w) = stream.into_split();
    let mut reader = BufReader::new(r);
    let mut line = String::new();
    reader.read_line(&mut line).await?;

    let resp = match serde_json::from_str::<Request>(line.trim()) {
        Ok(req) => dispatch(req, &state),
        Err(e) => Response {
            ok: false,
            data: None,
            error: Some(format!("parse error: {e}")),
        },
    };

    let mut buf = serde_json::to_vec(&resp).map_err(std::io::Error::other)?;
    buf.push(b'\n');
    w.write_all(&buf).await?;
    w.flush().await?;
    Ok(())
}

fn dispatch(req: Request, state: &ControlState) -> Response {
    match req {
        Request::Status => Response {
            ok: true,
            data: Some(StatusData {
                pid: std::process::id(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                host: state.host.clone(),
                port: state.port,
                uptime_secs: state.start.elapsed().as_secs(),
            }),
            error: None,
        },
        Request::Shutdown => {
            tracing::info!("control: shutdown requested");
            state.shutdown.notify_waiters();
            Response {
                ok: true,
                data: None,
                error: None,
            }
        }
        Request::Reload => match &state.watcher {
            Some(w) => match w.reload() {
                Ok(()) => {
                    tracing::info!("control: config reloaded");
                    Response {
                        ok: true,
                        data: None,
                        error: None,
                    }
                }
                Err(e) => Response {
                    ok: false,
                    data: None,
                    error: Some(e.to_string()),
                },
            },
            None => Response {
                ok: false,
                data: None,
                error: Some("no config file (server started without --config)".to_owned()),
            },
        },
    }
}

/// Best-effort: test whether a control socket at `path` is answering.
#[allow(dead_code)]
pub fn socket_reachable(_path: &Path) -> bool {
    byokey_daemon::control::is_alive()
}
