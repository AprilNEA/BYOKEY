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
    http::{Method, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use byokey_types::{ByokError, ProviderId};
use bytes::Bytes;
use futures_util::TryStreamExt as _;
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

use crate::middleware::forward::ForwardedHeaders;
use crate::util::stream::{
    CodexParser, GeminiParser, OpenAIParser, response_to_stream, tap_usage_stream,
};
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

    let (is_oauth, token, account_id) = if let Some(key) = api_key {
        (false, key, byokey_types::DEFAULT_ACCOUNT.to_string())
    } else {
        let (account_id, tok) = state
            .auth
            .get_token_with_account(&ProviderId::Codex)
            .await
            .map_err(ApiError::from)?;
        (true, tok.access_token, account_id)
    };

    // chatgpt.com/backend-api/codex/responses rejects sampling, limit, and
    // stream parameters that the public OpenAI Responses API accepts.
    // AmpCode sends `stream_options` which Codex rejects with HTTP 400.
    if is_oauth && let Some(obj) = body.as_object_mut() {
        obj.remove("max_output_tokens");
        obj.remove("temperature");
        obj.remove("top_p");
        obj.remove("stream_options");
    }

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
            &account_id,
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
            account_id.clone(),
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
            .record_success_for(&model_name, provider, &account_id, input, output);
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
#[allow(clippy::implicit_hasher, clippy::too_many_lines)] // Axum extractors fix the HashMap hasher; long due to auth+SSE branching.
pub async fn gemini_native_passthrough(
    State(state): State<Arc<AppState>>,
    Path(action): Path<String>,
    Query(query_params): Query<HashMap<String, String>>,
    uri: Uri,
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

    // Preserve the original query string for transparent passthrough. The
    // parsed `query_params` map is only used for routing decisions above.
    let url = gemini_models_url(&action, uri.query());

    // API key → `x-goog-api-key`; OAuth token → `Authorization: Bearer`.
    let (auth_name, auth_value, account_id): (&'static str, String, String) =
        if let Some(key) = api_key {
            (
                "x-goog-api-key",
                key,
                byokey_types::DEFAULT_ACCOUNT.to_string(),
            )
        } else {
            let (account_id, token) = state
                .auth
                .get_token_with_account(&ProviderId::Gemini)
                .await
                .map_err(ApiError::from)?;
            (
                "authorization",
                format!("Bearer {}", token.access_token),
                account_id,
            )
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
            &account_id,
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
            account_id,
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
            .record_success_for(model_name, provider, &account_id, input, output);
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

    // Translate Gemini-native request → canonical (OpenAI shape) via aigw.
    let mut native: aigw_gemini::GenerateContentRequest = serde_json::from_value(body.clone())
        .map_err(|e| {
            ApiError::from(ByokError::Translation(format!(
                "failed to parse Gemini-native body: {e}"
            )))
        })?;
    native.model = model.to_owned();
    let canonical = aigw_gemini::translate::gemini_request_to_canonical(native)
        .map_err(|e| ApiError::from(ByokError::Translation(e.to_string())))?;
    let mut openai_req: Value = serde_json::to_value(&canonical)
        .map_err(|e| ApiError::from(ByokError::Translation(e.to_string())))?;

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
    // Backend executors do their own account rotation; the specific account
    // isn't surfaced here yet, so attribute to DEFAULT_ACCOUNT for now.
    let account_id = byokey_types::DEFAULT_ACCOUNT;

    // Send through the backend executor.
    let provider_resp = match executor.chat_completion(chat_request).await {
        Ok(r) => r,
        Err(e) => {
            state
                .usage
                .record_failure_for(model, &provider_name, account_id);
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
                .record_success_for(model, &provider_name, account_id, input, output);

            // Translate OpenAI-format response → Gemini-native via aigw.
            let canonical: aigw_core::model::ChatResponse = serde_json::from_value(openai_resp)
                .map_err(|e| ApiError::from(ByokError::Translation(e.to_string())))?;
            let mut gemini_resp = aigw_gemini::translate::chat_response_to_gemini(canonical)
                .map_err(|e| ApiError::from(ByokError::Translation(e.to_string())))?;
            // Override modelVersion with the client-requested name (canonical
            // carries the upstream model id, but Gemini-native clients expect
            // the model they asked for).
            gemini_resp.model_version = Some(model.to_owned());
            let value = serde_json::to_value(&gemini_resp)
                .map_err(|e| ApiError::from(ByokError::Translation(e.to_string())))?;
            Ok(axum::Json(value).into_response())
        }
        byokey_types::traits::ProviderResponse::Stream(byte_stream) => {
            // Tap the OpenAI stream for usage before translating to Gemini SSE.
            let tapped = tap_usage_stream(
                byte_stream,
                state.usage.clone(),
                model.to_string(),
                provider_name,
                account_id.to_string(),
                OpenAIParser::new(),
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
/// split across chunks. We buffer incoming bytes, split on newlines, parse
/// each `data: ` line through aigw-openai's [`OpenAIStreamParser`] (which
/// emits canonical [`StreamEvent`]s), then feed each event through
/// aigw-gemini's [`stream_event_to_gemini_sse`] bridge to produce
/// Gemini-native SSE bytes.
///
/// [`OpenAIStreamParser`]: aigw_openai::translate::OpenAIStreamParser
/// [`StreamEvent`]: aigw_core::model::StreamEvent
/// [`stream_event_to_gemini_sse`]: aigw_gemini::translate::stream_event_to_gemini_sse
fn byte_stream_to_gemini_sse(
    upstream: byokey_types::traits::ByteStream,
    model: String,
) -> impl futures_util::Stream<Item = std::result::Result<Bytes, ByokError>> {
    use aigw_core::model::StreamEvent;
    use aigw_core::translate::StreamParser as _;
    use aigw_gemini::translate::{NativeSseContext, stream_event_to_gemini_sse};
    use aigw_openai::translate::OpenAIStreamParser;
    use futures_util::{StreamExt as _, stream::try_unfold};
    use std::collections::VecDeque;

    struct State {
        upstream: byokey_types::traits::ByteStream,
        buffer: Vec<u8>,
        output: VecDeque<Bytes>,
        parser: OpenAIStreamParser,
        ctx: NativeSseContext,
        model: String,
        finished: bool,
    }

    fn push_events(state: &mut State, events: &[StreamEvent]) {
        for ev in events {
            match ev {
                StreamEvent::ResponseMeta { id, .. } => {
                    let event = StreamEvent::ResponseMeta {
                        id: id.clone(),
                        model: state.model.clone(),
                    };
                    if let Some(bytes) = stream_event_to_gemini_sse(&event, &mut state.ctx) {
                        state.output.push_back(Bytes::from(bytes));
                    }
                }
                _ => {
                    if let Some(bytes) = stream_event_to_gemini_sse(ev, &mut state.ctx) {
                        state.output.push_back(Bytes::from(bytes));
                    }
                }
            }
        }
    }

    fn process_sse_line(state: &mut State, raw: &[u8]) -> Result<(), ByokError> {
        let line = String::from_utf8_lossy(raw);
        let line = line.trim_end_matches(['\r', '\n']);
        let Some(data) = line.strip_prefix("data:").map(str::trim_start) else {
            return Ok(());
        };
        let events = state.parser.parse_event("", data).map_err(|e| {
            ByokError::Translation(format!("failed to parse backend OpenAI SSE data: {e}"))
        })?;
        push_events(state, &events);
        Ok(())
    }

    fn process_complete_lines(state: &mut State) -> Result<(), ByokError> {
        while let Some(pos) = state.buffer.iter().position(|&b| b == b'\n') {
            let raw: Vec<u8> = state.buffer.drain(..=pos).collect();
            process_sse_line(state, &raw)?;
        }
        Ok(())
    }

    try_unfold(
        State {
            upstream,
            buffer: Vec::new(),
            output: VecDeque::new(),
            parser: OpenAIStreamParser::default(),
            ctx: NativeSseContext::with_model(model.clone()),
            model,
            finished: false,
        },
        |mut state| async move {
            loop {
                if let Some(bytes) = state.output.pop_front() {
                    return Ok(Some((bytes, state)));
                }
                if state.finished {
                    return Ok(None);
                }

                match state.upstream.next().await {
                    Some(Ok(chunk)) => {
                        state.buffer.extend_from_slice(&chunk);
                        process_complete_lines(&mut state)?;
                    }
                    Some(Err(e)) => return Err(e),
                    None => {
                        if !state.buffer.is_empty() {
                            let raw = std::mem::take(&mut state.buffer);
                            process_sse_line(&mut state, &raw)?;
                        }
                        let events = state.parser.finish().map_err(|e| {
                            ByokError::Translation(format!(
                                "failed to finish backend OpenAI SSE parser: {e}"
                            ))
                        })?;
                        push_events(&mut state, &events);
                        state.finished = true;
                    }
                }
            }
        },
    )
}

fn gemini_models_url(action: &str, raw_query: Option<&str>) -> String {
    if let Some(qs) = raw_query.filter(|qs| !qs.is_empty()) {
        format!("{GEMINI_MODELS_BASE}/{action}?{qs}")
    } else {
        format!("{GEMINI_MODELS_BASE}/{action}")
    }
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
    use byokey_types::traits::ByteStream;
    use bytes::Bytes;
    use futures_util::{StreamExt as _, stream};
    use serde_json::Value;

    #[test]
    fn test_urls_are_https() {
        assert!(super::CODEX_RESPONSES_URL.starts_with("https://"));
        assert!(super::OPENAI_RESPONSES_URL.starts_with("https://"));
        assert!(super::GEMINI_MODELS_BASE.starts_with("https://"));
    }

    #[test]
    fn gemini_models_url_preserves_raw_query_string() {
        let url = super::gemini_models_url(
            "gemini-3-pro:streamGenerateContent",
            Some("alt=sse&x=a%2Bb&x=c"),
        );
        assert_eq!(
            url,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3-pro:streamGenerateContent?alt=sse&x=a%2Bb&x=c"
        );
    }

    fn stream_from_chunks(chunks: &[&str]) -> ByteStream {
        let chunks: Vec<_> = chunks
            .iter()
            .map(|chunk| Ok(Bytes::from((*chunk).to_owned())))
            .collect();
        Box::pin(stream::iter(chunks))
    }

    async fn collect_gemini_values(chunks: &[&str]) -> Vec<Value> {
        super::byte_stream_to_gemini_sse(stream_from_chunks(chunks), "gemini-test".to_owned())
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(|result| {
                let bytes = result.unwrap();
                let line = std::str::from_utf8(&bytes).unwrap();
                let data = line.strip_prefix("data: ").unwrap().trim();
                serde_json::from_str(data).unwrap()
            })
            .collect()
    }

    #[tokio::test]
    async fn backend_stream_flushes_final_line_without_newline() {
        let values = collect_gemini_values(&[r#"data: {"id":"c","object":"chat.completion.chunk","created":0,"model":"gpt-backend","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}"#]).await;

        assert_eq!(values.len(), 1);
        assert_eq!(
            values[0]["candidates"][0]["content"]["parts"][0]["text"],
            "Hello"
        );
        assert_eq!(values[0]["modelVersion"], "gemini-test");
    }

    #[tokio::test]
    async fn backend_stream_done_flushes_pending_tool_call() {
        let values = collect_gemini_values(&[
            r#"data: {"id":"c","object":"chat.completion.chunk","created":0,"model":"gpt-backend","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{\"location\":\"SF\"}"}}]},"finish_reason":null}]}

"#,
            "data: [DONE]\n\n",
        ])
        .await;

        assert_eq!(values.len(), 1);
        let function_call = &values[0]["candidates"][0]["content"]["parts"][0]["functionCall"];
        assert_eq!(function_call["id"], "call_1");
        assert_eq!(function_call["name"], "get_weather");
        assert_eq!(function_call["args"]["location"], "SF");
    }
}
