//! Shared Copilot credentials and Anthropic request retry policy.

use axum::response::Response;
use byokey_provider::{Conversation, CopilotCredentials, CopilotIdentity, CopilotUpstream};
use byokey_types::{ByokError, ProviderId, Usage, UsageRecord};
use serde_json::Value;
use std::sync::Arc;

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

pub(super) struct CopilotCall<'a> {
    pub path: &'a str,
    pub body: Value,
    pub stream: bool,
    pub conversation: Conversation,
    pub headers: &'a [(&'a str, &'a str)],
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
        body,
        stream,
        conversation,
        headers,
    } = call;
    let copilot = copilot_upstream(state);
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
