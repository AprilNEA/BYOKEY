//! Shared Copilot credentials and Anthropic request retry policy.

use axum::response::Response;
use byokey_provider::{Conversation, CopilotCredentials, CopilotIdentity, CopilotUpstream};
use byokey_types::{ByokError, ProviderId, Usage, UsageRecord};
use serde_json::Value;
use std::{collections::HashSet, sync::Arc};

use super::forward::{end_with, forward, forward_response};
use crate::{AppState, error::ApiError, exchange::Exchange};

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

pub(super) fn copilot_request(
    http: &reqwest::Client,
    path: &str,
    creds: &CopilotCredentials,
    identity: &CopilotIdentity,
    conversation: &Conversation,
    body: &Value,
) -> reqwest::RequestBuilder {
    let mut builder = http
        .post(format!("{}{path}", creds.endpoint))
        .bearer_auth(&creds.token);
    for (name, value) in identity.request_headers(creds, conversation) {
        builder = builder.header(name, value);
    }
    builder.json(body)
}

const POLICED_SERVER_TOOLS: &[&str] = &["web_search", "web_fetch"];

fn policed_server_tool(tool: &Value) -> Option<&'static str> {
    let ty = tool.get("type").and_then(Value::as_str)?;
    POLICED_SERVER_TOOLS.iter().copied().find(|kind| {
        ty.strip_prefix(kind)
            .is_some_and(|rest| rest.starts_with('_'))
    })
}

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

fn rejected_server_tool(err: &ByokError) -> Option<&'static str> {
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

pub(super) struct CopilotCall<'a> {
    pub path: &'a str,
    pub body: Value,
    pub stream: bool,
    pub conversation: Conversation,
    pub headers: &'a [(&'a str, &'a str)],
    pub police_server_tools: bool,
}

#[allow(
    clippy::too_many_lines,
    reason = "the retry loop owns each attempt and its exchange"
)]
pub(super) async fn send_to_copilot(
    state: &Arc<AppState>,
    call: CopilotCall<'_>,
) -> Result<Response, ApiError> {
    let CopilotCall {
        path,
        mut body,
        stream,
        conversation,
        headers,
        police_server_tools,
    } = call;
    let copilot = copilot_upstream(state);
    let has_server_tools = police_server_tools
        && body
            .get("tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| tools.iter().any(|t| policed_server_tool(t).is_some()));
    let accounts = state
        .auth
        .list_accounts(ProviderId::Copilot)
        .await
        .unwrap_or_default();
    let max_attempts = accounts.len().clamp(1, 3);
    let accept = if stream {
        "text/event-stream"
    } else {
        "application/json"
    };
    let model_name = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
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
                return Err(e.into());
            }
        };
        if has_server_tools && strip_server_tools(&mut body, &creds.rejected_tools()) {
            tracing::info!("leaving out server tools this Copilot account's policy rejects");
        }
        let exchange = Exchange::start(
            &state.usage,
            ProviderId::Copilot,
            model_name.clone(),
            creds.account_id.clone(),
        )
        .attempt(attempt)
        .initiator(conversation.initiator());
        let mut request = copilot_request(
            &state.http,
            path,
            &creds,
            copilot.identity(),
            &conversation,
            &body,
        )
        .header("accept", accept);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let pending = exchange.track(request.send());
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
        if matches!(err.error, ByokError::Upstream { status: 401, .. })
            && !token_refreshed
            && CopilotUpstream::forget_token(&creds)
        {
            token_refreshed = true;
            tracing::warn!(attempt, "copilot rejected its token, exchanging a new one");
            continue;
        }
        if has_server_tools
            && let Some(kind) = rejected_server_tool(&err.error)
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
    state.usage.record(UsageRecord {
        model: model_name,
        provider: ProviderId::Copilot.to_string(),
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
    fn server_tools_an_account_rejects_are_removed_and_recognised() {
        let rejected = HashSet::from(["web_search".to_owned()]);
        let mut body = json!({"tools": [
            {"type": "web_search_20250305", "name": "web_search"},
            {"type": "web_fetch_20250910", "name": "web_fetch"},
            {"type": "text_editor_20250728", "name": "str_replace_based_edit_tool"},
            {"name": "web_search_notes", "input_schema": {}}
        ]});
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
            ]
        );
        assert!(!strip_server_tools(&mut body, &rejected));
        let mut body = json!({"tools": [{"type": "web_search_20250305", "name": "web_search"}]});
        assert!(strip_server_tools(&mut body, &rejected));
        assert!(body.get("tools").is_none());
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
            None
        );
    }
}
