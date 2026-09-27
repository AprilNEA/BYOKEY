//! `POST /v1/messages` served by Copilot's own Anthropic-format Messages
//! endpoint, with per-account token refresh, server-tool policy learning
//! and account rotation.

use axum::response::Response;
use byokey_provider::claude::ANTHROPIC_VERSION;
use byokey_provider::{Conversation, CopilotCredentials, CopilotIdentity, CopilotUpstream};
use byokey_types::{ByokError, ProviderId, Usage, UsageRecord};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;

use super::forward::{end_with, forward, forward_response};
use crate::exchange::Exchange;
use crate::{AppState, error::ApiError};

/// The Copilot accounts configured for this server.
pub(super) fn copilot_upstream(state: &AppState) -> CopilotUpstream {
    let config = state
        .config
        .load()
        .providers
        .get(&ProviderId::Copilot)
        .cloned()
        .unwrap_or_default();
    CopilotUpstream::builder()
        .http(state.http.clone())
        .auth(state.auth.clone())
        .maybe_api_key(config.api_key)
        .maybe_base_url(config.base_url)
        .identity(CopilotIdentity::clone(&state.copilot_identity.load()))
        .build()
}

/// A POST of `body` to Copilot's Anthropic-format `path` as `creds`' account.
pub(super) fn copilot_request(
    http: &reqwest::Client,
    path: &str,
    creds: &CopilotCredentials,
    beta: &str,
    identity: &CopilotIdentity,
    conversation: &Conversation,
    body: &Value,
) -> reqwest::RequestBuilder {
    let mut builder = http
        .post(format!("{}{path}", creds.endpoint))
        .header("authorization", format!("Bearer {}", creds.token))
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("anthropic-beta", beta)
        .header("content-type", "application/json");
    for (name, value) in identity.request_headers(creds, conversation) {
        builder = builder.header(name, value);
    }
    builder.json(body)
}

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

/// Anthropic server tools a Copilot organisation policy can turn off:
/// `web_search_*` and `web_fetch_*`. Copilot then rejects the whole request
/// with 400 and no flag announces it up front, so the account's rejection
/// is learned from the error (see [`rejected_server_tool`]) and remembered
/// on its [`CopilotCredentials`].
const POLICED_SERVER_TOOLS: &[&str] = &["web_search", "web_fetch"];

/// The server tool kind (`web_search`, `web_fetch`) of `tool`, if it is one
/// Copilot policy can reject.
fn policed_server_tool(tool: &Value) -> Option<&'static str> {
    let ty = tool.get("type").and_then(Value::as_str)?;
    POLICED_SERVER_TOOLS.iter().copied().find(|kind| {
        ty.strip_prefix(kind)
            .is_some_and(|rest| rest.starts_with('_'))
    })
}

/// Remove the server tools in `rejected` from `body.tools`. Returns whether
/// anything was removed.
pub(super) fn strip_server_tools(body: &mut Value, rejected: &HashSet<String>) -> bool {
    if rejected.is_empty() {
        return false;
    }
    let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) else {
        return false;
    };
    let before = tools.len();
    tools.retain(|tool| !policed_server_tool(tool).is_some_and(|kind| rejected.contains(kind)));
    let removed = tools.len() < before;
    if tools.is_empty()
        && let Some(body) = body.as_object_mut()
    {
        body.remove("tools");
    }
    removed
}

