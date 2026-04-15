//! Chat completions handler — proxies OpenAI-compatible requests to providers.

use axum::{
    Json,
    body::Body,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use byokey_provider::{make_executor_for_model, parse_qualified_model};
use byokey_translate::{apply_thinking, parse_model_suffix};
use byokey_types::traits::ByteStream;
use byokey_types::{ChatRequest, ProviderId, traits::ProviderResponse};
use futures_util::{StreamExt as _, TryStreamExt as _, stream::try_unfold};
use std::collections::HashSet;
use std::sync::Arc;

use crate::{AppState, UsageRecorder, error::ApiError};

/// Wraps a [`ByteStream`] to transparently extract token usage from SSE chunks.
///
/// Each chunk is forwarded unchanged. When the stream ends, the accumulated
/// `prompt_tokens` / `completion_tokens` are recorded via [`UsageRecorder`].
pub(crate) fn tap_stream_usage(
    inner: ByteStream,
    usage: Arc<UsageRecorder>,
    model: String,
    provider: String,
) -> ByteStream {
    struct State {
        inner: ByteStream,
        scan_buf: Vec<u8>,
        usage: Arc<UsageRecorder>,
        model: String,
        provider: String,
        input_tokens: u64,
        output_tokens: u64,
    }

    Box::pin(try_unfold(
        State {
            inner,
            scan_buf: Vec::new(),
            usage,
            model,
            provider,
            input_tokens: 0,
            output_tokens: 0,
        },
        |mut s| async move {
            match s.inner.next().await {
                Some(Ok(bytes)) => {
                    // Scan for usage data in SSE lines
                    s.scan_buf.extend_from_slice(&bytes);
                    while let Some(nl) = s.scan_buf.iter().position(|&b| b == b'\n') {
                        let line: Vec<u8> = s.scan_buf.drain(..=nl).collect();
                        let line = String::from_utf8_lossy(&line);
                        let line = line.trim();
                        if let Some(data) = line.strip_prefix("data: ")
                            && data != "[DONE]"
                            && let Ok(json) = serde_json::from_str::<serde_json::Value>(data)
                            && let Some(usage) = json.get("usage")
                        {
                            if let Some(v) = usage
                                .get("prompt_tokens")
                                .and_then(serde_json::Value::as_u64)
                            {
                                s.input_tokens = v;
                            }
                            if let Some(v) = usage
                                .get("completion_tokens")
                                .and_then(serde_json::Value::as_u64)
                            {
                                s.output_tokens = v;
                            }
                        }
                    }
                    // Forward original bytes unchanged
                    Ok(Some((bytes, s)))
                }
                Some(Err(e)) => {
                    s.usage.record_failure(&s.model, &s.provider);
                    Err(e)
                }
                None => {
                    // Stream ended — record accumulated usage
                    s.usage
                        .record_success(&s.model, &s.provider, s.input_tokens, s.output_tokens);
                    Ok(None)
                }
            }
        },
    ))
}

/// Extract input/output token counts from an OpenAI-compatible usage response.
fn extract_usage_tokens(json: &serde_json::Value) -> (u64, u64) {
    let usage = json.get("usage");
    let input = usage
        .and_then(|u| u.get("prompt_tokens"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let output = usage
        .and_then(|u| u.get("completion_tokens"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    (input, output)
}

/// Handles `POST /copilot/v1/chat/completions` — always routes through Copilot.
pub async fn copilot_chat_completions(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ChatRequest>,
) -> Result<Response, ApiError> {
    chat_completions_inner(state, request, true).await
}

/// Handles `POST /v1/chat/completions` requests.
///
/// Resolves the model to a provider, forwards the request, and returns
/// either a complete JSON response or an SSE stream.
///
/// # Errors
///
/// Returns [`ApiError`] if the model is unsupported or the upstream call fails.
pub async fn chat_completions(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ChatRequest>,
) -> Result<Response, ApiError> {
    chat_completions_inner(state, request, false).await
}

#[tracing::instrument(skip_all, fields(
    model = %request.model,
    force_copilot,
    provider = tracing::field::Empty,
    bare_model = tracing::field::Empty,
))]
async fn chat_completions_inner(
    state: Arc<AppState>,
    mut request: ChatRequest,
    force_copilot: bool,
) -> Result<Response, ApiError> {
    let config = state.config.load();

    // Pre-compute which providers have OAuth tokens (async → sync bridge).
    let mut oauth_providers = HashSet::new();
    for p in ProviderId::all() {
        if state.auth.is_authenticated(p).await {
            oauth_providers.insert(p.clone());
        }
    }

    // Resolve model alias before anything else.
    let resolved_model = config.resolve_alias(&request.model);

    // Strip provider qualifier (e.g. "codex/gpt-5.4" → "gpt-5.4").
    let (provider_hint, bare_model) = parse_qualified_model(&resolved_model);

    // Parse thinking suffix from (possibly alias-resolved) model name.
    let suffix = parse_model_suffix(bare_model);

    let config_fn = |p: &ProviderId| {
        let mut pc = config.providers.get(p).cloned().unwrap_or_default();
        if force_copilot && *p != ProviderId::Copilot {
            pc.backend = Some(ProviderId::Copilot);
        }
        Some(pc)
    };

    let executor = make_executor_for_model(
        &suffix.model,
        config_fn,
        &oauth_providers,
        provider_hint.as_ref(),
        state.auth.clone(),
        state.http.clone(),
        Some(state.ratelimits.clone()),
    )
    .map_err(ApiError::from)?;

    let provider = byokey_provider::resolve_provider(&suffix.model)
        .map_or_else(|| "unknown".to_string(), |p| p.to_string());
    let span = tracing::Span::current();
    span.record("provider", provider.as_str());
    span.record("bare_model", bare_model);
    tracing::info!(stream = request.stream, "chat completion request");

    // Replace model name with the clean version (suffix stripped)
    request.model.clone_from(&suffix.model);

    // Apply thinking config if suffix was parsed
    if let Some(ref thinking) = suffix.thinking {
        let provider =
            byokey_provider::resolve_provider(&suffix.model).unwrap_or(ProviderId::Claude);
        let capability = byokey_provider::thinking_capability(&suffix.model);
        let mut body = request.into_body();
        body = apply_thinking(body, &provider, thinking, capability);
        // Re-parse the modified body back into ChatRequest
        request = serde_json::from_value(body)
            .map_err(|e| ApiError::from(byokey_types::ByokError::Translation(e.to_string())))?;
    }

    // Apply payload rules (default/override/filter) based on model name.
    if !config.payload.default.is_empty()
        || !config.payload.r#override.is_empty()
        || !config.payload.filter.is_empty()
    {
        let mut body = request.into_body();
        body = config.apply_payload_rules(body, &suffix.model);
        request = serde_json::from_value(body)
            .map_err(|e| ApiError::from(byokey_types::ByokError::Translation(e.to_string())))?;
    }

    let model_name = suffix.model.clone();
    match executor.chat_completion(request).await {
        Ok(ProviderResponse::Complete(json)) => {
            // Extract token usage from the response if available.
            let (input_tok, output_tok) = extract_usage_tokens(&json);
            state
                .usage
                .record_success(&model_name, &provider, input_tok, output_tok);
            tracing::debug!(model = %model_name, "chat completion complete");
            Ok(Json(json).into_response())
        }
        Ok(ProviderResponse::Stream(byte_stream)) => {
            tracing::debug!(model = %model_name, "streaming chat completion");
            // Wrap stream to extract usage from SSE chunks; records on completion.
            let tapped = tap_stream_usage(
                byte_stream,
                state.usage.clone(),
                model_name,
                provider.clone(),
            );
            let mapped = tapped.map_err(|e| std::io::Error::other(e.to_string()));
            let body = Body::from_stream(mapped);
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "text/event-stream")
                .header("cache-control", "no-cache")
                .header("x-accel-buffering", "no")
                .body(body)
                .expect("valid response"))
        }
        Err(e) => {
            state.usage.record_failure(&model_name, &provider);
            Err(ApiError::from(e))
        }
    }
}
