//! Amp CLI compatibility layer.
//!
//! Routes:
//! - `GET  /amp/v1/login`              -> 302 redirect to ampcode.com/login.
//! - `ANY  /amp/v0/management/{*path}` -> proxy to ampcode.com/v0/management/*.
//! - `POST /amp/v1/chat/completions`   -> handled by `chat::chat_completions`.
use axum::{
    extract::{Path, State},
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

/// Transparently proxies requests to the ampcode.com management API.
pub async fn management_proxy(
    State(state): State<Arc<AppState>>,
    method: Method,
    Path(path): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let url = format!("{AMP_BACKEND}/v0/management/{path}");

    // Build upstream request, converting http::Method (same underlying type)
    let mut builder = state.http.request(method, url).body(body);

    // Forward headers, skipping hop-by-hop and Host
    let mut header_map = rquest::header::HeaderMap::new();
    for (name, value) in &headers {
        let name_str = name.as_str();
        if !HOP_BY_HOP.contains(&name_str)
            && name_str != "host"
            && let Ok(n) = rquest::header::HeaderName::from_bytes(name.as_ref())
            && let Ok(v) = rquest::header::HeaderValue::from_bytes(value.as_bytes())
        {
            header_map.insert(n, v);
        }
    }
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
