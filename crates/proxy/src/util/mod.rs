//! Shared proxy utilities — response builders, usage extraction, SSE stream tapping.

pub(crate) mod stream;

use axum::{
    body::Body,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use byokey_types::ByokError;
use serde_json::Value;

use crate::{UsageRecorder, error::ApiError};

/// Prefixes whose presence in a response header name indicates a third-party
/// API gateway fingerprint.  Names are case-insensitive.
static GATEWAY_HEADER_PREFIXES: &[&str] = &[
    "litellm-",
    "x-litellm-",
    "helicone-",
    "x-helicone-",
    "portkey-",
    "x-portkey-",
    "kong-",
    "x-kong-",
    "braintrust-",
    "x-braintrust-",
    "cf-",
    "x-cf-",
];

/// Removes response headers that reveal third-party API gateway fingerprints.
///
/// Checks each header name (lowercased) against [`GATEWAY_HEADER_PREFIXES`] and
/// drops any match in-place.  This is applied to upstream Claude/Copilot
/// responses **before** they are forwarded to the client so that downstream
/// tooling cannot detect intermediary gateway software.
pub(crate) fn strip_gateway_headers(headers: &mut HeaderMap) {
    let to_remove: Vec<_> = headers
        .keys()
        .filter(|name| {
            let lower = name.as_str().to_lowercase();
            GATEWAY_HEADER_PREFIXES
                .iter()
                .any(|prefix| lower.starts_with(prefix))
        })
        .cloned()
        .collect();
    for name in to_remove {
        headers.remove(&name);
    }
}

pub(crate) fn extract_usage(json: &Value, input_ptr: &str, output_ptr: &str) -> (u64, u64) {
    (
        json.pointer(input_ptr).and_then(Value::as_u64).unwrap_or(0),
        json.pointer(output_ptr)
            .and_then(Value::as_u64)
            .unwrap_or(0),
    )
}

pub(crate) fn bad_gateway(e: impl std::fmt::Display) -> Response {
    (
        StatusCode::BAD_GATEWAY,
        axum::Json(serde_json::json!({"error": {"message": e.to_string()}})),
    )
        .into_response()
}

pub(crate) fn sse_response(
    status: StatusCode,
    stream: impl futures_util::Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send + 'static,
) -> Response {
    Response::builder()
        .status(status)
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .header("x-accel-buffering", "no")
        .body(Body::from_stream(stream))
        .expect("valid response")
}

pub(crate) fn upstream_error(
    status: StatusCode,
    body: String,
    usage: &UsageRecorder,
    model: &str,
    provider: &str,
    account_id: &str,
) -> ApiError {
    tracing::warn!(
        %provider,
        %model,
        %account_id,
        upstream_status = status.as_u16(),
        body_len = body.len(),
        body_preview = %body.chars().take(512).collect::<String>(),
        "upstream_error: recording failure"
    );
    usage.record_failure_for(model, provider, account_id);
    ApiError::from(ByokError::Upstream {
        status: status.as_u16(),
        body,
        retry_after: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderName;
    use std::str::FromStr as _;

    #[test]
    fn strip_gateway_headers_drops_gateway_headers_keeps_safe() {
        let mut map = HeaderMap::new();
        map.insert(
            HeaderName::from_str("x-litellm-version").unwrap(),
            "1.0".parse().unwrap(),
        );
        map.insert(
            HeaderName::from_str("cf-ray").unwrap(),
            "abc123".parse().unwrap(),
        );
        map.insert(
            HeaderName::from_str("x-request-id").unwrap(),
            "req-1".parse().unwrap(),
        );
        map.insert(
            axum::http::header::CONTENT_TYPE,
            "application/json".parse().unwrap(),
        );

        strip_gateway_headers(&mut map);

        // Gateway headers gone.
        assert!(map.get("x-litellm-version").is_none());
        assert!(map.get("cf-ray").is_none());
        // Safe headers retained.
        assert!(map.get("x-request-id").is_some());
        assert!(map.get(axum::http::header::CONTENT_TYPE).is_some());
    }
}
