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
use byokey_types::ByokError;
use serde_json::{Value, json};
use std::sync::Arc;

use super::messages::{
    AnthropicUpstream, Backend, build_beta_header, copilot_executor, copilot_request,
    sanitize_system, strip_copilot_unsupported,
};
use crate::{AppState, error::ApiError};

/// Handles `POST /v1/messages/count_tokens`.
pub async fn count_tokens(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(mut body): Json<Value>,
) -> Result<Response, ApiError> {
    sanitize_system(&mut body);
    let beta = build_beta_header(&mut body, &headers);
    let config = state.config.load();
    let resp = match Backend::route(&config, &mut body) {
        Backend::Cursor => {
            return Ok(Json(json!({"input_tokens": estimate(&body)})).into_response());
        }
        Backend::Copilot => {
            strip_copilot_unsupported(&mut body);
            let (executor, identity) = copilot_executor(&state);
            let creds = executor.credentials().await?;
            let conversation = Conversation::from_messages(&[]);
            copilot_request(
                &state.http,
                "/v1/messages/count_tokens",
                &creds,
                &beta,
                &identity,
                &conversation,
                &body,
            )
            .send()
            .await
        }
        Backend::Anthropic => {
            let profile = state.device_profiles.resolve("global");
            let upstream = AnthropicUpstream::resolve(&state, &config, &profile, &beta).await?;
            let url = format!(
                "{}?beta=true",
                upstream.transport.url("/v1/messages/count_tokens")
            );
            upstream.request(&state.http, &url).json(&body).send().await
        }
    }
    .map_err(|e| ApiError(ByokError::from(e)))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| ApiError(ByokError::from(e)))?;
    if !status.is_success() {
        return Err(ApiError(ByokError::Upstream {
            status: status.as_u16(),
            body: text,
            retry_after: None,
        }));
    }
    let code = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::OK);
    Ok((code, [("content-type", "application/json")], text).into_response())
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
