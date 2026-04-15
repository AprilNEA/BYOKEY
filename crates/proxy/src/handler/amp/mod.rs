//! Amp CLI compatibility layer — served on a dedicated port when `amp.port`
//! is configured.
//!
//! Routes (all paths are served **without** an `/amp` prefix):
//! - `GET  /v1/login`              -> 302 redirect to ampcode.com/login.
//! - `GET  /auth/cli-login`        -> 302 redirect to ampcode.com/auth/cli-login.
//! - `ANY  /v0/management/{*path}` -> proxy to ampcode.com/v0/management/*.
//! - `POST /api/provider/*`        -> provider-specific handlers.
//! - `ANY  /api/{*path}`           -> catch-all proxy to ampcode.com.
//!
//! Submodules:
//! - [`provider`] — `AmpCode` provider-namespaced AI endpoints (`/api/provider/*`).
//! - [`threads`]  — local Amp CLI thread listing / detail endpoints.

pub mod provider;
pub mod threads;

use axum::{
    Router,
    extract::RawQuery,
    http::{HeaderValue, StatusCode},
    response::IntoResponse,
    routing::{any, get, post},
};
use std::sync::Arc;

use crate::AppState;

use super::{chat, messages};

/// Build the Amp CLI / `AmpCode` router.
///
/// Designed to run on its own listener (via `amp.port`). All paths match
/// what the Amp CLI actually sends on the wire — `new URL(path, ampUrl)` in
/// JS drops the base path component, so no `/amp` prefix is needed.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/auth/cli-login", get(cli_login_redirect))
        .route("/v1/login", get(login_redirect))
        .route("/v0/management/{*path}", any(provider::ampcode_proxy))
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
        .route("/api/{*path}", any(provider::ampcode_proxy))
}

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

/// Handles `GET /auth/cli-login?authToken=...&callbackPort=...`
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
