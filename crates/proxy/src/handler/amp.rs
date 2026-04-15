//! Amp CLI compatibility layer.
//!
//! Routes:
//! - `GET  /amp/v1/login`              -> 302 redirect to ampcode.com/login.
//! - `ANY  /amp/v0/management/{*path}` -> proxy to ampcode.com/v0/management/*.
//! - `POST /amp/v1/chat/completions`   -> handled by `chat::chat_completions`.
//!
//! Submodules:
//! - [`provider`] — `AmpCode` provider-namespaced AI endpoints.
//! - [`quota`]    — cached `AmpCode` quota snapshot endpoint.
//! - [`threads`]  — local Amp CLI thread listing / detail endpoints.

pub mod provider;
pub mod quota;
pub mod threads;

use axum::{
    Router,
    extract::{Path, RawQuery, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{any, get, post},
};
use bytes::Bytes;
use serde_json::json;
use std::sync::Arc;

use byokey_types::ProviderId;

use crate::AppState;

use super::{CLIENT_AUTH_HEADERS, FINGERPRINT_HEADERS, HOP_BY_HOP, chat, messages};

/// Build the Amp CLI / `AmpCode` sub-router.
///
/// Spans two URL prefixes:
/// - `/amp/*`  — Amp CLI OAuth and chat endpoints.
/// - `/api/*`  — `AmpCode` provider-namespaced endpoints and catch-all proxy.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        // Amp CLI routes
        .route("/amp/auth/cli-login", get(cli_login_redirect))
        .route("/amp/v1/login", get(login_redirect))
        .route("/amp/v0/management/{*path}", any(management_proxy))
        .route("/amp/v1/chat/completions", post(chat::chat_completions))
        // AmpCode provider-specific routes (must be registered before the catch-all)
        .route(
            "/api/provider/anthropic/v1/messages",
            post(messages::anthropic_messages),
        )
        .route(
            "/api/provider/openai/v1/chat/completions",
            post(chat::chat_completions),
        )
        .route(
            "/api/provider/openai/v1/responses",
            post(provider::codex_responses_passthrough),
        )
        .route(
            "/api/provider/google/v1beta/models/{action}",
            post(provider::gemini_native_passthrough),
        )
        // Catch-all: forward remaining /api/* routes to ampcode.com
        .route("/api/{*path}", any(provider::amp_management_proxy))
}

/// Amp backend base URL.
const AMP_BACKEND: &str = "https://ampcode.com";

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

    // Resolve AMP auth: stored BYOKEY token > upstream_key > client passthrough.
    let amp_token = state.auth.get_token(&ProviderId::Amp).await.ok();
    let strip_client_auth = amp_token.is_some() || config.amp.upstream_key.is_some();

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
        if FINGERPRINT_HEADERS.contains(&name_str)
            || name_str.starts_with("sec-ch-ua-")
            || name_str.starts_with("sec-fetch-")
        {
            continue;
        }
        if let (Ok(n), Ok(v)) = (
            rquest::header::HeaderName::from_bytes(name.as_ref()),
            rquest::header::HeaderValue::from_bytes(value.as_bytes()),
        ) {
            header_map.insert(n, v);
        }
    }

    // Inject auth: stored token takes priority over upstream_key.
    if let Some(token) = &amp_token {
        inject_amp_auth(&mut header_map, &token.access_token);
    } else if let Some(key) = &config.amp.upstream_key {
        inject_amp_auth(&mut header_map, key);
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

/// Set `Authorization` and `X-Api-Key` headers on an outgoing request.
fn inject_amp_auth(headers: &mut rquest::header::HeaderMap, token: &str) {
    if let (Ok(n_auth), Ok(v_auth), Ok(n_apikey), Ok(v_apikey)) = (
        rquest::header::HeaderName::from_bytes(b"authorization"),
        rquest::header::HeaderValue::from_str(&format!("Bearer {token}")),
        rquest::header::HeaderName::from_bytes(b"x-api-key"),
        rquest::header::HeaderValue::from_str(token),
    ) {
        headers.insert(n_auth, v_auth);
        headers.insert(n_apikey, v_apikey);
    }
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
