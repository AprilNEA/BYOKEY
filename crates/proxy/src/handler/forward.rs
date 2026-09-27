//! Forwarding an upstream's answer to the client: status and headers kept,
//! usage recorded, and streams kept alive and properly terminated.

use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use byokey_types::{ByokError, Usage, traits::ByteStream};
use bytes::Bytes;
use futures_util::{Future, StreamExt as _, TryStreamExt as _};
use serde_json::Value;
use std::fmt::Write as _;
use std::time::Duration;

use crate::error::ApiError;
use crate::usage::{AnthropicUsage as _, Attribution};
use crate::util::stream::{
    deferred_stream, keep_alive, response_to_stream, tap_usage_stream, terminate_anthropic_stream,
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
    attribution: Attribution,
    reverse_remap_tools: bool,
) -> Result<Response, ApiError> {
    if !stream {
        let resp = pending
            .await
            .map_err(|e| ApiError::from(ByokError::from(e)))?;
        return forward_response(resp, false, attribution, reverse_remap_tools).await;
    }
    let mut pending = Box::pin(pending);
    match tokio::time::timeout(FIRST_BYTE_GRACE, &mut pending).await {
        Ok(Ok(resp)) => forward_response(resp, true, attribution, reverse_remap_tools).await,
        Ok(Err(e)) => Err(ApiError::from(ByokError::from(e))),
        Err(_elapsed) => {
            tracing::info!(
                grace_secs = FIRST_BYTE_GRACE.as_secs(),
                "upstream headers are late; streaming keepalives to the client"
            );
            Ok(stream_response(
                StatusCode::OK,
                &HeaderMap::new(),
                deferred_stream(pending),
                attribution,
                reverse_remap_tools,
            ))
        }
    }
}

/// Forward an upstream response back to the client, recording token usage.
pub(super) async fn forward_response(
    resp: reqwest::Response,
    stream: bool,
    attribution: Attribution,
    reverse_remap_tools: bool,
) -> Result<Response, ApiError> {
    let status = resp.status();
    if !status.is_success() {
        let err = ByokError::from_response(resp).await;
        attribution.failure();
        return Err(ApiError::from(err));
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
            attribution,
            reverse_remap_tools,
        ));
    }
    let mut json: Value = resp
        .json()
        .await
        .map_err(|e| ApiError::from(ByokError::from(e)))?;
    if reverse_remap_tools {
        byokey_provider::cloak::reverse_remap_tool_names_response(&mut json);
    }
    attribution.success(Usage::from_response(&json));
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
/// OAuth, usage recorded, keepalives while the upstream is silent, and a
/// guaranteed terminal event.
fn stream_response(
    status: StatusCode,
    upstream_headers: &HeaderMap,
    raw: ByteStream,
    attribution: Attribution,
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
    let tapped = tap_usage_stream(remapped, attribution);
    let alive = keep_alive(tapped, KEEPALIVE_INTERVAL, SILENCE_LIMIT);
    let mapped =
        terminate_anthropic_stream(alive).map_err(|e| std::io::Error::other(e.to_string()));
    let mut sse = sse_response(status, mapped);
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

        let attribution = Attribution::new(
            Arc::new(crate::UsageRecorder::new(None)),
            "m",
            ProviderId::Copilot,
            "a",
        );
        let Ok(response) = forward_response(upstream, false, attribution, false).await else {
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
