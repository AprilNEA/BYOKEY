//! Amp CLI compatibility layer — served on the main byokey port alongside
//! the REST AI proxy and `ConnectRPC` management service.
//!
//! Routes (all paths are served **without** an `/amp` prefix, matching
//! what the amp CLI sends on the wire):
//! - `GET  /v1/login`              -> 302 redirect to ampcode.com/login.
//! - `GET  /auth/cli-login`        -> 302 redirect to ampcode.com/auth/cli-login.
//! - `ANY  /v0/management/{*path}` -> proxy to ampcode.com/v0/management/*.
//! - `POST /api/provider/*`        -> provider-specific handlers.
//! - `ANY  /api/{*path}`           -> catch-all proxy to ampcode.com.
//!
//! Submodules:
//! - [`provider`] — `AmpCode` provider-namespaced AI endpoints (`/api/provider/*`).
//! - [`threads`]  — parser + in-memory index for local Amp CLI thread files
//!   (consumed by the `ConnectRPC` management service).

pub mod provider;
pub mod threads;

use axum::{
    extract::RawQuery,
    http::{HeaderValue, StatusCode},
    response::IntoResponse,
};

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
