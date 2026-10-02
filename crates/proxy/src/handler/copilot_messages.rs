//! `POST /v1/messages` served by Copilot's own Anthropic-format Messages
//! endpoint: the fields Copilot rejects dropped, incidental requests
//! optionally on a small model, the rest done by [`send_to_copilot`].

use axum::response::Response;
use byokey_provider::Conversation;
use byokey_provider::claude::ANTHROPIC_VERSION;
use byokey_types::ProviderId;
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

/// Claude Code's compaction requests: they carry no tools yet must run on
/// the model the user chose, since their output replaces the conversation.
const COMPACTION_PROMPTS: &[&str] = &[
    "You are a helpful AI assistant tasked with summarizing conversations",
    "Your task is to create a detailed summary of the conversation so far",
];

/// Whether a request is one of the incidental calls Claude Code makes
/// around a turn (a title, a suggestion, a summary): no tools, and not a
/// compaction. On a per-request Copilot plan each one costs as much as a
/// real turn, so `providers.copilot.small_model` may serve them instead.
pub(super) fn is_incidental(body: &Value) -> bool {
    let has_tools = body
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(|t| !t.is_empty());
    if has_tools {
        return false;
    }
    let mut texts = Vec::new();
    match body.get("system") {
        Some(Value::String(s)) => texts.push(s.as_str()),
        Some(Value::Array(blocks)) => {
            texts.extend(blocks.iter().filter_map(|b| b["text"].as_str()));
        }
        _ => {}
    }
    if let Some(last) = body
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|m| m.last())
    {
        match last.get("content") {
            Some(Value::String(s)) => texts.push(s.as_str()),
            Some(Value::Array(blocks)) => {
                texts.extend(blocks.iter().filter_map(|b| b["text"].as_str()));
            }
            _ => {}
        }
    }
    !texts.iter().any(|t| {
        let t = t.trim_start();
        COMPACTION_PROMPTS.iter().any(|p| t.starts_with(p))
    })
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
    let small_model = state
        .config
        .load()
        .providers
        .get(&ProviderId::Copilot)
        .and_then(|c| c.small_model.clone());
    if let Some(small) = small_model
        && is_incidental(&body)
    {
        tracing::info!(small_model = %small, "serving a tool-less request with the small model");
        body["model"] = Value::String(small);
    }
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
            police_server_tools: true,
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

    #[test]
    fn incidental_requests_have_no_tools_and_are_not_compactions() {
        assert!(is_incidental(&json!({
            "system": "Generate a short title.",
            "messages": [{"role": "user", "content": "hi"}]
        })));
        assert!(is_incidental(&json!({
            "tools": [],
            "messages": [{"role": "user", "content": "hi"}]
        })));
        assert!(!is_incidental(&json!({
            "tools": [{"name": "Bash"}],
            "messages": [{"role": "user", "content": "hi"}]
        })));
        assert!(!is_incidental(&json!({
            "system": [{"type": "text", "text": "You are a helpful AI assistant tasked with summarizing conversations."}],
            "messages": [{"role": "user", "content": "go"}]
        })));
        assert!(!is_incidental(&json!({
            "messages": [{"role": "user", "content": [
                {"type": "text", "text": "Your task is to create a detailed summary of the conversation so far."}
            ]}]
        })));
    }
}
