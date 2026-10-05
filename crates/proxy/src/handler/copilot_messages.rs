//! `POST /v1/messages` served by Copilot's own Anthropic-format Messages
//! endpoint: unsupported fields are dropped before [`send_to_copilot`].
//! The client selects the model; requests without tools are not necessarily
//! background work.

use axum::response::Response;
use byokey_provider::Conversation;
use byokey_provider::claude::ANTHROPIC_VERSION;
use serde_json::Value;
use std::sync::Arc;

use super::copilot::{CopilotCall, send_to_copilot};
use crate::{AppState, error::ApiError};

/// Drop the request fields Copilot's `/v1/messages` answers with "Extra
/// inputs are not permitted": the per-message `output_config` of the
/// `per-turn-control` beta, the top-level `safeguards`, and the `scope` of
/// `cache_control` markers, all of which Claude Code sends by default.
pub(super) fn strip_copilot_unsupported(body: &mut Value) {
    strip_cache_scope(body);
    let Some(body) = body.as_object_mut() else {
        return;
    };
    body.remove("safeguards");
    if let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
        for message in messages.iter_mut().filter_map(Value::as_object_mut) {
            message.remove("output_config");
        }
    }
}

/// Remove `scope` from every `cache_control` marker in `value`.
fn strip_cache_scope(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(cc) = map.get_mut("cache_control").and_then(Value::as_object_mut) {
                cc.remove("scope");
            }
            map.values_mut().for_each(strip_cache_scope);
        }
        Value::Array(items) => items.iter_mut().for_each(strip_cache_scope),
        _ => {}
    }
}

/// Route Anthropic-format request to Copilot's native `/v1/messages` endpoint.
///
/// Copilot provides a native Anthropic-compatible Messages API at
/// `api.githubcopilot.com/v1/messages`. This handler authenticates as the
/// account's Copilot client and forwards the request verbatim.
pub(super) async fn copilot_messages(
    state: &Arc<AppState>,
    mut body: Value,
    stream: bool,
    beta: &str,
) -> Result<Response, ApiError> {
    strip_copilot_unsupported(&mut body);
    let messages = body
        .get("messages")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let conversation = Conversation::from_messages(messages);
    send_to_copilot(
        state,
        CopilotCall {
            path: "/v1/messages",
            body,
            stream,
            conversation,
            headers: &[
                ("anthropic-version", ANTHROPIC_VERSION),
                ("anthropic-beta", beta),
            ],
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn copilot_request_drops_fields_copilot_rejects_and_keeps_the_rest() {
        let mut body = json!({
            "model": "claude-fable-5-1",
            "safeguards": {"mode": "default"},
            "output_config": {"effort": "high"},
            "context_management": {"edits": []},
            "system": [{"type": "text", "text": "s", "cache_control": {"type": "ephemeral", "scope": "global"}}],
            "tools": [
                {"name": "t", "input_schema": {}, "eager_input_streaming": true},
                {"type": "web_search_20250305", "name": "web_search", "max_uses": 3}
            ],
            "messages": [
                {"role": "user", "content": "hi", "output_config": {"effort": "low"}},
                {"role": "assistant", "content": "hello"},
                {"role": "user", "content": [
                    {"type": "text", "text": "x", "cache_control": {"type": "ephemeral", "ttl": "1h", "scope": "global"}}
                ]}
            ]
        });
        strip_copilot_unsupported(&mut body);
        assert_eq!(
            body,
            json!({
                "model": "claude-fable-5-1",
                "output_config": {"effort": "high"},
                "context_management": {"edits": []},
                "system": [{"type": "text", "text": "s", "cache_control": {"type": "ephemeral"}}],
                "tools": [
                    {"name": "t", "input_schema": {}, "eager_input_streaming": true},
                    {"type": "web_search_20250305", "name": "web_search", "max_uses": 3}
                ],
                "messages": [
                    {"role": "user", "content": "hi"},
                    {"role": "assistant", "content": "hello"},
                    {"role": "user", "content": [
                        {"type": "text", "text": "x", "cache_control": {"type": "ephemeral", "ttl": "1h"}}
                    ]}
                ]
            })
        );
    }
}
