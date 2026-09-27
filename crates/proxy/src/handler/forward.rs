//! Forwarding an upstream's answer to the client: status and headers kept,
//! the exchange accounted for, and streams kept alive and properly ended.

use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use byokey_types::{ByokError, traits::ByteStream};
use bytes::Bytes;
use futures_util::{Future, StreamExt as _, TryStreamExt as _};
use serde_json::Value;
use std::fmt::Write as _;
use std::time::Duration;

use crate::error::ApiError;
use crate::exchange::Exchange;
use crate::util::stream::{
    deferred_stream, deliver_anthropic_stream, keep_alive, response_to_stream,
};
use crate::util::{sse_response, strip_gateway_headers};

/// How long a streaming request waits for the upstream's headers before the
/// client gets a response of its own, with keepalives, so that Claude Code
/// (which shows a retry banner after 20 s without a byte) keeps waiting.
/// Errors the upstream returns within this window keep their HTTP status.
const FIRST_BYTE_GRACE: Duration = Duration::from_secs(15);
/// A keepalive comment is written after this much upstream silence.
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(10);
/// This much upstream silence in a row ends the stream with an error. A
/// live upstream sends `ping` events every few seconds even while the model
/// thinks, so a longer silence means the connection is gone.
const SILENCE_LIMIT: Duration = Duration::from_secs(120);

/// `err` ends `exchange`, which logs it; the client then gets it without a
/// second log line.
pub(super) fn end_with(exchange: Exchange, err: ByokError) -> ApiError {
    exchange.fail(&err);
    ApiError::new(err).logged()
}

/// Forward the response to `pending` back to the client.
///
/// A streaming client is answered as soon as [`FIRST_BYTE_GRACE`] passes
/// without upstream headers: it gets a `200` and keepalive comments until
/// the upstream's body arrives, or its error as an in-stream `error` event.
/// Upstream errors that arrive within the grace period, and every
/// non-streaming response, keep their HTTP status.
pub(super) async fn forward(
    pending: impl Future<Output = reqwest::Result<reqwest::Response>> + Send + 'static,
    stream: bool,
    exchange: Exchange,
    reverse_remap_tools: bool,
) -> Result<Response, ApiError> {
    if !stream {
        return match pending.await {
            Ok(resp) => forward_response(resp, false, exchange, reverse_remap_tools).await,
            Err(e) => Err(end_with(exchange, e.into())),
        };
    }
    let mut pending = Box::pin(pending);
    match tokio::time::timeout(FIRST_BYTE_GRACE, &mut pending).await {
        Ok(Ok(resp)) => forward_response(resp, true, exchange, reverse_remap_tools).await,
        Ok(Err(e)) => Err(end_with(exchange, e.into())),
        Err(_elapsed) => {
            exchange.span().in_scope(|| {
                tracing::info!(
                    grace_secs = FIRST_BYTE_GRACE.as_secs(),
                    "upstream headers are late; streaming keepalives to the client"
                );
            });
            Ok(stream_response(
                StatusCode::OK,
                &HeaderMap::new(),
                deferred_stream(pending),
                exchange,
                reverse_remap_tools,
            ))
        }
    }
}

