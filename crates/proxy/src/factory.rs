//! Factory.ai passthrough proxy routes.
//!
//! Exposes `/factory/{provider}/*` endpoints that forward requests as-is to
//! Factory's LLM proxy with auth injected. No model inspection, no format
//! translation — the client speaks the provider's native API format directly.
//!
//! Routes:
//! - `/factory/anthropic/*` → `https://api.factory.ai/api/llm/a/*`
//! - `/factory/openai/*`    → `https://api.factory.ai/api/llm/o/*`
//! - `/factory/google/*`    → `https://api.factory.ai/api/llm/g/*`

use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use byokey_types::{ByokError, ProviderId};
use futures_util::TryStreamExt as _;
use std::sync::Arc;

use crate::{AppState, error::ApiError};

/// Factory API base URL.
const FACTORY_BASE: &str = "https://api.factory.ai";

/// User-Agent header matching the Factory CLI.
const USER_AGENT: &str = "factory-cli/0.62.1";

/// Headers that should not be forwarded to the upstream.
const STRIP_HEADERS: &[&str] = &["host", "authorization"];

/// Resolve the Factory bearer token from config API key or OAuth.
async fn resolve_token(state: &AppState) -> Result<String, ApiError> {
    let config = state.config.load();
    if let Some(key) = config
        .providers
        .get(&ProviderId::Factory)
        .and_then(|pc| pc.api_key.clone())
    {
        return Ok(key);
    }
    let token = state
        .auth
        .get_token(&ProviderId::Factory)
        .await
        .map_err(ApiError::from)?;
    Ok(token.access_token)
}

/// Generic Factory passthrough handler.
async fn factory_proxy(
    state: Arc<AppState>,
    provider: &str,
    llm_path: &str,
    sub_path: &str,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let token = resolve_token(&state).await?;
    let upstream_url = format!("{FACTORY_BASE}/api/llm/{llm_path}/{sub_path}");

    let mut builder = state
        .http
        .request(method, &upstream_url)
        .header("authorization", format!("Bearer {token}"))
        .header("user-agent", USER_AGENT)
        .header("x-api-provider", provider)
        .header("x-session-id", uuid::Uuid::new_v4().to_string())
        .header(
            "x-assistant-message-id",
            uuid::Uuid::new_v4().to_string(),
        );

    // Forward original headers, stripping hop-by-hop and auth.
    for (name, value) in &headers {
        let n = name.as_str();
        if STRIP_HEADERS.iter().any(|&s| n.eq_ignore_ascii_case(s)) {
            continue;
        }
        builder = builder.header(name, value);
    }

    let resp = builder
        .body(body)
        .send()
        .await
        .map_err(|e| ApiError(ByokError::from(e)))?;

    let status = resp.status();
    let upstream_status = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);

    let is_stream = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("text/event-stream"));

    if is_stream {
        let byte_stream = resp
            .bytes_stream()
            .map_err(|e| std::io::Error::other(e.to_string()));
        Ok(Response::builder()
            .status(upstream_status)
            .header("content-type", "text/event-stream")
            .header("cache-control", "no-cache")
            .header("x-accel-buffering", "no")
            .body(Body::from_stream(byte_stream))
            .expect("valid response"))
    } else {
        let resp_bytes = resp
            .bytes()
            .await
            .map_err(|e| ApiError(ByokError::from(e)))?;
        let content_type = headers
            .get("accept")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/json");
        Ok((upstream_status, [(axum::http::header::CONTENT_TYPE, content_type)], resp_bytes)
            .into_response())
    }
}

/// `ANY /factory/anthropic/{*path}` — Anthropic native passthrough via Factory.
pub async fn factory_anthropic(
    State(state): State<Arc<AppState>>,
    method: Method,
    headers: HeaderMap,
    Path(path): Path<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    factory_proxy(state, "anthropic", "a", &path, method, headers, body).await
}

/// `ANY /factory/openai/{*path}` — `OpenAI` native passthrough via Factory.
pub async fn factory_openai(
    State(state): State<Arc<AppState>>,
    method: Method,
    headers: HeaderMap,
    Path(path): Path<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    factory_proxy(state, "openai", "o", &path, method, headers, body).await
}

/// `ANY /factory/google/{*path}` — Google/Gemini native passthrough via Factory.
pub async fn factory_google(
    State(state): State<Arc<AppState>>,
    method: Method,
    headers: HeaderMap,
    Path(path): Path<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    factory_proxy(state, "google", "g", &path, method, headers, body).await
}
