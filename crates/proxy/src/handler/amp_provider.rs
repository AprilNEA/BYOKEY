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
use futures_util::{StreamExt as _, TryStreamExt as _, stream::try_unfold};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

use crate::{AppState, UsageRecorder, error::ApiError};

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

use super::{CLIENT_AUTH_HEADERS, FINGERPRINT_HEADERS, HOP_BY_HOP};

// ── Usage extraction helpers ────────────────────────────────────────────

/// Extract token counts from a Codex Responses API non-streaming response.
fn extract_codex_usage(json: &Value) -> (u64, u64) {
    let usage = json.get("usage");
    let input = usage
        .and_then(|u| u.get("input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output = usage
        .and_then(|u| u.get("output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    (input, output)
}

/// Wraps a raw byte stream to extract token usage from Codex Responses SSE events.
///
/// Scans for `response.completed` events containing `response.usage.input_tokens`
/// and `response.usage.output_tokens`, forwards all bytes unchanged.
fn tap_codex_stream_usage(
    resp: rquest::Response,
    usage: Arc<UsageRecorder>,
    model: String,
    provider: String,
) -> byokey_types::traits::ByteStream {
    use byokey_types::ByokError as BE;

    struct State {
        inner: byokey_types::traits::ByteStream,
        buf: Vec<u8>,
        usage: Arc<UsageRecorder>,
        model: String,
        provider: String,
        input_tokens: u64,
        output_tokens: u64,
    }

    let inner: byokey_types::traits::ByteStream =
        Box::pin(resp.bytes_stream().map(|r| r.map_err(BE::from)));

    Box::pin(try_unfold(
        State {
            inner,
            buf: Vec::new(),
            usage,
            model,
            provider,
            input_tokens: 0,
            output_tokens: 0,
        },
        |mut s| async move {
            match s.inner.next().await {
                Some(Ok(bytes)) => {
                    s.buf.extend_from_slice(&bytes);
                    while let Some(nl) = s.buf.iter().position(|&b| b == b'\n') {
                        let line: Vec<u8> = s.buf.drain(..=nl).collect();
                        let line = String::from_utf8_lossy(&line);
                        let line = line.trim();
                        if let Some(data) = line.strip_prefix("data: ")
                            && let Ok(ev) = serde_json::from_str::<Value>(data)
                            && ev.get("type").and_then(Value::as_str) == Some("response.completed")
                        {
                            if let Some(v) = ev
                                .pointer("/response/usage/input_tokens")
                                .and_then(Value::as_u64)
                            {
                                s.input_tokens = v;
                            }
                            if let Some(v) = ev
                                .pointer("/response/usage/output_tokens")
                                .and_then(Value::as_u64)
                            {
                                s.output_tokens = v;
                            }
                        }
                    }
                    Ok(Some((bytes, s)))
                }
                Some(Err(e)) => {
                    s.usage.record_failure(&s.model, &s.provider);
                    Err(e)
                }
                None => {
                    s.usage
                        .record_success(&s.model, &s.provider, s.input_tokens, s.output_tokens);
                    Ok(None)
                }
            }
        },
    ))
}

/// Extract token counts from a Gemini native non-streaming response.
fn extract_gemini_usage(json: &Value) -> (u64, u64) {
    let meta = json.get("usageMetadata");
    let input = meta
        .and_then(|u| u.get("promptTokenCount"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output = meta
        .and_then(|u| u.get("candidatesTokenCount"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    (input, output)
}

/// Wraps a raw byte stream to extract token usage from Gemini native SSE events.
///
/// Scans each SSE data line for `usageMetadata` (last occurrence wins).
fn tap_gemini_stream_usage(
    resp: rquest::Response,
    usage: Arc<UsageRecorder>,
    model: String,
    provider: String,
) -> byokey_types::traits::ByteStream {
    use byokey_types::ByokError as BE;

    struct State {
        inner: byokey_types::traits::ByteStream,
        buf: Vec<u8>,
        usage: Arc<UsageRecorder>,
        model: String,
        provider: String,
        input_tokens: u64,
        output_tokens: u64,
    }

    let inner: byokey_types::traits::ByteStream =
        Box::pin(resp.bytes_stream().map(|r| r.map_err(BE::from)));

    Box::pin(try_unfold(
        State {
            inner,
            buf: Vec::new(),
            usage,
            model,
            provider,
            input_tokens: 0,
            output_tokens: 0,
        },
        |mut s| async move {
            match s.inner.next().await {
                Some(Ok(bytes)) => {
                    s.buf.extend_from_slice(&bytes);
                    while let Some(nl) = s.buf.iter().position(|&b| b == b'\n') {
                        let line: Vec<u8> = s.buf.drain(..=nl).collect();
                        let line = String::from_utf8_lossy(&line);
                        let line = line.trim();
                        if let Some(data) = line.strip_prefix("data: ")
                            && let Ok(ev) = serde_json::from_str::<Value>(data)
                            && ev.get("usageMetadata").is_some()
                        {
                            if let Some(v) = ev
                                .pointer("/usageMetadata/promptTokenCount")
                                .and_then(Value::as_u64)
                            {
                                s.input_tokens = v;
                            }
                            if let Some(v) = ev
                                .pointer("/usageMetadata/candidatesTokenCount")
                                .and_then(Value::as_u64)
                            {
                                s.output_tokens = v;
                            }
                        }
                    }
                    Ok(Some((bytes, s)))
                }
                Some(Err(e)) => {
                    s.usage.record_failure(&s.model, &s.provider);
                    Err(e)
                }
                None => {
                    s.usage
                        .record_success(&s.model, &s.provider, s.input_tokens, s.output_tokens);
                    Ok(None)
                }
            }
        },
    ))
}

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

    let provider = "codex";
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        state.usage.record_failure(&model_name, provider);
        return Err(ApiError::from(ByokError::Upstream {
            status: status.as_u16(),
            body: text,
            retry_after: None,
        }));
    }

    let is_sse = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("text/event-stream"));

    if is_sse {
        let tapped =
            tap_codex_stream_usage(resp, state.usage.clone(), model_name, provider.to_string());
        let mapped = tapped.map_err(|e| std::io::Error::other(e.to_string()));
        Ok(Response::builder()
            .status(status)
            .header("content-type", "text/event-stream")
            .header("cache-control", "no-cache")
            .header("x-accel-buffering", "no")
            .body(Body::from_stream(mapped))
            .expect("valid response"))
    } else {
        let json: Value = resp
            .json()
            .await
            .map_err(|e| ApiError(ByokError::from(e)))?;
        let (input, output) = extract_codex_usage(&json);
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
        state.usage.record_failure(model_name, provider);
        return Err(ApiError::from(ByokError::Upstream {
            status: status.as_u16(),
            body: text,
            retry_after: None,
        }));
    }

    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_string();

    if content_type.contains("text/event-stream") {
        let tapped = tap_gemini_stream_usage(
            resp,
            state.usage.clone(),
            model_name.to_string(),
            provider.to_string(),
        );
        let mapped = tapped.map_err(|e| std::io::Error::other(e.to_string()));
        Ok(Response::builder()
            .status(status)
            .header("content-type", content_type)
            .header("cache-control", "no-cache")
            .header("x-accel-buffering", "no")
            .body(Body::from_stream(mapped))
            .expect("valid response"))
    } else {
        let json: Value = resp
            .json()
            .await
            .map_err(|e| ApiError(ByokError::from(e)))?;
        let (input, output) = extract_gemini_usage(&json);
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
            let tapped = super::chat::tap_stream_usage(
                byte_stream,
                state.usage.clone(),
                model.to_string(),
                provider_name,
            );
            let model_owned = model.to_string();
            let translated = byte_stream_to_gemini_sse(tapped, model_owned);
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

/// Handles `ANY /api/{*path}` — forwards non-provider `ampcode.com` management
/// routes (auth, threads, telemetry, etc.) transparently to the upstream.
#[allow(clippy::too_many_lines)]
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

    // Resolve AMP auth: stored BYOKEY token > upstream_key > client passthrough.
    let amp_token = state.auth.get_token(&ProviderId::Amp).await.ok();
    let strip_client_auth = amp_token.is_some() || config.amp.upstream_key.is_some();

    let mut upstream_headers = rquest::header::HeaderMap::new();
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
            upstream_headers.insert(n, v);
        }
    }

    // Inject auth: stored token takes priority over upstream_key.
    if let Some(token) = &amp_token {
        inject_amp_auth(&mut upstream_headers, &token.access_token);
    } else if let Some(key) = &config.amp.upstream_key {
        inject_amp_auth(&mut upstream_headers, key);
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

    let mut body_bytes = resp.bytes().await.unwrap_or_default();

    // Cache AmpCode quota data from intercepted responses (before any rewriting).
    if status.is_success()
        && let Some(q) = query.as_deref()
    {
        if q.contains("getUserFreeTierStatus")
            && let Ok(json) = serde_json::from_slice::<Value>(&body_bytes)
            && let Some(result) = json.get("result")
        {
            let can_use = result
                .get("canUseAmpFree")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let daily_grant = result
                .get("isDailyGrantEnabled")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            state.amp_quota.update_free_tier(can_use, daily_grant);
        } else if q.contains("userDisplayBalanceInfo")
            && let Ok(json) = serde_json::from_slice::<Value>(&body_bytes)
            && let Some(display_text) = json.pointer("/result/displayText").cloned()
        {
            state.amp_quota.update_balance(display_text);
        }
    }

    // Hide free-tier ads: rewrite getUserFreeTierStatus response when enabled.
    if config.amp.hide_free_tier
        && query
            .as_deref()
            .is_some_and(|q| q.contains("getUserFreeTierStatus"))
        && let Ok(mut json) = serde_json::from_slice::<serde_json::Value>(&body_bytes)
    {
        if let Some(result) = json.get_mut("result").and_then(|r| r.as_object_mut()) {
            result.insert("canUseAmpFree".into(), serde_json::Value::Bool(false));
            result.insert("isDailyGrantEnabled".into(), serde_json::Value::Bool(false));
        }
        if let Ok(rewritten) = serde_json::to_vec(&json) {
            body_bytes = Bytes::from(rewritten);
            resp_headers.remove(axum::http::header::CONTENT_LENGTH);
        }
    }

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
