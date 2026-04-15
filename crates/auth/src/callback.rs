//! Local HTTP callback server for OAuth redirect flows.
//!
//! Binds TCP listeners on both `[::1]:<port>` and `127.0.0.1:<port>` to cover
//! IPv6-first systems (macOS resolves `localhost` → `::1`), waits for the OAuth
//! provider to redirect the browser back, and extracts query parameters.

use byokey_types::{ByokError, Result};
use std::{collections::HashMap, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const TIMEOUT_SECS: u64 = 120;
const SUCCESS_HTML: &[u8] =
    b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n\
    <html><body><h1>Login successful!</h1><p>You may close this tab.</p></body></html>";

/// Listeners bound on both IPv4 and IPv6 loopback (whichever are available).
pub struct CallbackListeners {
    v4: Option<TcpListener>,
    v6: Option<TcpListener>,
}

/// Bind the callback port on both `[::1]` and `127.0.0.1`.
///
/// At least one must succeed; returns an error only if both fail.
///
/// # Errors
///
/// Returns an error if neither address can be bound.
pub async fn bind_callback(port: u16) -> Result<CallbackListeners> {
    let v6 = TcpListener::bind(format!("[::1]:{port}")).await.ok();
    let v4 = TcpListener::bind(format!("127.0.0.1:{port}")).await.ok();

    if v4.is_none() && v6.is_none() {
        return Err(ByokError::Auth(format!(
            "port {port} is already in use (another OAuth service may be running, e.g. vibeproxy/cli-proxy)\n\
             run `lsof -i :{port}` to find the process and stop it before retrying"
        )));
    }
    Ok(CallbackListeners { v4, v6 })
}

/// Wait for a single OAuth callback on the bound listeners.
///
/// Accepts from whichever listener receives a connection first (IPv4 or IPv6).
/// Times out after 120 seconds.
///
/// # Errors
///
/// Returns an error on accept/read failure or if the timeout expires.
pub async fn accept_callback(listeners: CallbackListeners) -> Result<HashMap<String, String>> {
    let accept = async {
        let stream = match (listeners.v4, listeners.v6) {
            (Some(v4), Some(v6)) => {
                tokio::select! {
                    r = v4.accept() => r.map(|(s, _)| s),
                    r = v6.accept() => r.map(|(s, _)| s),
                }
            }
            (Some(v4), None) => v4.accept().await.map(|(s, _)| s),
            (None, Some(v6)) => v6.accept().await.map(|(s, _)| s),
            (None, None) => unreachable!("bind_callback guarantees at least one"),
        }
        .map_err(|e| ByokError::Auth(e.to_string()))?;

        handle_callback_stream(stream).await
    };

    tokio::time::timeout(Duration::from_secs(TIMEOUT_SECS), accept)
        .await
        .map_err(|_| ByokError::Auth("timed out waiting for OAuth callback".into()))?
}

async fn handle_callback_stream(
    mut stream: TcpStream,
) -> std::result::Result<HashMap<String, String>, ByokError> {
    let mut buf = vec![0u8; 8192];
    let n = stream
        .read(&mut buf)
        .await
        .map_err(|e| ByokError::Auth(e.to_string()))?;

    let request = String::from_utf8_lossy(&buf[..n]);
    let params = parse_query_from_request(&request)?;

    stream
        .write_all(SUCCESS_HTML)
        .await
        .map_err(|e| ByokError::Auth(format!("write error: {e}")))?;
    let _ = stream.shutdown().await;

    Ok(params)
}

/// Bind a local port, wait for a single OAuth callback, and return its query parameters.
///
/// Convenience wrapper around [`bind_callback`] + [`accept_callback`].
///
/// # Errors
///
/// Returns an error if binding or accepting fails, or if the timeout expires.
pub async fn wait_for_callback(port: u16) -> Result<HashMap<String, String>> {
    let listeners = bind_callback(port).await?;
    accept_callback(listeners).await
}

fn parse_query_from_request(request: &str) -> Result<HashMap<String, String>> {
    // First line format: "GET /?code=...&state=... HTTP/1.1"
    let first_line = request.lines().next().unwrap_or("");
    let path = first_line.split_ascii_whitespace().nth(1).unwrap_or("/");
    let query = path.split_once('?').map_or("", |(_, q)| q);
    serde_urlencoded::from_str(query)
        .map_err(|e| ByokError::Auth(format!("invalid callback query params: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_query_standard() {
        let req = "GET /?code=abc123&state=xyz HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let params = parse_query_from_request(req).unwrap();
        assert_eq!(params.get("code").map(String::as_str), Some("abc123"));
        assert_eq!(params.get("state").map(String::as_str), Some("xyz"));
    }

    #[test]
    fn test_parse_query_no_query_string() {
        let req = "GET / HTTP/1.1\r\n\r\n";
        let params = parse_query_from_request(req).unwrap();
        assert!(params.is_empty());
    }

    #[test]
    fn test_parse_query_encoded() {
        let req = "GET /?code=a%2Bb&state=st HTTP/1.1\r\n\r\n";
        let params = parse_query_from_request(req).unwrap();
        assert_eq!(params.get("code").map(String::as_str), Some("a+b"));
    }
}
