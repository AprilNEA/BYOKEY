//! Byte-preserving Responses forwarding and stream completion accounting.

use axum::{body::Body, response::Response};
use byokey_types::{ByokError, traits::ByteStream};
use bytes::Bytes;
use futures_util::{StreamExt as _, stream};
use serde_json::{Value, json};
use std::time::Duration;

use super::{super::forward::end_with, strip_hop_headers};
use crate::{
    ApiError,
    exchange::Exchange,
    util::stream::{keep_alive, response_to_stream},
};

pub(super) async fn response(
    upstream: reqwest::Response,
    exchange: Exchange,
) -> Result<Response, ApiError> {
    let status = upstream.status();
    let mut headers = upstream.headers().clone();
    strip_hop_headers(&mut headers);
    let streaming = status.is_success()
        && headers
            .get("content-type")
            .is_some_and(|v| v.as_bytes().starts_with(b"text/event-stream"));
    let body = if streaming {
        headers.insert("cache-control", "no-cache".parse().unwrap());
        headers.insert("x-accel-buffering", "no".parse().unwrap());
        Body::from_stream(deliver(
            keep_alive(
                response_to_stream(upstream),
                Duration::from_secs(10),
                Duration::from_secs(120),
            ),
            exchange,
        ))
    } else {
        let bytes = match tokio::time::timeout(Duration::from_secs(120), upstream.bytes()).await {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(e)) => return Err(end_with(exchange, e.into())),
            Err(_) => {
                return Err(end_with(
                    exchange,
                    ByokError::Http("upstream response body timed out after 120s".into()),
                ));
            }
        };
        if status.is_success() {
            let value: Value = match serde_json::from_slice(&bytes) {
                Ok(value) => value,
                Err(e) => {
                    return Err(end_with(
                        exchange,
                        ByokError::Http(format!("invalid Responses JSON: {e}")),
                    ));
                }
            };
            match value.get("status").and_then(Value::as_str) {
                Some("failed" | "incomplete") => exchange.fail_with_event(&value),
                _ => exchange.complete_with(&value),
            }
        } else {
            exchange.fail(&ByokError::Upstream {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&bytes).into_owned(),
                retry_after: None,
            });
        }
        Body::from(bytes)
    };
    let mut response = Response::new(body);
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    Ok(response)
}

struct StreamState {
    inner: ByteStream,
    exchange: Option<Exchange>,
    line: Vec<u8>,
    data: Vec<u8>,
    at_boundary: bool,
    closed: bool,
}
impl StreamState {
    fn line(&mut self, line: &[u8]) {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        self.at_boundary = line.is_empty();
        if line.is_empty() {
            let data = std::mem::take(&mut self.data);
            let Ok(event) = serde_json::from_slice::<Value>(&data) else {
                return;
            };
            match event.get("type").and_then(Value::as_str) {
                Some("response.completed") => {
                    if let Some(exchange) = self.exchange.take() {
                        exchange.complete_with(&event["response"]);
                    }
                }
                Some("response.failed" | "response.incomplete") => {
                    if let Some(exchange) = self.exchange.take() {
                        exchange.fail_with_event(&event["response"]);
                    }
                }
                Some("error") => {
                    if let Some(exchange) = self.exchange.take() {
                        // Responses errors may carry code/message at the top level.
                        exchange.fail_with_event(
                            &json!({"error": event.get("error").unwrap_or(&event)}),
                        );
                    }
                }
                _ => {}
            }
        } else if let Some(data) = line.strip_prefix(b"data:") {
            self.data
                .extend_from_slice(data.strip_prefix(b" ").unwrap_or(data));
            self.data.push(b'\n');
        }
    }

    fn receive(&mut self, bytes: &[u8]) {
        for part in bytes.split_inclusive(|b| *b == b'\n') {
            if self.exchange.is_none() {
                break;
            }
            self.line.extend_from_slice(part);
            if self.line.last() == Some(&b'\n') {
                self.line.pop();
                let line = std::mem::take(&mut self.line);
                self.line(&line);
            }
        }
    }

    fn fail(&mut self, error: &ByokError) -> Bytes {
        if let Some(exchange) = self.exchange.take() {
            exchange.fail(error);
        }
        self.closed = true;
        let event = json!({"type":"error", "code":"upstream_error", "message":error.to_string(), "param":null});
        Bytes::from(format!("\n\nevent: error\ndata: {event}\n\n"))
    }
}

