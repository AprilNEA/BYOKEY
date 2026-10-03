//! Responses API routing. Only the `ChatGPT` route receives the client's login.

mod catalog;
mod forward;
mod item_ids;
#[cfg(test)]
mod tests;

pub(crate) use catalog::models;

use axum::{
    Json,
    extract::{State, rejection::JsonRejection},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::Response,
};
use byokey_config::schema::responses::ResponsesUpstream;
use byokey_provider::{Conversation, CopilotUpstream};
use byokey_types::{ByokError, ProviderId};
use serde_json::Value;
use std::sync::Arc;

use super::copilot::{copilot_request, copilot_upstream};
use super::forward::end_with;
use crate::{ApiError, AppState, exchange::Exchange};

pub(crate) async fn responses(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>,
) -> Result<Response, ApiError> {
    if headers
        .get("content-encoding")
        .is_some_and(|v| v != "identity")
    {
        return Err(ByokError::InvalidRequest("compressed requests are not supported; set features.enable_request_compression = false in Codex".into()).into());
    }
    let Json(mut body) =
        body.map_err(|e| ApiError::from(ByokError::InvalidRequest(e.body_text())))?;
    let stream = body.get("stream").and_then(Value::as_bool).unwrap_or(false);
    super::record_model(&body).record("stream", stream);
    let requested = body
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| ByokError::InvalidRequest("model must be a string".into()))?
        .to_owned();
    let config = state.config.load_full();
    let (upstream, model) = config.responses.route(&requested)?;
    body["model"] = Value::String(model.to_owned());
    match upstream {
        "chatgpt" => {
            require_chatgpt_auth(&headers)?;
            let account = headers
                .get("chatgpt-account-id")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("client");
            let exchange = Exchange::start(&state.usage, "chatgpt", model, account);
            let request = state
                .http
                .post(endpoint(&config.responses.chatgpt_base_url, "responses"))
                .headers(chatgpt_headers(headers))
                .json(&body);
            let response = match send(request, &exchange).await {
                Ok(response) => response,
                Err(error) => return Err(end_with(exchange, error)),
            };
            // Authentication failures return to the client so its own refresh flow runs.
            forward::response(response, exchange, stream, forward::StreamMode::Passthrough).await
        }
        "copilot" => {
            if config
                .providers
                .get(&ProviderId::Copilot)
                .is_some_and(|c| !c.enabled)
            {
                return Err(ByokError::UnsupportedProvider("copilot is disabled".into()).into());
            }
            copilot_responses(&state, &headers, body, stream).await
        }
        name => {
            let upstream = &config.responses.upstreams[name];
            if let Some(tier) = &upstream.service_tier {
                body["service_tier"] = Value::String(tier.clone());
            }
            let request = state
                .http
                .post(endpoint(&upstream.base_url, "responses"))
                .headers(custom_headers(upstream, &headers)?)
                .json(&body);
            let exchange = Exchange::start(&state.usage, name, model, "configured");
            let response = match send(request, &exchange).await {
                Ok(response) => response,
                Err(error) => return Err(end_with(exchange, error)),
            };
            forward::response(response, exchange, stream, forward::StreamMode::Passthrough).await
        }
    }
}

async fn copilot_responses(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    body: Value,
    stream: bool,
) -> Result<Response, ApiError> {
    let upstream = copilot_upstream(state);
    let model = body["model"]
        .as_str()
        .expect("model was validated at ingress");
    if !upstream
        .models()
        .await?
        .iter()
        .any(|m| m.id == model && m.responses)
    {
        return Err(ByokError::UnsupportedModel(format!(
            "{model} does not support Copilot /responses"
        ))
        .into());
    }
    let conversation = Conversation::from_responses(&body["input"]);
    let mut refreshed = false;
    loop {
        let creds = upstream.credentials().await?;
        let exchange = Exchange::start(&state.usage, ProviderId::Copilot, model, &creds.account_id)
            .initiator(conversation.initiator());
        let request = copilot_request(
            &state.http,
            "/responses",
            &creds,
            upstream.identity(),
            &conversation,
            &body,
        )
        .headers(protocol_headers(headers))
        .header("accept", "text/event-stream, application/json");
        let response = match send(request, &exchange).await {
            Ok(response) => response,
            Err(error) => return Err(end_with(exchange, error)),
        };
        if response.status() == StatusCode::UNAUTHORIZED
            && !refreshed
            && CopilotUpstream::forget_token(&creds)
        {
            refreshed = true;
            exchange.fail(&ByokError::from_response(response).await);
            continue;
        }
        return forward::response(response, exchange, stream, forward::StreamMode::Copilot).await;
    }
}

async fn send(
    request: reqwest::RequestBuilder,
    exchange: &Exchange,
) -> Result<reqwest::Response, ByokError> {
    tokio::time::timeout(
        std::time::Duration::from_secs(120),
        exchange.track(request.send()),
    )
    .await
    .map_err(|_| ByokError::Http("upstream response headers timed out after 120s".into()))?
    .map_err(|e| ByokError::Http(e.without_url().to_string()))
}

fn endpoint(base: &str, path: &str) -> String {
    format!("{}/{path}", base.trim_end_matches('/'))
}

fn require_chatgpt_auth(headers: &HeaderMap) -> Result<(), ByokError> {
    if !headers.contains_key("authorization") {
        return Err(ByokError::Auth(
            "sign in to ChatGPT and set requires_openai_auth = true in Codex".into(),
        ));
    }
    Ok(())
}

fn chatgpt_headers(mut headers: HeaderMap) -> HeaderMap {
    strip_hop_headers(&mut headers);
    headers.remove("cookie");
    headers
}

fn custom_headers(
    upstream: &ResponsesUpstream,
    incoming: &HeaderMap,
) -> Result<HeaderMap, ByokError> {
    let mut headers = protocol_headers(incoming);
    if let Some(key) = &upstream.api_key {
        let mut value = HeaderValue::from_str(&format!("Bearer {}", key.resolve()?))
            .map_err(|_| ByokError::Config("invalid upstream API key header".into()))?;
        value.set_sensitive(true);
        headers.insert("authorization", value);
    }
    for (name, source) in &upstream.headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| ByokError::Config(format!("invalid upstream header name: {name}")))?;
        let mut value = HeaderValue::from_str(&source.resolve()?)
            .map_err(|_| ByokError::Config(format!("invalid value for upstream header {name}")))?;
        value.set_sensitive(true);
        headers.insert(name, value);
    }
    strip_hop_headers(&mut headers);
    Ok(headers)
}

fn protocol_headers(incoming: &HeaderMap) -> HeaderMap {
    let mut headers = HeaderMap::new();
    // Other upstreams inherit protocol state, never the client's ChatGPT credentials.
    for name in [
        "accept",
        "session-id",
        "thread-id",
        "x-client-request-id",
        "x-codex-turn-state",
        "x-codex-beta-features",
        "x-openai-internal-codex-responses-lite",
    ] {
        if let Some(value) = incoming.get(name) {
            headers.insert(HeaderName::from_static(name), value.clone());
        }
    }
    headers
}

/// Remove connection-specific headers, including the fields named by Connection.
fn strip_hop_headers(headers: &mut HeaderMap) {
    let named: Vec<_> = headers
        .get_all("connection")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(',').map(str::trim))
        .map(str::to_owned)
        .collect();
    for name in named {
        headers.remove(name);
    }
    for name in [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
        "host",
        "content-length",
    ] {
        headers.remove(name);
    }
}
