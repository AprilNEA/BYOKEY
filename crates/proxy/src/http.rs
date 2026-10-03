//! HTTP clients for upstream requests.
//!
//! Copilot, Anthropic and Cursor share a client and connection pool. They speak
//! HTTP/2, so every in-flight request shares a connection, and a connection
//! that dies silently would stall all of them at once. The client therefore
//! probes its connections: TCP keepalive at the socket (the library
//! default), and HTTP/2 PING frames on the connection, also while it sits
//! idle in the pool, so a dead connection is noticed within about half a
//! minute and the requests on it fail instead of waiting on the peer.
//! Raw Codex forwarding uses a separate pool because reqwest configures
//! automatic response decompression per client, not per request.
//!
//! Requests carry no overall or read timeout here. A Messages response
//! legitimately streams for minutes, and a Cursor run stays open while the
//! caller runs its tools. Silence is handled where the protocol is known:
//! in the proxy's SSE layer.

use byokey_types::{ByokError, Result};
use std::time::Duration;

/// How long to wait for a TCP connection and TLS handshake.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// HTTP/2 PING every this often, on idle connections too.
const HTTP2_PING_INTERVAL: Duration = Duration::from_secs(15);
/// An HTTP/2 PING unanswered for this long closes the connection.
const HTTP2_PING_TIMEOUT: Duration = Duration::from_secs(10);

/// Build the client for upstream requests, through `proxy_url` when given
/// (`http://`, `https://`, `socks5://` or `socks5h://`, with optional
/// credentials).
///
/// # Errors
///
/// Returns [`ByokError::Config`] if `proxy_url` is not a proxy URL or the
/// client cannot be built.
pub fn upstream_client(proxy_url: Option<&str>) -> Result<reqwest::Client> {
    client_builder(proxy_url)?
        .build()
        .map_err(|e| ByokError::Config(format!("HTTP client: {e}")))
}

pub(crate) fn passthrough_client(proxy_url: Option<&str>) -> Result<reqwest::Client> {
    client_builder(proxy_url)?
        .no_gzip()
        .no_brotli()
        .no_zstd()
        .no_deflate()
        .retry(reqwest::retry::never())
        .build()
        .map_err(|e| ByokError::Config(format!("passthrough HTTP client: {e}")))
}

fn client_builder(proxy_url: Option<&str>) -> Result<reqwest::ClientBuilder> {
    let mut builder = reqwest::Client::builder()
        // Custom auth headers must not reach a redirect destination.
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(CONNECT_TIMEOUT)
        .http2_keep_alive_interval(HTTP2_PING_INTERVAL)
        .http2_keep_alive_timeout(HTTP2_PING_TIMEOUT)
        .http2_keep_alive_while_idle(true);
    if let Some(url) = proxy_url {
        let proxy = reqwest::Proxy::all(url)
            .map_err(|e| ByokError::Config(format!("invalid proxy_url {url}: {e}")))?;
        builder = builder.proxy(proxy);
    }
    Ok(builder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_urls_are_validated_up_front() {
        assert!(upstream_client(None).is_ok());
        assert!(upstream_client(Some("socks5h://127.0.0.1:1080")).is_ok());
        assert!(matches!(
            upstream_client(Some("not a url")),
            Err(ByokError::Config(_))
        ));
    }
}
