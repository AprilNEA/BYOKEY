//! Header-forwarding middleware for amp proxy routes.
//!
//! Resolves amp auth, filters hop-by-hop / fingerprint headers, injects
//! `Authorization` + `X-Api-Key`, and stores the result as [`ForwardedHeaders`]
//! in request extensions so handlers can use it directly.

use axum::{extract::Request, middleware::Next, response::Response};
use byokey_types::ProviderId;
use std::sync::Arc;

use crate::AppState;

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

const CLIENT_AUTH_HEADERS: &[&str] = &["authorization", "x-api-key", "x-goog-api-key"];

const FINGERPRINT_HEADERS: &[&str] = &[
    "x-forwarded-for",
    "x-forwarded-host",
    "x-forwarded-proto",
    "x-real-ip",
    "forwarded",
    "via",
    "priority",
];

/// Prepared upstream headers with amp auth already injected.
/// Stored in request extensions by [`forward_headers_middleware`].
#[derive(Clone)]
pub struct ForwardedHeaders {
    pub headers: rquest::header::HeaderMap,
}

pub async fn forward_headers_middleware(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    mut request: Request,
    next: Next,
) -> Response {
    let config = state.config.load();
    let amp_token = state.auth.get_token(&ProviderId::Amp).await.ok();
    let strip_auth = amp_token.is_some() || config.amp.upstream_key.is_some();

    let mut out = rquest::header::HeaderMap::new();
    for (name, value) in request.headers() {
        let name_str = name.as_str();
        if HOP_BY_HOP.contains(&name_str) || name_str == "host" {
            continue;
        }
        if strip_auth && CLIENT_AUTH_HEADERS.contains(&name_str) {
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
            out.insert(n, v);
        }
    }

    if let Some(token) = &amp_token {
        inject_amp_auth(&mut out, &token.access_token);
    } else if let Some(key) = &config.amp.upstream_key {
        inject_amp_auth(&mut out, key);
    }

    request
        .extensions_mut()
        .insert(ForwardedHeaders { headers: out });
    next.run(request).await
}

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
