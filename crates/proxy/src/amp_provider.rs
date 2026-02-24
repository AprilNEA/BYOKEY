//! `AmpCode` provider-specific API route handlers.
//!
//! `AmpCode` routes AI requests to provider-namespaced endpoints instead of
//! the generic `/v1/chat/completions`:
//!
//! | `AmpCode` route | Handler |
//! |---|---|
//! | `POST /api/provider/anthropic/v1/messages` | [`messages::anthropic_messages`] (aliased) |
//! | `POST /api/provider/openai/v1/chat/completions` | [`chat::chat_completions`] (aliased) |
//! | `POST /api/provider/openai/v1/responses` | [`codex_responses_passthrough`] |
//! | `POST /api/provider/google/v1beta/models/{action}` | [`gemini_native_passthrough`] |
//!
//! Management routes (`/api/auth`, `/api/threads`, etc.) are forwarded to
//! `ampcode.com` verbatim via [`amp_management_proxy`].

use axum::{
    body::Body,
    extract::{Path, Query, RawQuery, State},
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
};
use byokey_types::{ByokError, ProviderId};
use bytes::Bytes;
use futures_util::TryStreamExt as _;
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

use crate::{AppState, error::ApiError};

/// Codex OAuth Responses endpoint (`ChatGPT` subscription).
const CODEX_RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
/// `OpenAI` public Responses API endpoint (API key).
const OPENAI_RESPONSES_URL: &str = "https://api.openai.com/v1/responses";
/// Codex CLI version header value.
const CODEX_VERSION: &str = "0.101.0";
/// Codex CLI `User-Agent` header value.
const CODEX_USER_AGENT: &str = "codex_cli_rs/0.101.0 (Mac OS 26.0.1; arm64) Apple_Terminal/464";

/// Google Generative Language API models base URL.
const GEMINI_MODELS_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";

/// `ampcode.com` backend base URL for management route proxying.
const AMP_BACKEND: &str = "https://ampcode.com";

/// Hop-by-hop headers that must not be forwarded.
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

/// Handles `POST /api/provider/openai/v1/responses`.
///
/// `AmpCode` sends requests already formatted as `OpenAI` Responses API objects
/// (used for Oracle and Deep modes: `GPT-5.2`, `GPT-5.3 Codex`).
///
/// Routing:
/// - **OAuth token** → `chatgpt.com/backend-api/codex/responses` (Codex CLI endpoint)
/// - **API key** → `api.openai.com/v1/responses` (public `OpenAI` Responses API)
pub async fn codex_responses_passthrough(
    State(state): State<Arc<AppState>>,
    axum::extract::Json(body): axum::extract::Json<Value>,
) -> Result<Response, ApiError> {
    let config = state.config.load();
    let api_key = config
        .providers
        .get(&ProviderId::Codex)
        .and_then(|pc| pc.api_key.clone());

    let (is_oauth, token) = if let Some(key) = api_key {
        (false, key)
    } else {
        let tok = state
            .auth
            .get_token(&ProviderId::Codex)
            .await
            .map_err(ApiError::from)?;
        (true, tok.access_token)
    };

    let resp = if is_oauth {
        state
            .http
            .post(CODEX_RESPONSES_URL)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .header("Version", CODEX_VERSION)
            .header("User-Agent", CODEX_USER_AGENT)
            .header("Originator", "codex_cli_rs")
            .header("Accept", "text/event-stream")
            .json(&body)
            .send()
            .await
    } else {
        state
            .http
            .post(OPENAI_RESPONSES_URL)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
    }
    .map_err(|e| ApiError(ByokError::from(e)))?;

    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(ApiError::from(ByokError::Upstream {
            status: status.as_u16(),
            body: text,
        }));
    }

    let is_sse = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("text/event-stream"));

    if is_sse {
        let stream = resp
            .bytes_stream()
            .map_err(|e| std::io::Error::other(e.to_string()));
        Ok(Response::builder()
            .status(status)
            .header("content-type", "text/event-stream")
            .header("cache-control", "no-cache")
            .header("x-accel-buffering", "no")
            .body(Body::from_stream(stream))
            .expect("valid response"))
    } else {
        let json: Value = resp
            .json()
            .await
            .map_err(|e| ApiError(ByokError::from(e)))?;
        Ok((status, axum::Json(json)).into_response())
    }
}

