//! Control RPC between the CLI and the running `byokey serve` process.
//!
//! Uses tarpc over a Unix domain socket (`~/.byokey/control.sock`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{DaemonError, Result};
use crate::paths;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusData {
    pub pid: u32,
    pub version: String,
    pub host: String,
    pub port: u16,
    pub uptime_secs: u64,
}

#[tarpc::service]
pub trait Control {
    async fn status() -> StatusData;
    async fn shutdown();
    async fn reload() -> std::result::Result<(), String>;
}

/// Default socket path (`~/.byokey/control.sock`).
pub fn default_socket() -> Result<PathBuf> {
    paths::control_sock_path()
}

async fn connect(sock: &Path) -> Result<ControlClient> {
    let stream = tokio::net::UnixStream::connect(sock)
        .await
        .map_err(|e| DaemonError::Io {
            path: sock.to_path_buf(),
            source: e,
        })?;
    let transport = tarpc::serde_transport::new(
        tokio_util::codec::Framed::new(stream, tokio_util::codec::LengthDelimitedCodec::new()),
        tarpc::tokio_serde::formats::Json::default(),
    );
    let client = ControlClient::new(tarpc::client::Config::default(), transport).spawn();
    Ok(client)
}

fn make_context() -> tarpc::context::Context {
    let mut ctx = tarpc::context::current();
    ctx.deadline = std::time::Instant::now() + DEFAULT_TIMEOUT;
    ctx
}

/// True if the server is reachable over the control socket.
#[must_use]
pub fn is_alive() -> bool {
    let Ok(path) = default_socket() else {
        return false;
    };
    let Ok(rt) = tokio::runtime::Handle::try_current() else {
        return false;
    };
    std::thread::scope(|s| {
        s.spawn(|| {
            rt.block_on(async {
                let Ok(client) = connect(&path).await else {
                    return false;
                };
                client.status(make_context()).await.is_ok()
            })
        })
        .join()
        .unwrap_or(false)
    })
}

pub fn status() -> Result<StatusData> {
    let path = default_socket()?;
    let rt = tokio::runtime::Handle::try_current().map_err(|_| DaemonError::ControlFailed {
        msg: "no tokio runtime".to_owned(),
    })?;
    std::thread::scope(|s| {
        s.spawn(|| {
            rt.block_on(async {
                let client = connect(&path).await?;
                client
                    .status(make_context())
                    .await
                    .map_err(|e| DaemonError::ControlFailed { msg: e.to_string() })
            })
        })
        .join()
        .unwrap_or(Err(DaemonError::ControlFailed {
            msg: "thread panicked".to_owned(),
        }))
    })
}

pub fn shutdown() -> Result<()> {
    let path = default_socket()?;
    let rt = tokio::runtime::Handle::try_current().map_err(|_| DaemonError::ControlFailed {
        msg: "no tokio runtime".to_owned(),
    })?;
    std::thread::scope(|s| {
        s.spawn(|| {
            rt.block_on(async {
                let client = connect(&path).await?;
                client
                    .shutdown(make_context())
                    .await
                    .map_err(|e| DaemonError::ControlFailed { msg: e.to_string() })
            })
        })
        .join()
        .unwrap_or(Err(DaemonError::ControlFailed {
            msg: "thread panicked".to_owned(),
        }))
    })
}

pub fn reload() -> Result<()> {
    let path = default_socket()?;
    let rt = tokio::runtime::Handle::try_current().map_err(|_| DaemonError::ControlFailed {
        msg: "no tokio runtime".to_owned(),
    })?;
    std::thread::scope(|s| {
        s.spawn(|| {
            rt.block_on(async {
                let client = connect(&path).await?;
                client
                    .reload(make_context())
                    .await
                    .map_err(|e| DaemonError::ControlFailed { msg: e.to_string() })?
                    .map_err(|msg| DaemonError::ControlFailed { msg })
            })
        })
        .join()
        .unwrap_or(Err(DaemonError::ControlFailed {
            msg: "thread panicked".to_owned(),
        }))
    })
}
