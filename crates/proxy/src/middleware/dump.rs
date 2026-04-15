//! Request/response dumping middleware for capturing real API traffic.
//!
//! When enabled via the `BYOKEY_DUMP` environment variable, all requests and
//! responses flowing through the proxy are written as JSON files to the
//! specified directory. Each file is named `{timestamp}_{method}_{path}.json`.
//!
//! Usage: `BYOKEY_DUMP=/tmp/byokey-dump cargo run -- serve`

use axum::{
    body::Body,
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Returns the dump directory from `BYOKEY_DUMP`, or `None` if unset.
pub fn dump_dir() -> Option<PathBuf> {
    std::env::var("BYOKEY_DUMP").ok().map(PathBuf::from)
}

/// Axum middleware that dumps every request and response to disk as JSON.
pub async fn dump_middleware(request: Request, next: Next) -> Response {
    let Some(dir) = dump_dir() else {
        return next.run(request).await;
    };

    // Ensure dump directory exists.
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(error = %e, "failed to create dump directory");
        return next.run(request).await;
    }

    let method = request.method().to_string();
    let uri = request.uri().to_string();
    let req_headers = format_headers(request.headers());

    // Buffer the request body so we can log it and reconstruct the request.
    let (parts, body) = request.into_parts();
    let req_bytes = match axum::body::to_bytes(body, 200 * 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::warn!(error = %e, "failed to read request body for dump");
            return (StatusCode::INTERNAL_SERVER_ERROR, "failed to read body").into_response();
        }
    };

    let req_body_json = bytes_to_json(&req_bytes);

    // Reconstruct the request with the buffered body.
    let request = Request::from_parts(parts, Body::from(req_bytes.clone()));

    // Forward to the actual handler.
    let response = next.run(request).await;

    // Capture response metadata.
    let resp_status = response.status().as_u16();
    let resp_headers = format_headers(response.headers());
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    // For streaming responses (SSE), don't buffer — just note it's a stream.
    let is_stream = content_type.contains("text/event-stream");

    if is_stream {
        // Dump request only; response body is streaming.
        let dump = json!({
            "request": {
                "method": method,
                "uri": uri,
                "headers": req_headers,
                "body": req_body_json,
            },
            "response": {
                "status": resp_status,
                "headers": resp_headers,
                "body": "__streaming__",
            }
        });

        let filename = make_filename(&method, &uri);
        let filepath = dir.join(filename);
        tokio::spawn(async move {
            write_dump(&filepath, &dump);
        });

        return response;
    }

    // Non-streaming: buffer the response body too.
    let (resp_parts, resp_body) = response.into_parts();
    let resp_bytes = match axum::body::to_bytes(resp_body, 200 * 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::warn!(error = %e, "failed to read response body for dump");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to read response body",
            )
                .into_response();
        }
    };

    let resp_body_json = bytes_to_json(&resp_bytes);

    // Build the dump document.
    let dump = json!({
        "request": {
            "method": method,
            "uri": uri,
            "headers": req_headers,
            "body": req_body_json,
        },
        "response": {
            "status": resp_status,
            "headers": resp_headers,
            "body": resp_body_json,
        }
    });

    // Write to file asynchronously (fire-and-forget).
    let filename = make_filename(&method, &uri);
    let filepath = dir.join(filename);
    tokio::spawn(async move {
        write_dump(&filepath, &dump);
    });

    // Reconstruct the response with the buffered body.
    Response::from_parts(resp_parts, Body::from(resp_bytes))
}

/// Format HTTP headers as a JSON object (multi-value headers become arrays).
fn format_headers(headers: &axum::http::HeaderMap) -> Value {
    let mut map = Map::new();
    for (name, value) in headers {
        let key = name.as_str().to_string();
        let val = value.to_str().unwrap_or("<binary>").to_string();
        map.entry(key)
            .and_modify(|existing| {
                if let Value::Array(arr) = existing {
                    arr.push(Value::String(val.clone()));
                } else {
                    let prev = existing.clone();
                    *existing = Value::Array(vec![prev, Value::String(val.clone())]);
                }
            })
            .or_insert_with(|| Value::String(val));
    }
    Value::Object(map)
}

/// Try to parse bytes as JSON; fall back to a string or base64 representation.
fn bytes_to_json(bytes: &Bytes) -> Value {
    if bytes.is_empty() {
        return Value::Null;
    }
    // Try JSON first.
    if let Ok(v) = serde_json::from_slice::<Value>(bytes) {
        return v;
    }
    // Try UTF-8 string.
    if let Ok(s) = std::str::from_utf8(bytes) {
        return Value::String(s.to_string());
    }
    // Binary data — record length only (avoid huge base64 blobs).
    json!({"__binary_length": bytes.len()})
}

/// Generate a filename like `1712345678901_POST_v1_chat_completions.json`.
fn make_filename(method: &str, uri: &str) -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let sanitized = uri
        .trim_start_matches('/')
        .replace('/', "_")
        .replace('?', "_q_")
        .chars()
        .take(80)
        .collect::<String>();
    format!("{ts}_{method}_{sanitized}.json")
}

/// Write the dump JSON to a file (blocking I/O, called from spawn).
fn write_dump(path: &Path, dump: &Value) {
    match serde_json::to_string_pretty(dump) {
        Ok(content) => {
            if let Err(e) = std::fs::write(path, content) {
                tracing::warn!(path = %path.display(), error = %e, "failed to write dump file");
            } else {
                tracing::debug!(path = %path.display(), "request/response dumped");
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to serialize dump");
        }
    }
}
