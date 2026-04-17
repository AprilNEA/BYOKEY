//! Shared proxy utilities — response builders, usage extraction, SSE stream tapping.

pub(crate) mod stream;

use axum::{
    body::Body,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use byokey_types::ByokError;
use serde_json::Value;

use crate::{UsageRecorder, error::ApiError};

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
