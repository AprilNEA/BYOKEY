//! `POST /v1/messages` served by Cursor.
//!
//! Anthropic requests are translated to the canonical format, run through
//! [`CursorExecutor`], and the canonical events rendered back as Anthropic
//! SSE or a complete Messages response.

use aigw::anthropic::translate::{
    NativeSseContext, chat_response_to_messages, messages_request_to_canonical,
    stream_event_to_anthropic_sse,
};
use aigw::anthropic::types::MessagesRequest;
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use byokey_provider::CursorExecutor;
use byokey_types::{ByokError, traits::ByteStream};
use bytes::Bytes;
use futures_util::StreamExt as _;
use serde_json::Value;
use std::sync::Arc;

use crate::util::sse_response;
use crate::util::stream::{AnthropicParser, tap_usage_stream};
use crate::{AppState, error::ApiError};

/// Serve an Anthropic Messages request from Cursor. `body.model` is the
/// Cursor model name, without any `cursor/` qualifier.
pub(crate) async fn cursor_messages(
    state: &Arc<AppState>,
    body: Value,
    stream: bool,
) -> Result<Response, ApiError> {
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let model = model.as_str();
    let request: MessagesRequest =
        serde_json::from_value(body).map_err(|e| ByokError::Translation(e.to_string()))?;
    let canonical = messages_request_to_canonical(request)
        .map_err(|e| ByokError::Translation(e.to_string()))?;
    let config = state.config.load();
    let api_key = config
        .providers
        .get(&byokey_types::ProviderId::Cursor)
        .and_then(|c| c.api_key.clone());
    let executor = CursorExecutor::builder()
        .http(state.http.clone())
        .auth(state.auth.clone())
        .maybe_api_key(api_key)
        .build();
    let events = executor.events(canonical).await?;

    if !stream {
        let response = byokey_provider::executor::cursor::collect(events).await?;
        let messages = chat_response_to_messages(response)
            .map_err(|e| ByokError::Translation(e.to_string()))?;
        return Ok((StatusCode::OK, Json(messages)).into_response());
    }

    let mut ctx = NativeSseContext::with_model(model);
    let sse: ByteStream = Box::pin(events.map(move |e| {
        e.map(|event| {
            let frames = stream_event_to_anthropic_sse(&event, &mut ctx);
            Bytes::from(
                frames
                    .iter()
                    .flat_map(aigw::anthropic::translate::AnthropicSseFrame::to_sse_bytes)
                    .collect::<Vec<u8>>(),
            )
        })
    }));
    let tapped = tap_usage_stream(
        sse,
        state.usage.clone(),
        model.to_owned(),
        "cursor".into(),
        byokey_types::DEFAULT_ACCOUNT.into(),
        AnthropicParser::new(),
    );
    Ok(sse_response(
        StatusCode::OK,
        tapped.map(|r| r.map_err(std::io::Error::other)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_code_requests_translate_to_canonical() {
        // Claude Code marks system blocks with `"ttl": "1h"` cache markers.
        let body = serde_json::json!({
            "model": "m", "max_tokens": 1,
            "system": [{"type": "text", "text": "s", "cache_control": {"type": "ephemeral", "ttl": "1h"}}],
            "messages": [{"role": "user", "content": "hi"}],
        });
        let request: MessagesRequest = serde_json::from_value(body).unwrap();
        assert!(messages_request_to_canonical(request).is_ok());
    }
}
