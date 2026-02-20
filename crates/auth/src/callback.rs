//! Local HTTP callback server for OAuth redirect flows.
//!
//! Binds a TCP listener on `127.0.0.1:<port>`, waits for the OAuth provider
//! to redirect the browser back, and extracts the query parameters (e.g.
//! `code` and `state`) from the request.

use byok_types::{ByokError, traits::Result};
use std::{collections::HashMap, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const TIMEOUT_SECS: u64 = 120;
const SUCCESS_HTML: &[u8] =
    b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n\
    <html><body><h1>Login successful!</h1><p>You may close this tab.</p></body></html>";

/// Bind the local callback port and return the listener.
///
/// The caller should bind the port **before** opening the browser to avoid a
/// race condition, then call [`accept_callback`] on the returned listener.
///
/// # Errors
///
/// Returns an error if the port is already in use or cannot be bound.
pub async fn bind_callback(port: u16) -> Result<TcpListener> {
    let addr = format!("127.0.0.1:{port}");
    TcpListener::bind(&addr).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::AddrInUse {
            ByokError::Auth(format!(
                "端口 {port} 已被占用（可能有其他 OAuth 服务正在运行，如 vibeproxy/cli-proxy）\n\
                 请运行 `lsof -i :{port}` 找到占用进程并关闭后重试"
            ))
        } else {
            ByokError::Auth(format!("无法绑定 callback 端口 {port}: {e}"))
        }
    })
}

/// Wait for a single OAuth callback on an already-bound listener.
///
/// Parses the query parameters from the incoming HTTP request and returns
/// them as a map. Times out after 120 seconds.
///
/// # Errors
///
/// Returns an error on accept/read failure or if the timeout expires.
pub async fn accept_callback(listener: TcpListener) -> Result<HashMap<String, String>> {
    let accept = async {
        let (mut stream, _) = listener
            .accept()
            .await
            .map_err(|e| ByokError::Auth(e.to_string()))?;

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

        Ok::<HashMap<String, String>, ByokError>(params)
    };

    tokio::time::timeout(Duration::from_secs(TIMEOUT_SECS), accept)
        .await
        .map_err(|_| ByokError::Auth("timed out waiting for OAuth callback".into()))?
}

/// Bind a local port, wait for a single OAuth callback, and return its query parameters.
///
/// Convenience wrapper around [`bind_callback`] + [`accept_callback`].
/// Primarily useful in tests or scenarios where early binding is not needed.
///
/// # Errors
///
/// Returns an error if binding or accepting fails, or if the timeout expires.
pub async fn wait_for_callback(port: u16) -> Result<HashMap<String, String>> {
    let listener = bind_callback(port).await?;
    accept_callback(listener).await
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