/// Handles `POST /api/provider/google/v1beta/models/{action}`.
///
/// `AmpCode` sends requests in Google's native `generateContent` /
/// `streamGenerateContent` format (used for Review, Search, Look At, Handoff,
/// Topics, and Painter modes).
///
/// The `{action}` path segment contains `{model}:{method}`, e.g.
/// `gemini-3-pro:generateContent` or `gemini-3-flash:streamGenerateContent`.
/// Query parameters (e.g. `?alt=sse`) are forwarded verbatim to the upstream.
///
/// When the Gemini provider has `backend` configured (e.g. `backend: copilot`),
/// the request is translated from Google native format to `OpenAI` format,
/// sent to the backend provider, and the response is translated back.
pub async fn gemini_native_passthrough(
    State(state): State<Arc<AppState>>,
    Path(action): Path<String>,
    Query(query_params): Query<HashMap<String, String>>,
    axum::extract::Json(body): axum::extract::Json<Value>,
) -> Result<Response, ApiError> {
    let config = state.config.load();
    let gemini_config = config
        .providers
        .get(&ProviderId::Gemini)
        .cloned()
        .unwrap_or_default();

    // Extract model name from action (e.g. "gemini-3-pro:generateContent" → "gemini-3-pro")
    let model_name = action
        .split_once(':')
        .map_or(action.as_str(), |(model, _)| model);

    // If a backend override is configured, translate and route through it.
    if let Some(backend_id) = &gemini_config.backend {
        return gemini_native_via_backend(
            &state,
            &action,
            &query_params,
            body,
            model_name,
            backend_id,
        )
        .await;
    }

    // Direct passthrough to Gemini API.
    let api_key = gemini_config.api_key;

    // Rebuild query string from parsed params.
    let qs: String = query_params
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    let url = if qs.is_empty() {
        format!("{GEMINI_MODELS_BASE}/{action}")
    } else {
        format!("{GEMINI_MODELS_BASE}/{action}?{qs}")
    };

    // API key → `x-goog-api-key`; OAuth token → `Authorization: Bearer`.
    let (auth_name, auth_value): (&'static str, String) = if let Some(key) = api_key {
        ("x-goog-api-key", key)
    } else {
        let token = state
            .auth
            .get_token(&ProviderId::Gemini)
            .await
            .map_err(ApiError::from)?;
        ("authorization", format!("Bearer {}", token.access_token))
    };

    let resp = state
        .http
        .post(&url)
        .header(auth_name, auth_value)
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| ApiError(ByokError::from(e)))?;

    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(ApiError::from(ByokError::Upstream {
            status: status.as_u16(),
            body: text,
        }));
    }

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_string();

    if content_type.contains("text/event-stream") {
        let stream = resp
            .bytes_stream()
            .map_err(|e| std::io::Error::other(e.to_string()));
        Ok(Response::builder()
            .status(status)
            .header("content-type", content_type)
            .header("cache-control", "no-cache")
            .header("x-accel-buffering", "no")
            .body(Body::from_stream(stream))
            .expect("valid response"))
    } else {
        let json: Value = resp
            .json()
            .await
            .map_err(|e| ApiError(ByokError::from(e)))?;
        Ok((status, axum::Json(json)).into_response())
    }
}

/// Route a Gemini native request through an `OpenAI`-compatible backend provider.
///
/// Translates: Google native → `OpenAI` → backend → `OpenAI` response → Google native.
async fn gemini_native_via_backend(
    state: &Arc<AppState>,
    action: &str,
    query_params: &HashMap<String, String>,
    body: Value,
    model: &str,
    backend_id: &ProviderId,
) -> Result<Response, ApiError> {
    let is_stream = action.contains("streamGenerateContent")
        || query_params.get("alt").is_some_and(|v| v == "sse");

    // Build the executor for the backend provider.
    let config = state.config.load();
    let backend_config = config
        .providers
        .get(backend_id)
        .cloned()
        .unwrap_or_default();
    let executor = byokey_provider::make_executor(
        backend_id,
        backend_config.api_key,
        state.auth.clone(),
        state.http.clone(),
    )
    .ok_or_else(|| {
        ApiError::from(ByokError::UnsupportedModel(format!(
            "backend {backend_id:?} has no executor"
        )))
    })?;

    // Translate Gemini native request → OpenAI format.
    let openai_req: Value = byokey_translate::GeminiNativeRequest { body: &body, model }
        .try_into()
        .map_err(ApiError::from)?;

    // Send through the backend executor.
    let provider_resp = executor
        .chat_completion(openai_req, is_stream)
        .await
        .map_err(ApiError::from)?;

    match provider_resp {
        byokey_types::traits::ProviderResponse::Complete(openai_resp) => {
            let gemini_resp: Value = byokey_translate::OpenAIResponseToGemini {
                body: &openai_resp,
                model,
            }
            .try_into()
            .map_err(ApiError::from)?;
            Ok(axum::Json(gemini_resp).into_response())
        }
        byokey_types::traits::ProviderResponse::Stream(byte_stream) => {
            // Translate each OpenAI SSE chunk → Gemini native SSE chunk.
            let model_owned = model.to_string();
            let translated = byte_stream_to_gemini_sse(byte_stream, model_owned);
            let mapped = translated.map_err(|e| std::io::Error::other(e.to_string()));
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "text/event-stream")
                .header("cache-control", "no-cache")
                .header("x-accel-buffering", "no")
                .body(Body::from_stream(mapped))
                .expect("valid response"))
        }
    }
}