/// The server tool a Copilot 400 rejected, if that is what it says: "The
/// use of the web search tool is not supported" (`unsupported_value`) or
/// "rejected tool(s): `web_fetch`" (`invalid_request_body`).
pub(super) fn rejected_server_tool(err: &ByokError) -> Option<&'static str> {
    let ByokError::Upstream {
        status: 400, body, ..
    } = err
    else {
        return None;
    };
    let message = serde_json::from_str::<Value>(body)
        .ok()?
        .pointer("/error/message")?
        .as_str()?
        .to_ascii_lowercase();
    POLICED_SERVER_TOOLS
        .iter()
        .copied()
        .find(|kind| message.contains(&kind.replace('_', " ")) || message.contains(kind))
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
///
/// A Copilot API token that is rejected before its stated expiry is
/// exchanged again once. With multiple Copilot accounts, transient failures
/// are retried with quota-aware rotation.
#[allow(clippy::too_many_lines)]
pub(super) async fn copilot_messages(
    state: &Arc<AppState>,
    mut body: Value,
    stream: bool,
    beta: &str,
) -> Result<Response, ApiError> {
    strip_copilot_unsupported(&mut body);
    let copilot = copilot_upstream(state);
    let has_server_tools = body
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(|tools| tools.iter().any(|t| policed_server_tool(t).is_some()));
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

    let accounts = state
        .auth
        .list_accounts(ProviderId::Copilot)
        .await
        .unwrap_or_default();
    let max_attempts = if accounts.len() > 1 {
        accounts.len().min(3)
    } else {
        1
    };

    let accept = if stream {
        "text/event-stream"
    } else {
        "application/json"
    };
    let messages = body
        .get("messages")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let conversation = Conversation::from_messages(messages);
    let model_name = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();

    let mut last_err = None;
    let mut attempt = 0;
    let mut token_refreshed = false;
    while attempt < max_attempts {
        let creds = match copilot.credentials().await {
            Ok(c) => c,
            Err(e) => {
                if max_attempts > 1 {
                    tracing::warn!(attempt, error = %e, "copilot token failed, trying next account");
                    CopilotUpstream::invalidate_current_account();
                    last_err = Some(ApiError::from(e));
                    attempt += 1;
                    continue;
                }
                return Err(ApiError::from(e));
            }
        };
        // What this account's organisation policy rejected before is left
        // out up front; a rejection learned now is retried without the tool.
        if has_server_tools && strip_server_tools(&mut body, &creds.rejected_tools()) {
            tracing::info!("leaving out server tools this Copilot account's policy rejects");
        }
        tracing::debug!(
            endpoint = %creds.endpoint,
            ?conversation,
            "routing Anthropic messages through Copilot"
        );

        let exchange = Exchange::start(
            &state.usage,
            ProviderId::Copilot,
            model_name.clone(),
            creds.account_id.clone(),
        )
        .attempt(attempt)
        .initiator(conversation.initiator());
        let pending = exchange.track(
            copilot_request(
                &state.http,
                "/v1/messages",
                &creds,
                beta,
                copilot.identity(),
                &conversation,
                &body,
            )
            .header("accept", accept)
            .send(),
        );
        // Only the last attempt may hand the client a response before the
        // upstream answered: an earlier one still needs the status to decide
        // whether to try the next account.
        let last_attempt = attempt + 1 >= max_attempts;
        let outcome = if last_attempt {
            forward(pending, stream, exchange, false).await
        } else {
            match pending.await {
                Ok(resp) => forward_response(resp, stream, exchange, false).await,
                Err(e) => Err(end_with(exchange, e.into())),
            }
        };
        let err = match outcome {
            Ok(response) => return Ok(response),
            Err(err) => err,
        };
        // The cached token may have been revoked ahead of its expiry.
        if matches!(err.error, ByokError::Upstream { status: 401, .. })
            && !token_refreshed
            && CopilotUpstream::forget_token(&creds)
        {
            token_refreshed = true;
            tracing::warn!(attempt, "copilot rejected its token, exchanging a new one");
            continue;
        }
        // The account's policy rejects a server tool: remember it and retry
        // the same account without that tool. Claude Code's WebSearch or
        // WebFetch then just does nothing on this account.
        if let Some(kind) = rejected_server_tool(&err.error)
            && creds.reject_tool(kind)
            && strip_server_tools(&mut body, &HashSet::from([kind.to_owned()]))
        {
            tracing::warn!(
                tool = kind,
                "this Copilot account's policy rejects a server tool; retrying without it"
            );
            continue;
        }
        if !err.error.is_retryable() || last_attempt {
            return Err(err);
        }
        tracing::warn!(attempt, error = %err.error, "copilot messages failed, trying next account");
        CopilotUpstream::invalidate_current_account();
        last_err = Some(err);
        attempt += 1;
    }

    tracing::warn!(
        attempts = max_attempts,
        "all copilot accounts exhausted for messages request"
    );
    state.usage.record(UsageRecord {
        model: model_name,
        provider: ProviderId::Copilot,
        account_id: byokey_types::DEFAULT_ACCOUNT.to_owned(),
        usage: Usage::default(),
        success: false,
    });
    Err(last_err
        .unwrap_or_else(|| ApiError::from(ByokError::Auth("no copilot accounts available".into()))))
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
    fn server_tools_an_account_rejects_are_removed_and_recognised() {
        let rejected = HashSet::from(["web_search".to_owned()]);
        let mut body = json!({
            "tools": [
                {"type": "web_search_20250305", "name": "web_search"},
                {"type": "web_fetch_20250910", "name": "web_fetch"},
                {"type": "text_editor_20250728", "name": "str_replace_based_edit_tool"},
                {"name": "web_search_notes", "input_schema": {}}
            ]
        });
        assert!(strip_server_tools(&mut body, &rejected));
        let kept: Vec<&str> = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            kept,
            [
                "web_fetch",
                "str_replace_based_edit_tool",
                "web_search_notes"
            ],
            "only the rejected kind goes; a custom tool that mentions the name stays"
        );
        assert!(
            !strip_server_tools(&mut body, &rejected),
            "nothing left to remove"
        );

        let mut body = json!({"tools": [{"type": "web_search_20250305", "name": "web_search"}]});
        assert!(strip_server_tools(&mut body, &rejected));
        assert!(body.get("tools").is_none(), "an emptied list is dropped");
        assert!(!strip_server_tools(
            &mut json!({"tools": []}),
            &HashSet::new()
        ));

        let upstream = |body: &str| ByokError::Upstream {
            status: 400,
            body: body.into(),
            retry_after: None,
        };
        assert_eq!(
            rejected_server_tool(&upstream(
                r#"{"error":{"message":"The use of the web search tool is not supported.","code":"unsupported_value"}}"#
            )),
            Some("web_search")
        );
        assert_eq!(
            rejected_server_tool(&upstream(
                r#"{"error":{"message":"rejected tool(s): web_fetch","code":"invalid_request_body"}}"#
            )),
            Some("web_fetch")
        );
        assert_eq!(
            rejected_server_tool(&upstream(
                r#"{"error":{"message":"The requested model is not supported.","code":"model_not_supported"}}"#
            )),
            None
        );
        assert_eq!(
            rejected_server_tool(&ByokError::Upstream {
                status: 403,
                body: "web search".into(),
                retry_after: None
            }),
            None,
            "only a 400 is a policy rejection"
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
