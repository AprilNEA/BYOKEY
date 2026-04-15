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
//! `ampcode.com` verbatim via [`ampcode_proxy`].

use axum::{
    extract::{Path, Query, State},
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
};
use byokey_types::{ByokError, ProviderId};
use bytes::Bytes;
use futures_util::TryStreamExt as _;
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

use crate::middleware::forward::ForwardedHeaders;
use crate::util::stream::{CodexParser, GeminiParser, response_to_stream, tap_usage_stream};
use crate::util::{bad_gateway, extract_usage, sse_response, upstream_error};
use crate::{AppState, error::ApiError};

const CODEX_RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
const OPENAI_RESPONSES_URL: &str = "https://api.openai.com/v1/responses";
const CODEX_VERSION: &str = "0.120.0";
const CODEX_USER_AGENT: &str = "codex-tui/0.120.0 (Mac OS 26.0.1; arm64) Apple_Terminal/464";
const GEMINI_MODELS_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";
const AMP_BACKEND: &str = "https://ampcode.com";

/// Handles `POST /api/provider/openai/v1/responses`.
///
/// `AmpCode` sends requests already formatted as `OpenAI` Responses API objects
/// (used for Oracle and Deep modes: `GPT-5.2`, `GPT-5.3 Codex`).
///
/// Routing:
/// - **OAuth token** → `chatgpt.com/backend-api/codex/responses` (Codex CLI endpoint)
/// - **API key** → `api.openai.com/v1/responses` (public `OpenAI` Responses API)
///
/// # Errors
///
/// Returns [`ApiError`] if Codex auth fails, the upstream returns a non-2xx
/// status, or the upstream JSON cannot be parsed.
///
/// # Panics
///
/// Panics if `axum::Response::builder` somehow fails to build a valid SSE
/// response (only possible if the constant headers above are malformed).
#[allow(clippy::too_many_lines)]
pub async fn codex_responses_passthrough(
    State(state): State<Arc<AppState>>,
    axum::extract::Json(body): axum::extract::Json<Value>,
) -> Result<Response, ApiError> {
    let mut body = body;

    // The Codex Responses API requires `instructions`; inject an empty default
    // when the client (e.g. AmpCode) omits it.
    if body.get("instructions").is_none() {
        body["instructions"] = Value::String(String::new());
    }

    let config = state.config.load();
    let api_key = config
        .providers
        .get(&ProviderId::Codex)
        .and_then(|pc| pc.api_key.clone());

    let model_name = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();

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

    let upstream_url = if is_oauth {
        CODEX_RESPONSES_URL
    } else {
        OPENAI_RESPONSES_URL
    };
    let auth_mode = if is_oauth { "oauth" } else { "api_key" };

    tracing::info!(
        model = %model_name,
        auth_mode,
        upstream_url,
        "codex responses: sending request to upstream"
    );

    let start = std::time::Instant::now();

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
    };

    let elapsed = start.elapsed();

    let resp = resp.map_err(|e| {
        tracing::error!(
            model = %model_name,
            auth_mode,
            upstream_url,
            ?elapsed,
            error = %e,
            "codex responses: transport error (DNS/TLS/connection)"
        );
        ApiError(ByokError::from(e))
    })?;

    let provider = "codex";
    let upstream_status = resp.status().as_u16();
    let status = StatusCode::from_u16(upstream_status).unwrap_or(StatusCode::BAD_GATEWAY);

    if !status.is_success() {
        let headers_dbg = format!("{:?}", resp.headers());
        let text = resp.text().await.unwrap_or_default();
        tracing::error!(
            model = %model_name,
            auth_mode,
            upstream_url,
            upstream_status,
            ?elapsed,
            response_headers = %headers_dbg,
            response_body = %text,
            "codex responses: upstream returned non-2xx"
        );
        return Err(upstream_error(
            status,
            text,
            &state.usage,
            &model_name,
            provider,
        ));
    }

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    tracing::info!(
        model = %model_name,
        upstream_status,
        ?elapsed,
        content_type = %content_type,
        "codex responses: upstream returned success"
    );

    // chatgpt.com may omit Content-Type entirely for SSE responses;
    // default to streaming when the header is absent or unrecognised.
    let is_sse = content_type.is_empty()
        || content_type.contains("text/event-stream")
        || content_type.contains("application/x-ndjson");

    if is_sse {
        let tapped = tap_usage_stream(
            response_to_stream(resp),
            state.usage.clone(),
            model_name.clone(),
            provider.to_string(),
            CodexParser::new(),
        );
        let stream_model = model_name;
        let mapped = tapped.map_err(move |e| {
            tracing::error!(
                model = %stream_model,
                error = %e,
                "codex responses: SSE stream error mid-transfer"
            );
            std::io::Error::other(e.to_string())
        });
        Ok(sse_response(status, mapped))
    } else {
        let json: Value = resp
            .json()
            .await
            .map_err(|e| ApiError(ByokError::from(e)))?;
        let (input, output) = extract_usage(&json, "/usage/input_tokens", "/usage/output_tokens");
        state
            .usage
            .record_success(&model_name, provider, input, output);
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
///
/// # Errors
///
/// Returns [`ApiError`] if Gemini auth fails, the upstream returns a non-2xx
/// status, the upstream JSON cannot be parsed, or backend translation fails.
///
/// # Panics
///
/// Panics if `axum::Response::builder` somehow fails to build a valid SSE
/// response (only possible if the constant headers above are malformed).
#[allow(clippy::implicit_hasher)] // Axum extractors fix the HashMap hasher.
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

    let provider = "gemini";
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(upstream_error(
            status,
            text,
            &state.usage,
            model_name,
            provider,
        ));
    }

    let is_sse = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("text/event-stream"));

    if is_sse {
        let tapped = tap_usage_stream(
            response_to_stream(resp),
            state.usage.clone(),
            model_name.to_string(),
            provider.to_string(),
            GeminiParser::new(),
        );
        let mapped = tapped.map_err(|e| std::io::Error::other(e.to_string()));
        Ok(sse_response(status, mapped))
    } else {
        let json: Value = resp
            .json()
            .await
            .map_err(|e| ApiError(ByokError::from(e)))?;
        let (input, output) = extract_usage(
            &json,
            "/usageMetadata/promptTokenCount",
            "/usageMetadata/candidatesTokenCount",
        );
        state
            .usage
            .record_success(model_name, provider, input, output);
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
        backend_config.base_url,
        state.auth.clone(),
        state.http.clone(),
        Some(state.ratelimits.clone()),
        &state.versions,
    )
    .ok_or_else(|| {
        ApiError::from(ByokError::UnsupportedModel(format!(
            "backend {backend_id:?} has no executor"
        )))
    })?;

    // Translate Gemini native request → OpenAI format.
    let mut openai_req: Value = byokey_translate::GeminiNativeRequest { body: &body, model }
        .try_into()
        .map_err(ApiError::from)?;

    // Inject stream flag based on the Gemini action URL.
    openai_req["stream"] = Value::Bool(is_stream);

    // Build a ChatRequest from the translated OpenAI body.
    let chat_request: byokey_types::ChatRequest =
        serde_json::from_value(openai_req).map_err(|e| {
            ApiError::from(ByokError::Translation(format!(
                "failed to parse translated request: {e}"
            )))
        })?;

    let provider_name = backend_id.to_string();

    // Send through the backend executor.
    let provider_resp = match executor.chat_completion(chat_request).await {
        Ok(r) => r,
        Err(e) => {
            state.usage.record_failure(model, &provider_name);
            return Err(ApiError::from(e));
        }
    };

    match provider_resp {
        byokey_types::traits::ProviderResponse::Complete(openai_resp) => {
            // Extract usage from the OpenAI-format response before translating.
            let usage_obj = openai_resp.get("usage");
            let input = usage_obj
                .and_then(|u| u.get("prompt_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let output = usage_obj
                .and_then(|u| u.get("completion_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            state
                .usage
                .record_success(model, &provider_name, input, output);

            let gemini_resp: Value = byokey_translate::OpenAIResponseToGemini {
                body: &openai_resp,
                model,
            }
            .try_into()
            .map_err(ApiError::from)?;
            Ok(axum::Json(gemini_resp).into_response())
        }
        byokey_types::traits::ProviderResponse::Stream(byte_stream) => {
            // Tap the OpenAI stream for usage before translating to Gemini SSE.
            let tapped = tap_usage_stream(
                byte_stream,
                state.usage.clone(),
                model.to_string(),
                provider_name,
                GeminiParser::new(),
            );
            let model_owned = model.to_string();
            let translated = byte_stream_to_gemini_sse(tapped, model_owned);
            let mapped = translated.map_err(|e| std::io::Error::other(e.to_string()));
            Ok(sse_response(StatusCode::OK, mapped))
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

/// Transparent proxy to `ampcode.com` — used for both `/api/{*path}` and
/// `/v0/management/{*path}`. Takes the original URI path directly so a
/// single handler covers all non-provider amp routes.
pub async fn ampcode_proxy(
    State(state): State<Arc<AppState>>,
    axum::extract::Extension(fwd): axum::extract::Extension<ForwardedHeaders>,
    method: Method,
    uri: axum::http::Uri,
    body: Bytes,
) -> Response {
    let path = uri.path();
    let url = match uri.query() {
        Some(q) => format!("{AMP_BACKEND}{path}?{q}"),
        None => format!("{AMP_BACKEND}{path}"),
    };

    let debug = path.ends_with("/internal") && tracing::enabled!(tracing::Level::DEBUG);
    if debug {
        let req_body = std::str::from_utf8(&body)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .map_or_else(
                || format!("{body:?}"),
                |v| serde_json::to_string_pretty(&v).unwrap_or_default(),
            );
        tracing::debug!(%method, %url, body = %req_body, "ampcode proxy request");
    }

    let resp = match state
        .http
        .request(method, url)
        .headers(fwd.headers)
        .body(body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return bad_gateway(e),
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
        tracing::debug!(%status, body = %resp_body, "ampcode proxy response");
    }

    (status, resp_headers, body_bytes).into_response()
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_urls_are_https() {
        assert!(super::CODEX_RESPONSES_URL.starts_with("https://"));
        assert!(super::OPENAI_RESPONSES_URL.starts_with("https://"));
        assert!(super::GEMINI_MODELS_BASE.starts_with("https://"));
    }
}
