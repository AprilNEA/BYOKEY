//! Shared HTTP utilities for provider executors.
//!
//! Eliminates duplicated send → status-check → stream-or-complete logic
//! across all executor implementations.

use byokey_types::{
    ByokError, ProviderId, RateLimitSnapshot, RateLimitStore,
    traits::{ByteStream, ProviderResponse, Result},
};
use futures_util::StreamExt as _;
use rquest::{Client, RequestBuilder};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Optional rate-limit capture context attached to a `ProviderHttp`.
#[derive(Clone)]
struct RateLimitCtx {
    store: Arc<RateLimitStore>,
    provider: ProviderId,
    account_id: String,
}

/// Shared HTTP helper that all executors can use to send requests and
/// handle the common response patterns (status check, stream vs complete).
#[derive(Clone)]
pub struct ProviderHttp {
    http: Client,
    rl_ctx: Option<RateLimitCtx>,
}

impl ProviderHttp {
    /// Creates a new helper wrapping the given HTTP client.
    #[must_use]
    pub fn new(http: Client) -> Self {
        Self { http, rl_ctx: None }
    }

    /// Attaches rate-limit capture context. Headers from every response
    /// sent through this helper will be stored in `store`.
    #[must_use]
    pub fn with_ratelimit(mut self, store: Arc<RateLimitStore>, provider: ProviderId) -> Self {
        self.rl_ctx = Some(RateLimitCtx {
            store,
            provider,
            account_id: "active".to_string(),
        });
        self
    }

    /// Returns a reference to the inner HTTP client for building requests.
    #[must_use]
    pub fn client(&self) -> &Client {
        &self.http
    }

    /// Extracts rate-limit-related headers from the response and writes
    /// them into the store (if a context is configured).
    fn capture_ratelimit_headers(&self, headers: &rquest::header::HeaderMap) {
        let Some(ctx) = &self.rl_ctx else { return };

        let mut captured = HashMap::new();
        for (name, value) in headers {
            let key = name.as_str();
            // Capture anthropic-ratelimit-*, x-ratelimit-*, retry-after
            if (key.starts_with("anthropic-ratelimit-")
                || key.starts_with("x-ratelimit-")
                || key == "retry-after")
                && let Ok(v) = value.to_str()
            {
                captured.insert(key.to_string(), v.to_string());
            }
        }

        if captured.is_empty() {
            return;
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        ctx.store.update(
            ctx.provider.clone(),
            ctx.account_id.clone(),
            RateLimitSnapshot {
                headers: captured,
                captured_at: now,
            },
        );
    }

    /// Sends a request and checks for success status.
    ///
    /// On non-2xx responses, reads the body text and returns
    /// [`ByokError::Upstream`]. Rate limit headers are captured from
    /// **both** success and error responses.
    ///
    /// # Errors
    ///
    /// Returns `ByokError::Upstream` on non-success HTTP status codes,
    /// or a transport error if the request fails to send.
    pub async fn send(&self, builder: RequestBuilder) -> Result<rquest::Response> {
        let resp = builder.send().await?;
        // Capture rate limit headers before consuming the body.
        self.capture_ratelimit_headers(resp.headers());
        let status = resp.status();
        if status.is_success() {
            Ok(resp)
        } else {
            let retry_after = parse_retry_after_header(resp.headers());
            let text = resp.text().await.unwrap_or_default();
            let retry_after = parse_retry_after_body(&text, status.as_u16()).or(retry_after);
            Err(ByokError::Upstream {
                status: status.as_u16(),
                body: text,
                retry_after,
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

/// Parse `Retry-After` header value (seconds integer).
fn parse_retry_after_header(headers: &rquest::header::HeaderMap) -> Option<std::time::Duration> {
    let val = headers.get("retry-after")?.to_str().ok()?;
    let secs: u64 = val.parse().ok()?;
    Some(std::time::Duration::from_secs(secs))
}

/// Parse retry delay from a 429 response body (Codex `usage_limit_reached` format).
///
/// Checks `error.type == "usage_limit_reached"`, then extracts either
/// `error.resets_in_seconds` or `error.resets_at` (unix timestamp).
fn parse_retry_after_body(body: &str, status: u16) -> Option<std::time::Duration> {
    if status != 429 {
        return None;
    }
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    let error = json.get("error")?;
    if error.get("type").and_then(serde_json::Value::as_str) != Some("usage_limit_reached") {
        return None;
    }
    // Try resets_in_seconds first (direct duration).
    if let Some(secs) = error
        .get("resets_in_seconds")
        .and_then(serde_json::Value::as_u64)
    {
        return Some(std::time::Duration::from_secs(secs));
    }
    // Fallback: resets_at as unix timestamp.
    if let Some(ts) = error.get("resets_at").and_then(serde_json::Value::as_u64) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if ts > now {
            return Some(std::time::Duration::from_secs(ts - now));
        }
    }
    None
}

/// Returns the appropriate `Accept` header value for a request.
///
/// Streaming requests need `text/event-stream`; non-streaming need `application/json`.
#[must_use]
pub fn accept_for_stream(stream: bool) -> &'static str {
    if stream {
        "text/event-stream"
    } else {
        "application/json"
    }
}

/// Injects `stream_options: { include_usage: true }` into the body when streaming.
///
/// Used by OpenAI-passthrough providers (Kimi, Qwen, iFlow, Copilot, Gemini).
pub fn ensure_stream_options(body: &mut serde_json::Value, stream: bool) {
    if stream {
        body["stream_options"] = serde_json::json!({ "include_usage": true });
    }
}

/// Resolves a bearer token: returns the API key if present, otherwise fetches
/// an OAuth token from the [`AuthManager`](byokey_auth::AuthManager).
///
/// This is the common pattern used by most providers (Kimi, Qwen, iFlow,
/// Antigravity, Kiro).
///
/// # Errors
///
/// Returns an error if the OAuth token fetch fails.
pub async fn resolve_bearer_token(
    api_key: Option<&str>,
    auth: &Arc<byokey_auth::AuthManager>,
    provider: &ProviderId,
) -> byokey_types::traits::Result<String> {
    if let Some(key) = api_key {
        return Ok(key.to_string());
    }
    let token = auth.get_token(provider).await?;
    Ok(token.access_token)
}

/// Creates a test `AuthManager` and HTTP client pair for executor unit tests.
///
/// Returns `(rquest::Client, Arc<AuthManager>)` backed by an in-memory token store.
#[cfg(test)]
#[must_use]
pub fn test_auth() -> (Client, Arc<byokey_auth::AuthManager>) {
    let store = Arc::new(byokey_store::InMemoryTokenStore::new());
    let auth = Arc::new(byokey_auth::AuthManager::new(store, Client::new()));
    (Client::new(), auth)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_http_clone() {
        let http = ProviderHttp::new(Client::new());
        let _http2 = http.clone();
    }

    #[test]
    fn test_with_ratelimit() {
        let store = Arc::new(RateLimitStore::new());
        let http = ProviderHttp::new(Client::new()).with_ratelimit(store, ProviderId::Claude);
        assert!(http.rl_ctx.is_some());
    }
}
