//! Chat completions handler — proxies OpenAI-compatible requests to providers.

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use byokey_provider::{make_executor_for_model, parse_qualified_model};
use byokey_translate::{apply_thinking, parse_model_suffix};
use byokey_types::{ChatRequest, ProviderId, traits::ProviderResponse};
use futures_util::TryStreamExt as _;
use std::collections::HashSet;
use std::sync::Arc;

use crate::util::stream::{OpenAIParser, tap_usage_stream};
use crate::util::{extract_usage, sse_response};
use crate::{AppState, error::ApiError};

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
            let (input_tok, output_tok) =
                extract_usage(&json, "/usage/prompt_tokens", "/usage/completion_tokens");
            state
                .usage
                .record_success(&model_name, &provider, input_tok, output_tok);
            tracing::debug!(model = %model_name, "chat completion complete");
            Ok(Json(json).into_response())
        }
        Ok(ProviderResponse::Stream(byte_stream)) => {
            tracing::debug!(model = %model_name, "streaming chat completion");
            let tapped = tap_usage_stream(
                byte_stream,
                state.usage.clone(),
                model_name,
                provider.clone(),
                OpenAIParser::new(),
            );
            let mapped = tapped.map_err(|e| std::io::Error::other(e.to_string()));
            Ok(sse_response(StatusCode::OK, mapped))
        }
        Err(e) => {
            state.usage.record_failure(&model_name, &provider);
            Err(ApiError::from(e))
        }
    }
}