/// Inspect SSE frames without changing their bytes, even across UTF-8/chunk boundaries.
fn deliver(inner: ByteStream, exchange: Exchange) -> ByteStream {
    Box::pin(stream::unfold(
        StreamState {
            inner,
            exchange: Some(exchange),
            line: Vec::new(),
            data: Vec::new(),
            at_boundary: true,
            closed: false,
        },
        |mut state| async move {
            if state.closed {
                return None;
            }
            loop {
                match state.inner.next().await {
                    Some(Ok(bytes)) => {
                        // A comment inserted inside an unfinished frame would corrupt its payload.
                        if bytes.as_ref() == b": keepalive\n\n"
                            && (!state.at_boundary || !state.line.is_empty())
                        {
                            continue;
                        }
                        if let Some(exchange) = &mut state.exchange {
                            if bytes.as_ref() == b": keepalive\n\n" {
                                exchange.kept_alive();
                            } else {
                                exchange.received();
                            }
                        }
                        state.receive(&bytes);
                        if state.exchange.is_none() {
                            state.closed = true;
                            state.inner = Box::pin(stream::empty());
                        }
                        return Some((Ok(bytes), state));
                    }
                    Some(Err(error)) => {
                        let bytes = state.fail(&error);
                        return Some((Ok(bytes), state));
                    }
                    None => {
                        let line = std::mem::take(&mut state.line);
                        if !line.is_empty() {
                            state.line(&line);
                        }
                        state.line(b"");
                        state.exchange.as_ref()?;
                        let bytes = state.fail(&ByokError::Http(
                            "upstream closed before a terminal Responses event".into(),
                        ));
                        return Some((Ok(bytes), state));
                    }
                }
            }
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{UsageRecorder, test_logs::Logs};
    use std::sync::Arc;
    use tokio_stream::wrappers::UnboundedReceiverStream;

    fn exchange(usage: &Arc<UsageRecorder>) -> Exchange {
        Exchange::start(usage, "company", "gpt-example", "configured")
    }

    async fn collect(stream: ByteStream) -> Vec<u8> {
        stream
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .flat_map(Result::unwrap)
            .collect()
    }

    #[tokio::test]
    async fn utf8_and_multiline_frames_survive_chunk_boundaries() {
        let usage = Arc::new(UsageRecorder::new(None));
        let body = "event: response.completed\r\ndata: {\"type\":\"response.completed\",\r\ndata: \"response\":{\"status\":\"completed\",\"output\":[\"你好\"],\"usage\":{\"input_tokens\":19,\"output_tokens\":3}}}\r\n\r\n";
        let bytes = body
            .as_bytes()
            .iter()
            .map(|b| Ok(Bytes::copy_from_slice(&[*b])))
            .collect::<Vec<_>>();

        let out = collect(deliver(Box::pin(stream::iter(bytes)), exchange(&usage))).await;

        assert_eq!(out, body.as_bytes());
        let snapshot = usage.snapshot();
        assert_eq!(
            (
                snapshot.success_requests,
                snapshot.failure_requests,
                snapshot.input_tokens,
                snapshot.output_tokens
            ),
            (1, 0, 19, 3)
        );
    }

    #[tokio::test]
    async fn failed_and_incomplete_events_are_preserved_but_not_counted_as_successes() {
        for body in [
            "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"server_error\",\"message\":\"failed\"}}}\n\n",
            "data: {\"type\":\"response.incomplete\",\"response\":{\"status\":\"incomplete\",\"incomplete_details\":{\"reason\":\"max_output_tokens\"}}}\n\n",
            "data: {\"type\":\"error\",\"code\":\"rate_limit_exceeded\",\"message\":\"slow down\"}\n\n",
        ] {
            let usage = Arc::new(UsageRecorder::new(None));
            let input = Box::pin(stream::iter([Ok(Bytes::from_static(body.as_bytes()))]));
            let out = collect(deliver(input, exchange(&usage))).await;
            assert_eq!(out, body.as_bytes());
            let snapshot = usage.snapshot();
            assert_eq!(
                (snapshot.success_requests, snapshot.failure_requests),
                (0, 1)
            );
        }
    }

    #[tokio::test]
    async fn premature_eof_appends_a_terminal_error() {
        let usage = Arc::new(UsageRecorder::new(None));
        let body = "data: {\"type\":\"response.created\"}\n\n";
        let input = Box::pin(stream::iter([Ok(Bytes::from_static(body.as_bytes()))]));
        let out = String::from_utf8(collect(deliver(input, exchange(&usage))).await).unwrap();
        assert!(out.starts_with(body));
        let error: Value = serde_json::from_str(out.rsplit_once("data: ").unwrap().1).unwrap();
        assert_eq!(error["type"], "error");
        assert_eq!(error["code"], "upstream_error");
        assert_eq!(usage.snapshot().failure_requests, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn silence_times_out_without_inserting_comments_inside_partial_frames() {
        let usage = Arc::new(UsageRecorder::new(None));
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let input = keep_alive(
            Box::pin(UnboundedReceiverStream::new(rx)),
            Duration::from_secs(10),
            Duration::from_secs(30),
        );
        let mut out = deliver(input, exchange(&usage));
        assert_eq!(out.next().await.unwrap().unwrap(), ": keepalive\n\n");
        tx.send(Ok(Bytes::from_static(b"data: {\"type\":\"response.cre")))
            .unwrap();
        assert_eq!(
            out.next().await.unwrap().unwrap(),
            "data: {\"type\":\"response.cre"
        );
        let tail = collect(out).await;
        assert!(!String::from_utf8_lossy(&tail).contains(": keepalive"));
        assert!(String::from_utf8_lossy(&tail).contains("upstream sent nothing for 30s"));
        assert_eq!(usage.snapshot().failure_requests, 1);
    }

    #[tokio::test]
    async fn cancellation_drops_the_upstream_and_records_abandonment() {
        let logs = Logs::capture();
        let usage = Arc::new(UsageRecorder::new(None));
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tx.send(Ok(Bytes::from_static(
            b"data: {\"type\":\"response.created\"}\n\n",
        )))
        .unwrap();
        let mut out = deliver(Box::pin(UnboundedReceiverStream::new(rx)), exchange(&usage));
        out.next().await.unwrap().unwrap();

        drop(out);

        assert!(tx.is_closed());
        assert_eq!(
            logs.at_least(tracing::Level::INFO)[0].field("outcome"),
            Some("abandoned")
        );
        let snapshot = usage.snapshot();
        assert_eq!(
            (snapshot.success_requests, snapshot.failure_requests),
            (0, 0)
        );
    }

    #[tokio::test]
    async fn invalid_success_json_is_an_upstream_failure_not_a_cancellation() {
        let usage = Arc::new(UsageRecorder::new(None));
        let upstream = axum::http::Response::new("not JSON").into();
        let error = response(upstream, exchange(&usage)).await.unwrap_err();
        assert!(matches!(error.error, ByokError::Http(_)));
        assert_eq!(usage.snapshot().failure_requests, 1);
    }
}
