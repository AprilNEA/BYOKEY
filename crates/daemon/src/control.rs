//! Control socket protocol between the CLI and the running `byokey serve` process.
//!
//! A running server binds `~/.byokey/control.sock` and accepts newline-delimited JSON
//! requests. The CLI uses socket connectivity as the authoritative signal of liveness,
//! avoiding PID-file races and PID reuse.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{DaemonError, Result};
use crate::paths;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "lowercase")]
pub enum Request {
    Status,
    Shutdown,
    Reload,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<StatusData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusData {
    pub pid: u32,
    pub version: String,
    pub host: String,
    pub port: u16,
    pub uptime_secs: u64,
}

/// Connect to the control socket, send a request, and read the response.
pub fn request(sock: &Path, req: &Request) -> Result<Response> {
    let mut stream = UnixStream::connect(sock).map_err(|e| DaemonError::Io {
        path: sock.to_path_buf(),
        source: e,
    })?;
    stream.set_read_timeout(Some(DEFAULT_TIMEOUT)).ok();
    stream.set_write_timeout(Some(DEFAULT_TIMEOUT)).ok();

    let payload = serde_json::to_vec(req).map_err(|e| DaemonError::Io {
        path: sock.to_path_buf(),
        source: std::io::Error::other(e),
    })?;
    stream.write_all(&payload).map_err(|e| DaemonError::Io {
        path: sock.to_path_buf(),
        source: e,
    })?;
    stream.write_all(b"\n").map_err(|e| DaemonError::Io {
        path: sock.to_path_buf(),
        source: e,
    })?;
    stream.flush().ok();

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| DaemonError::Io {
        path: sock.to_path_buf(),
        source: e,
    })?;
    serde_json::from_str(line.trim()).map_err(|e| DaemonError::Io {
        path: sock.to_path_buf(),
        source: std::io::Error::other(e),
    })
}

/// Default socket path (`~/.byokey/control.sock`).
pub fn default_socket() -> Result<PathBuf> {
    paths::control_sock_path()
}

/// True if the server is reachable over the control socket.
#[must_use]
pub fn is_alive() -> bool {
    let Ok(path) = default_socket() else {
        return false;
    };
    matches!(
        request(&path, &Request::Status),
        Ok(Response { ok: true, .. })
    )
}

pub fn status() -> Result<StatusData> {
    let path = default_socket()?;
    let resp = request(&path, &Request::Status)?;
    match resp {
        Response {
            ok: true,
            data: Some(data),
            ..
        } => Ok(data),
        Response { error, .. } => Err(DaemonError::ControlFailed {
            msg: error.unwrap_or_else(|| "status request failed".to_owned()),
        }),
    }
}

pub fn shutdown() -> Result<()> {
    let path = default_socket()?;
    let resp = request(&path, &Request::Shutdown)?;
    if resp.ok {
        Ok(())
    } else {
        Err(DaemonError::ControlFailed {
            msg: resp
                .error
                .unwrap_or_else(|| "shutdown request failed".to_owned()),
        })
    }
}

pub fn reload() -> Result<()> {
    let path = default_socket()?;
    let resp = request(&path, &Request::Reload)?;
    if resp.ok {
        Ok(())
    } else {
        Err(DaemonError::ControlFailed {
            msg: resp
                .error
                .unwrap_or_else(|| "reload request failed".to_owned()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip_json() {
        let s = serde_json::to_string(&Request::Status).unwrap();
        assert_eq!(s, r#"{"cmd":"status"}"#);
        let s = serde_json::to_string(&Request::Shutdown).unwrap();
        assert_eq!(s, r#"{"cmd":"shutdown"}"#);
        let s = serde_json::to_string(&Request::Reload).unwrap();
        assert_eq!(s, r#"{"cmd":"reload"}"#);
    }

    #[test]
    fn response_roundtrip_json() {
        let ok = Response {
            ok: true,
            data: None,
            error: None,
        };
        assert_eq!(serde_json::to_string(&ok).unwrap(), r#"{"ok":true}"#);

        let err = Response {
            ok: false,
            data: None,
            error: Some("nope".to_owned()),
        };
        assert_eq!(
            serde_json::to_string(&err).unwrap(),
            r#"{"ok":false,"error":"nope"}"#
        );
    }
}
