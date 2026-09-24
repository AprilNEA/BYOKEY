//! `OpenAI` Responses API route handler, served by Codex.

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use byokey_types::{ByokError, ProviderId};
use futures_util::TryStreamExt as _;
use serde_json::Value;
use std::sync::Arc;

use crate::util::stream::{CodexParser, response_to_stream, tap_usage_stream};
use crate::util::{extract_usage, sse_response, upstream_error};
use crate::{AppState, error::ApiError};

const CODEX_RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
const OPENAI_RESPONSES_URL: &str = "https://api.openai.com/v1/responses";
const CODEX_VERSION: &str = "0.120.0";
const CODEX_USER_AGENT: &str = "codex-tui/0.120.0 (Mac OS 26.0.1; arm64) Apple_Terminal/464";

/// Handles `POST /v1/responses`, forwarding `OpenAI` Responses API
/// requests to Codex.
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
pub async fn codex_responses(
    State(state): State<Arc<AppState>>,
    axum::extract::Json(body): axum::extract::Json<Value>,
) -> Result<Response, ApiError> {
    let mut body = body;

    // The Codex Responses API requires `instructions`; inject an empty default
    // when the client omits it.
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
    // stream parameters that the public OpenAI Responses API accepts
    // (`stream_options` fails with HTTP 400).
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
