//! Chat completions handler — proxies OpenAI-compatible requests to providers.

use axum::{
    Json,
    body::Body,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use byokey_provider::make_executor_for_model;
use byokey_translate::{apply_thinking, parse_model_suffix};
use byokey_types::{ProviderId, traits::ProviderResponse};
use futures_util::TryStreamExt as _;
use serde_json::Value;
use std::sync::Arc;

use crate::{AppState, error::ApiError};

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
    Json(mut body): Json<Value>,
) -> Result<Response, ApiError> {
    let raw_model = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let stream = body.get("stream").and_then(Value::as_bool).unwrap_or(false);

    // Parse thinking suffix from model name
    let suffix = parse_model_suffix(&raw_model);
    let model = &suffix.model;

    let config = state.config.load();
    let config_fn = move |p: &ProviderId| config.providers.get(p).cloned();

    let executor =
        make_executor_for_model(model, &config_fn, state.auth.clone(), state.http.clone())
            .map_err(ApiError::from)?;

    // Replace model name with the clean version (suffix stripped)
    body["model"] = Value::String(model.clone());

    // Apply thinking config if suffix was parsed
    if let Some(ref thinking) = suffix.thinking {
        let provider = byokey_provider::resolve_provider(model).unwrap_or(ProviderId::Claude);
        body = apply_thinking(body, &provider, thinking);
    }

    match executor
        .chat_completion(body, stream)
        .await
        .map_err(ApiError::from)?
    {
        ProviderResponse::Complete(json) => Ok(Json(json).into_response()),
        ProviderResponse::Stream(byte_stream) => {
            // Convert ByokError → std::io::Error for Body::from_stream
            let mapped = byte_stream.map_err(|e| std::io::Error::other(e.to_string()));
            let body = Body::from_stream(mapped);
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "text/event-stream")
                .header("cache-control", "no-cache")
                .header("x-accel-buffering", "no")
                .body(body)
                .expect("valid response"))
        }
    }
}
