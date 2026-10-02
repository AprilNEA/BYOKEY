//! `POST /v1/messages/count_tokens`, routed like `/v1/messages`.
//!
//! Anthropic and Copilot count natively. Cursor has no counting endpoint for
//! its agent protocol, so its requests get an estimate; Claude Code would
//! otherwise estimate from characters itself.

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use byokey_provider::Conversation;
use byokey_provider::claude::ANTHROPIC_VERSION;
use byokey_types::{ByokError, ProviderId};
use serde_json::{Value, json};
use std::sync::Arc;

use super::copilot::{copilot_request, copilot_upstream, strip_server_tools};
use super::copilot_messages::strip_copilot_unsupported;
use super::messages::{AnthropicUpstream, route};
use super::normalize::{
    CONTEXT_1M_BETA, build_beta_header, sanitize_system, take_long_context_suffix,
};
use crate::{AppState, error::ApiError};

/// Handles `POST /v1/messages/count_tokens`.
pub async fn count_tokens(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    super::record_model(&body);
    serve_count_tokens(&state, &headers, body)
        .await
        .map_err(ApiError::anthropic)
}

async fn serve_count_tokens(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    mut body: Value,
) -> Result<Response, ApiError> {
    let long_context = take_long_context_suffix(&mut body);
    sanitize_system(&mut body);
    let beta = build_beta_header(&mut body, headers, long_context.then_some(CONTEXT_1M_BETA));
    let config = state.config.load();
    let resp = match route(&config, &mut body) {
        ProviderId::Cursor => {
            return Ok(Json(json!({"input_tokens": estimate(&body)})).into_response());
        }
        ProviderId::Copilot => {
            strip_copilot_unsupported(&mut body);
            let copilot = copilot_upstream(state);
            let creds = copilot.credentials().await?;
            // Counting is not worth a learning round trip: leave out what
            // the account is already known to reject.
            strip_server_tools(&mut body, &creds.rejected_tools());
            let conversation = Conversation::from_messages(&[]);
            copilot_request(
                &state.http,
                "/v1/messages/count_tokens",
                &creds,
                copilot.identity(),
                &conversation,
                &body,
            )
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("anthropic-beta", &beta)
            .send()
            .await
        }
        ProviderId::Claude => {
            let profile = state.device_profiles.resolve("global");
            let upstream = AnthropicUpstream::resolve(state, &config, &profile, &beta).await?;
            upstream
                .request(&state.http, "/v1/messages/count_tokens")
                .json(&body)
                .send()
                .await
        }
    }
    .map_err(|e| ApiError::from(ByokError::from(e)))?;

    if !resp.status().is_success() {
        return Err(ApiError::from(ByokError::from_response(resp).await));
    }
    let text = resp
        .text()
        .await
        .map_err(|e| ApiError::from(ByokError::from(e)))?;
    Ok((StatusCode::OK, [("content-type", "application/json")], text).into_response())
}

/// Approximate input tokens: every text-like string in the request, at the
/// usual ~4 characters per token.
// ponytail: character heuristic; a real tokenizer if estimates prove too rough.
fn estimate(body: &Value) -> u64 {
    fn chars(v: &Value) -> usize {
        match v {
            Value::String(s) => s.chars().count(),
            Value::Array(items) => items.iter().map(chars).sum(),
            Value::Object(map) => map
                .iter()
                .filter(|(k, _)| {
                    !matches!(
                        k.as_str(),
                        "type" | "id" | "tool_use_id" | "cache_control" | "media_type"
                    )
                })
                .map(|(_, v)| chars(v))
                .sum(),
            _ => 0,
        }
    }
    let n: usize = ["system", "messages", "tools"]
        .iter()
        .filter_map(|k| body.get(*k))
        .map(chars)
        .sum();
    (n as u64).div_ceil(4).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_counts_text_not_structure() {
        let body = json!({
            "system": "abcd",
            "messages": [{"role": "user", "content": [{"type": "text", "text": "efgh"}]}],
        });
        // "abcd" + "user" + "efgh" = 12 characters → 3 tokens.
        assert_eq!(estimate(&body), 3);
        assert_eq!(estimate(&json!({})), 1);
    }
}
