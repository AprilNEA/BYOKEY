//! `POST /v1/messages` served by Cursor.
//!
//! Anthropic requests are translated to the canonical format, run through
//! [`CursorUpstream`], and the canonical events rendered back as Anthropic
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
use byokey_provider::CursorUpstream;
use byokey_types::{ByokError, ProviderId, traits::ByteStream};
use bytes::Bytes;
use futures_util::StreamExt as _;
use serde_json::Value;
use std::sync::Arc;

use super::forward::end_with;
use crate::exchange::Exchange;
use crate::util::sse_response;
use crate::util::stream::{deliver_anthropic_stream, keep_alive};
use std::time::Duration;

/// See `forward::KEEPALIVE_INTERVAL`; a Cursor run parked on a tool call
/// can legitimately sit silent, so it gets no silence limit.
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(10);
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
    let cursor = CursorUpstream::builder()
        .http(state.http.clone())
        .auth(state.auth.clone())
        .maybe_api_key(api_key)
        .build();
    let exchange = Exchange::start(
        &state.usage,
        ProviderId::Cursor,
        model,
        byokey_types::DEFAULT_ACCOUNT,
    );
    let events = match cursor.events(canonical).await {
        Ok(events) => events,
        Err(e) => return Err(end_with(exchange, e)),
    };

    if !stream {
        let messages = match byokey_provider::cursor::collect(events)
            .await
            .and_then(|response| {
                chat_response_to_messages(response)
                    .map_err(|e| ByokError::Translation(e.to_string()))
            })
            .and_then(|messages| serde_json::to_value(messages).map_err(ByokError::from))
        {
            Ok(messages) => messages,
            Err(e) => return Err(end_with(exchange, e)),
        };
        exchange.complete_with(&messages);
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
    let alive = keep_alive(sse, KEEPALIVE_INTERVAL, Duration::MAX);
    Ok(sse_response(
        StatusCode::OK,
        deliver_anthropic_stream(alive, exchange).map(|r| r.map_err(std::io::Error::other)),
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
