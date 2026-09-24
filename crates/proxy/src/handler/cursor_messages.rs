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

/// Serve an Anthropic Messages request from Cursor. `model` is the Cursor
/// model name, without any `cursor/` qualifier.
pub(crate) async fn cursor_messages(
    state: &Arc<AppState>,
    mut body: Value,
    model: &str,
    stream: bool,
) -> Result<Response, ApiError> {
    body["model"] = Value::String(model.to_owned());
    strip_cache_control(&mut body);
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

/// Remove every `cache_control` marker. Cursor has no prompt caching, and
/// Claude Code's `"ttl": "1h"` strings do not fit aigw's numeric TTL.
fn strip_cache_control(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("cache_control");
            map.values_mut().for_each(strip_cache_control);
        }
        Value::Array(items) => items.iter_mut().for_each(strip_cache_control),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_control_is_removed_at_any_depth() {
        let mut body = serde_json::json!({
            "system": [{"type": "text", "text": "s", "cache_control": {"type": "ephemeral", "ttl": "1h"}}],
            "messages": [{"role": "user", "content": [{"type": "text", "text": "hi", "cache_control": {}}]}],
        });
        strip_cache_control(&mut body);
        assert!(!body.to_string().contains("cache_control"));
        assert_eq!(body["system"][0]["text"], "s");
    }
}
