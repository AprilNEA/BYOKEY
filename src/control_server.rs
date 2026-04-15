//! Control socket server: tarpc service over a Unix domain socket.
//!
//! Bound to `~/.byokey/control.sock` at server startup. Shutdown is triggered
//! via a shared `Notify`; the HTTP server awaits that same `Notify` for graceful
//! shutdown.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use byokey_config::ConfigWatcher;
use byokey_daemon::control::{Control, StatusData};
use futures_util::stream::StreamExt as _;
use tarpc::server::{BaseChannel, Channel as _};
use tokio::net::UnixListener;
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

#[derive(Clone)]
struct ControlServer(Arc<ControlState>);

impl Control for ControlServer {
    async fn status(self, _: tarpc::context::Context) -> StatusData {
        StatusData {
            pid: std::process::id(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            host: self.0.host.clone(),
            port: self.0.port,
            uptime_secs: self.0.start.elapsed().as_secs(),
        }
    }

    async fn shutdown(self, _: tarpc::context::Context) {
        tracing::info!("control: shutdown requested");
        self.0.shutdown.notify_waiters();
    }

    async fn reload(self, _: tarpc::context::Context) -> Result<(), String> {
        match &self.0.watcher {
            Some(w) => match w.reload() {
                Ok(()) => {
                    tracing::info!("control: config reloaded");
                    Ok(())
                }
                Err(e) => Err(e.to_string()),
            },
            None => Err("no config file (server started without --config)".to_owned()),
        }
    }
}

pub fn bind_and_serve(
    sock_path: PathBuf,
    state: Arc<ControlState>,
) -> std::io::Result<ControlHandle> {
    if let Some(parent) = sock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
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
    let server = ControlServer(state);

    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let transport = tarpc::serde_transport::new(
                        tokio_util::codec::Framed::new(
                            stream,
                            tokio_util::codec::LengthDelimitedCodec::new(),
                        ),
                        tarpc::tokio_serde::formats::Json::default(),
                    );
                    let s = server.clone();
                    tokio::spawn(
                        BaseChannel::with_defaults(transport)
                            .execute(s.serve())
                            .for_each(|resp| async {
                                tokio::spawn(resp);
                            }),
                    );
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
