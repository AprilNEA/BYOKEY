//! Amp CLI compatibility layer.
//!
//! Routes:
//! - `GET  /amp/v1/login`              -> 302 redirect to ampcode.com/login.
//! - `ANY  /amp/v0/management/{*path}` -> proxy to ampcode.com/v0/management/*.
//! - `POST /amp/v1/chat/completions`   -> handled by `chat::chat_completions`.
use axum::{
    extract::{Path, RawQuery, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use serde_json::json;
use std::sync::Arc;

use crate::AppState;

/// Amp backend base URL.
const AMP_BACKEND: &str = "https://ampcode.com";

/// Headers that must not be forwarded (hop-by-hop).
const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailers",
    "transfer-encoding",
    "upgrade",
];

/// 共享代理模式下需要从客户端请求中剥离的认证头。
const CLIENT_AUTH_HEADERS: &[&str] = &["authorization", "x-api-key", "x-goog-api-key"];

/// Redirects Amp CLI to the web login page.
pub async fn login_redirect() -> impl IntoResponse {
    (
        StatusCode::FOUND,
        [(
            axum::http::header::LOCATION,
            HeaderValue::from_static("https://ampcode.com/login"),
        )],
    )
}

/// Handles `GET /amp/auth/cli-login?authToken=...&callbackPort=...`
///
/// `amp login` opens this URL in the browser. We forward it to `AmpCode`'s
/// own login endpoint so `AmpCode` can authenticate the user and then
/// callback to `http://localhost:{callbackPort}/...` directly.
pub async fn cli_login_redirect(RawQuery(query): RawQuery) -> impl IntoResponse {
    let url = match query {
        Some(q) => format!("https://ampcode.com/auth/cli-login?{q}"),
        None => "https://ampcode.com/auth/cli-login".to_string(),
    };
    let location = HeaderValue::from_str(&url)
        .unwrap_or_else(|_| HeaderValue::from_static("https://ampcode.com/amp/auth/cli-login"));
    (
        StatusCode::FOUND,
        [(axum::http::header::LOCATION, location)],
    )
}

/// Transparently proxies requests to the ampcode.com management API.
pub async fn management_proxy(
    State(state): State<Arc<AppState>>,
    method: Method,
    Path(path): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let url = format!("{AMP_BACKEND}/v0/management/{path}");

    let config = state.config.load();
    let strip_client_auth = config.amp.upstream_key.is_some();

    // Forward headers, skipping hop-by-hop and Host
    let mut header_map = rquest::header::HeaderMap::new();
    for (name, value) in &headers {
        let name_str = name.as_str();
        if HOP_BY_HOP.contains(&name_str) || name_str == "host" {
            continue;
        }
        if strip_client_auth && CLIENT_AUTH_HEADERS.contains(&name_str) {
            continue;
        }
        if let (Ok(n), Ok(v)) = (
            rquest::header::HeaderName::from_bytes(name.as_ref()),
            rquest::header::HeaderValue::from_bytes(value.as_bytes()),
        ) {
            header_map.insert(n, v);
        }
    }

    if let Some(key) = &config.amp.upstream_key
        && let (Ok(n_auth), Ok(v_auth), Ok(n_apikey), Ok(v_apikey)) = (
            rquest::header::HeaderName::from_bytes(b"authorization"),
            rquest::header::HeaderValue::from_str(&format!("Bearer {key}")),
            rquest::header::HeaderName::from_bytes(b"x-api-key"),
            rquest::header::HeaderValue::from_str(key.as_str()),
        )
    {
        header_map.insert(n_auth, v_auth);
        header_map.insert(n_apikey, v_apikey);
    }

    // Build upstream request
    let mut builder = state.http.request(method, url).body(body);
    builder = builder.headers(header_map);

    let resp = match builder.send().await {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                axum::Json(json!({"error": {"message": e.to_string()}})),
            )
                .into_response();
        }
    };

    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    // Forward upstream response headers
    let mut resp_headers = axum::http::HeaderMap::new();
    for (name, value) in resp.headers() {
        if let (Ok(n), Ok(v)) = (
            axum::http::HeaderName::from_bytes(name.as_ref()),
            axum::http::HeaderValue::from_bytes(value.as_bytes()),
        ) {
            resp_headers.insert(n, v);
        }
    }

    let body_bytes = resp.bytes().await.unwrap_or_default();
    (status, resp_headers, body_bytes).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hop_by_hop_includes_connection() {
        assert!(HOP_BY_HOP.contains(&"connection"));
        assert!(HOP_BY_HOP.contains(&"transfer-encoding"));
    }
}