/// Transform a stream of `OpenAI` SSE byte chunks into Gemini-native SSE chunks.
///
/// The upstream `ByteStream` yields arbitrary byte boundaries; SSE lines may be
/// split across chunks. We buffer incoming bytes and split on newlines so that
/// each line is translated individually.
fn byte_stream_to_gemini_sse(
    upstream: byokey_types::traits::ByteStream,
    model: String,
) -> impl futures_util::Stream<Item = std::result::Result<Bytes, ByokError>> {
    use futures_util::StreamExt as _;

    let mut buffer = Vec::<u8>::new();

    upstream.flat_map(move |chunk_result| {
        let mut output: Vec<std::result::Result<Bytes, ByokError>> = Vec::new();

        match chunk_result {
            Err(e) => output.push(Err(e)),
            Ok(chunk) => {
                buffer.extend_from_slice(&chunk);

                // Process complete lines from the buffer.
                while let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = buffer.drain(..=pos).collect();
                    let translated: Option<Vec<u8>> = byokey_translate::OpenAISseChunk {
                        line: &line,
                        model: &model,
                    }
                    .into();
                    if let Some(bytes) = translated {
                        output.push(Ok(Bytes::from(bytes)));
                    }
                }
            }
        }

        futures_util::stream::iter(output)
    })
}

/// 共享代理模式下需要从客户端请求中剥离的认证头。
const CLIENT_AUTH_HEADERS: &[&str] = &["authorization", "x-api-key", "x-goog-api-key"];

/// Handles `ANY /api/{*path}` — forwards non-provider `ampcode.com` management
/// routes (auth, threads, telemetry, etc.) transparently to the upstream.
pub async fn amp_management_proxy(
    State(state): State<Arc<AppState>>,
    method: Method,
    Path(path): Path<String>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let url = match query.as_deref() {
        Some(q) if !q.is_empty() => format!("{AMP_BACKEND}/api/{path}?{q}"),
        _ => format!("{AMP_BACKEND}/api/{path}"),
    };

    // Debug logging for /api/internal requests (controlled by tracing level).
    let debug = path == "internal" && tracing::enabled!(tracing::Level::DEBUG);
    if debug {
        let req_body = std::str::from_utf8(&body)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .map_or_else(
                || format!("{body:?}"),
                |v| serde_json::to_string_pretty(&v).unwrap_or_default(),
            );
        tracing::debug!(%method, %url, body = %req_body, "amp proxy request");
    }

    let config = state.config.load();
    let strip_client_auth = config.amp.upstream_key.is_some();

    let mut upstream_headers = rquest::header::HeaderMap::new();
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
            upstream_headers.insert(n, v);
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
        upstream_headers.insert(n_auth, v_auth);
        upstream_headers.insert(n_apikey, v_apikey);
    }

    let resp = match state
        .http
        .request(method, url)
        .headers(upstream_headers)
        .body(body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                axum::Json(serde_json::json!({"error": {"message": e.to_string()}})),
            )
                .into_response();
        }
    };

    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);

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

    if debug {
        let resp_body = std::str::from_utf8(&body_bytes)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .map_or_else(
                || format!("{body_bytes:?}"),
                |v| serde_json::to_string_pretty(&v).unwrap_or_default(),
            );
        tracing::debug!(%status, body = %resp_body, "amp proxy response");
    }

    (status, resp_headers, body_bytes).into_response()
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_hop_by_hop_list() {
        assert!(super::HOP_BY_HOP.contains(&"connection"));
        assert!(super::HOP_BY_HOP.contains(&"transfer-encoding"));
        assert!(!super::HOP_BY_HOP.contains(&"authorization"));
    }

    #[test]
    fn test_urls_are_https() {
        assert!(super::CODEX_RESPONSES_URL.starts_with("https://"));
        assert!(super::OPENAI_RESPONSES_URL.starts_with("https://"));
        assert!(super::GEMINI_MODELS_BASE.starts_with("https://"));
    }
}