/// Forward an upstream response back to the client; its end ends `exchange`.
pub(super) async fn forward_response(
    resp: reqwest::Response,
    stream: bool,
    exchange: Exchange,
    reverse_remap_tools: bool,
) -> Result<Response, ApiError> {
    let status = resp.status();
    if !status.is_success() {
        return Err(end_with(exchange, ByokError::from_response(resp).await));
    }

    let upstream_status = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::OK);

    // Collect upstream response headers and strip gateway fingerprints before
    // forwarding anything to the client.
    let mut upstream_headers = axum::http::HeaderMap::new();
    for (name, value) in resp.headers() {
        if let Ok(name) = axum::http::HeaderName::from_bytes(name.as_str().as_bytes())
            && let Ok(value) = axum::http::HeaderValue::from_bytes(value.as_bytes())
        {
            upstream_headers.insert(name, value);
        }
    }
    strip_gateway_headers(&mut upstream_headers);
    // Both branches re-encode the body, so the upstream framing no longer
    // describes it. A stale content-length makes hyper panic mid-response.
    for framing in [
        axum::http::header::CONTENT_LENGTH,
        axum::http::header::TRANSFER_ENCODING,
        axum::http::header::CONTENT_ENCODING,
    ] {
        upstream_headers.remove(framing);
    }

    if stream {
        return Ok(stream_response(
            upstream_status,
            &upstream_headers,
            response_to_stream(resp),
            exchange,
            reverse_remap_tools,
        ));
    }
    let mut json: Value = match resp.json().await {
        Ok(json) => json,
        Err(e) => return Err(end_with(exchange, e.into())),
    };
    if reverse_remap_tools {
        byokey_provider::cloak::reverse_remap_tool_names_response(&mut json);
    }
    exchange.complete_with(&json);
    let mut response = (upstream_status, axum::Json(json)).into_response();
    // Merge upstream headers (gateway-stripped) into the JSON response.
    for (name, value) in &upstream_headers {
        response
            .headers_mut()
            .entry(name)
            .or_insert_with(|| value.clone());
    }
    Ok(response)
}

/// The SSE response for an upstream byte stream: tool names mapped back for
/// OAuth, keepalives while the upstream is silent, a guaranteed terminal
/// event, and the exchange accounted for.
fn stream_response(
    status: StatusCode,
    upstream_headers: &HeaderMap,
    raw: ByteStream,
    exchange: Exchange,
    reverse_remap_tools: bool,
) -> Response {
    let remapped: ByteStream = if reverse_remap_tools {
        Box::pin(raw.map(move |chunk| {
            let bytes = chunk?;
            let text = String::from_utf8_lossy(&bytes);
            let mut output = String::new();
            for line in text.split_inclusive('\n') {
                if let Some(data) = line.trim().strip_prefix("data: ")
                    && let Ok(mut ev) = serde_json::from_str::<Value>(data)
                {
                    byokey_provider::cloak::reverse_remap_tool_name_sse(&mut ev);
                    let _ = writeln!(output, "data: {ev}");
                    continue;
                }
                output.push_str(line);
            }
            Ok(Bytes::from(output))
        }))
    } else {
        raw
    };
    let alive = keep_alive(remapped, KEEPALIVE_INTERVAL, SILENCE_LIMIT);
    let delivered =
        deliver_anthropic_stream(alive, exchange).map_err(|e| std::io::Error::other(e.to_string()));
    let mut sse = sse_response(status, delivered);
    // Merge upstream headers (gateway-stripped) into the SSE response,
    // without overwriting the SSE-specific ones sse_response set.
    for (name, value) in upstream_headers {
        sse.headers_mut()
            .entry(name)
            .or_insert_with(|| value.clone());
    }
    sse
}

#[cfg(test)]
mod tests {
    use super::*;
    use byokey_types::ProviderId;
    use std::sync::Arc;

    // ── forward_response: re-encoded bodies ────────────────────────────

    #[tokio::test]
    async fn non_stream_response_does_not_forward_upstream_content_length() {
        // The body is parsed and re-serialized, so its length can change; the
        // upstream content-length then disagrees with it and hyper panics.
        let upstream_body = r#"{"id": "msg_1", "type": "message", "content": []}"#;
        let upstream: reqwest::Response = axum::http::Response::builder()
            .header("content-type", "application/json")
            .header("content-length", upstream_body.len())
            .header("x-upstream-marker", "kept")
            .body(upstream_body)
            .unwrap()
            .into();

        let exchange = Exchange::start(
            &Arc::new(crate::UsageRecorder::new(None)),
            ProviderId::Copilot,
            "m",
            "a",
        );
        let Ok(response) = forward_response(upstream, false, exchange, false).await else {
            panic!("a 200 upstream response must forward");
        };

        assert_eq!(response.headers()["x-upstream-marker"], "kept");
        let declared = response
            .headers()
            .get(axum::http::header::CONTENT_LENGTH)
            .cloned();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        if let Some(declared) = declared {
            assert_eq!(declared.to_str().unwrap(), body.len().to_string());
        }
        assert_ne!(
            body.len(),
            upstream_body.len(),
            "fixture must change length"
        );
    }
}
