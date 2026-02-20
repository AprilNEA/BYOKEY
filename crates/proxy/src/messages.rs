//! Anthropic Messages API passthrough handler.
//!
//! Accepts requests in native Anthropic format and forwards them directly to
//! `api.anthropic.com/v1/messages` without any format translation.  The
//! response (streaming SSE or complete JSON) is returned as-is.

use axum::{
    body::Body,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use byokey_types::{ByokError, ProviderId};
use futures_util::TryStreamExt as _;
use serde_json::Value;
use std::sync::Arc;

use crate::{AppState, error::ApiError};

const API_URL: &str = "https://api.anthropic.com/v1/messages?beta=true";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const ANTHROPIC_BETA: &str = "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14,fine-grained-tool-streaming-2025-05-14,prompt-caching-2024-07-31";
const USER_AGENT: &str = "claude-cli/2.1.44 (external, sdk-cli)";

/// Handles `POST /v1/messages` — Anthropic native format passthrough.
///
/// Authenticates with the Claude provider (API key or OAuth), then forwards
/// the request body verbatim to the Anthropic API and streams the response
/// back without translation.
pub async fn anthropic_messages(
    State(state): State<Arc<AppState>>,
    body: axum::extract::Json<Value>,
) -> Result<Response, ApiError> {
    let body = body.0;
    let stream = body.get("stream").and_then(Value::as_bool).unwrap_or(false);

    // Resolve Claude auth: config API key takes priority over OAuth token.
    let provider_cfg = state.config.providers.get(&ProviderId::Claude);
    let api_key = provider_cfg.and_then(|pc| pc.api_key.clone());

    let builder = state
        .http
        .post(API_URL)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("anthropic-beta", ANTHROPIC_BETA)
        .header("anthropic-dangerous-direct-browser-access", "true")
        .header("x-app", "cli")
        .header("user-agent", USER_AGENT)
        .header("content-type", "application/json");

    let builder = if let Some(key) = api_key {
        builder.header("x-api-key", key)
    } else {
        let token = state
            .auth
            .get_token(&ProviderId::Claude)
            .await
            .map_err(|e| ApiError::from(ByokError::Auth(e.to_string())))?;
        builder.header("authorization", format!("Bearer {}", token.access_token))
    };

    let resp = builder
        .json(&body)
        .send()
        .await
        .map_err(|e| ApiError::from(ByokError::Http(e.to_string())))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(ApiError::from(ByokError::Http(format!(
            "Claude API {status}: {text}"
        ))));
    }

    let upstream_status = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::OK);

    if stream {
        let byte_stream = resp
            .bytes_stream()
            .map_err(|e| std::io::Error::other(e.to_string()));
        let out_body = Body::from_stream(byte_stream);
        Ok(Response::builder()
            .status(upstream_status)
            .header("content-type", "text/event-stream")
            .header("cache-control", "no-cache")
            .header("x-accel-buffering", "no")
            .body(out_body)
            .expect("valid response"))
    } else {
        let json: Value = resp
            .json()
            .await
            .map_err(|e| ApiError::from(ByokError::Http(e.to_string())))?;
        Ok((upstream_status, axum::Json(json)).into_response())
    }
}
