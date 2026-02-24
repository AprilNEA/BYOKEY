//! Shared HTTP utilities for provider executors.
//!
//! Eliminates duplicated send → status-check → stream-or-complete logic
//! across all executor implementations.

use byokey_types::{
    ByokError,
    traits::{ByteStream, ProviderResponse, Result},
};
use futures_util::StreamExt as _;
use rquest::{Client, RequestBuilder};
use serde_json::Value;

/// Shared HTTP helper that all executors can use to send requests and
/// handle the common response patterns (status check, stream vs complete).
#[derive(Clone)]
pub struct ProviderHttp {
    http: Client,
}

impl ProviderHttp {
    /// Creates a new helper wrapping the given HTTP client.
    #[must_use]
    pub fn new(http: Client) -> Self {
        Self { http }
    }

    /// Returns a reference to the inner HTTP client for building requests.
    #[must_use]
    pub fn client(&self) -> &Client {
        &self.http
    }

    /// Sends a request and checks for success status.
    ///
    /// On non-2xx responses, reads the body text and returns
    /// [`ByokError::Upstream`].
    ///
    /// # Errors
    ///
    /// Returns `ByokError::Upstream` on non-success HTTP status codes,
    /// or a transport error if the request fails to send.
    pub async fn send(&self, builder: RequestBuilder) -> Result<rquest::Response> {
        let resp = builder.send().await?;
        let status = resp.status();
        if status.is_success() {
            Ok(resp)
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(ByokError::Upstream {
                status: status.as_u16(),
                body: text,
            })
        }
    }

    /// Sends a request and returns a `ProviderResponse` for OpenAI-passthrough
    /// providers (those that don't need response translation).
    ///
    /// If `stream` is true, wraps the bytes stream; otherwise parses JSON.
    ///
    /// # Errors
    ///
    /// Returns `ByokError::Upstream` on non-success status, or a transport/parse error.
    pub async fn send_passthrough(
        &self,
        builder: RequestBuilder,
        stream: bool,
    ) -> Result<ProviderResponse> {
        let resp = self.send(builder).await?;
        if stream {
            Ok(ProviderResponse::Stream(Self::byte_stream(resp)))
        } else {
            let json: Value = resp.json().await?;
            Ok(ProviderResponse::Complete(json))
        }
    }

    /// Converts an `rquest::Response` into a `ByteStream`.
    #[must_use]
    pub fn byte_stream(resp: rquest::Response) -> ByteStream {
        Box::pin(resp.bytes_stream().map(|r| r.map_err(ByokError::from)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_http_clone() {
        let http = ProviderHttp::new(Client::new());
        let _http2 = http.clone();
    }
}
