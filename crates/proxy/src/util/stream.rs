//! SSE stream adapters: token usage tapping and Anthropic stream termination.

use std::sync::Arc;

use byokey_types::ByokError;
use byokey_types::traits::ByteStream;
use bytes::Bytes;
use futures_util::{StreamExt as _, stream::try_unfold};
use serde_json::{Value, json};

use crate::UsageRecorder;

/// Implemented per-provider to extract (`input_tokens`, `output_tokens`) from SSE data lines.
pub(crate) trait UsageParser: Send + 'static {
    fn parse_line(&mut self, data: &Value);
    fn finish(self) -> (u64, u64);
}

/// Wraps a [`ByteStream`], scanning each SSE `data:` line through `parser`
/// and recording usage via [`UsageRecorder`] when the stream ends.
/// All bytes are forwarded unchanged.
pub(crate) fn tap_usage_stream<P: UsageParser>(
    inner: ByteStream,
    usage: Arc<UsageRecorder>,
    model: String,
    provider: String,
    account_id: String,
    parser: P,
) -> ByteStream {
    struct State<P> {
        inner: ByteStream,
        buf: Vec<u8>,
        usage: Arc<UsageRecorder>,
        model: String,
        provider: String,
        account_id: String,
        parser: P,
    }

    Box::pin(try_unfold(
        State {
            inner,
            buf: Vec::new(),
            usage,
            model,
            provider,
            account_id,
            parser,
        },
        |mut s| async move {
            match s.inner.next().await {
                Some(Ok(bytes)) => {
                    split_lines(&mut s.buf, &bytes, |line| {
                        if let Some(ev) = sse_event(line) {
                            s.parser.parse_line(&ev);
                        }
                    });
                    Ok(Some((bytes, s)))
                }
                Some(Err(e)) => {
                    tracing::error!(
                        model = %s.model,
                        provider = %s.provider,
                        account_id = %s.account_id,
                        error = %e,
                        "tap_usage_stream: upstream SSE stream yielded error"
                    );
                    s.usage
                        .record_failure_for(&s.model, &s.provider, &s.account_id);
                    Err(e)
                }
                None => {
                    if !s.buf.is_empty()
                        && let Some(ev) = sse_event(&std::mem::take(&mut s.buf))
                    {
                        s.parser.parse_line(&ev);
                    }
                    let (input, output) = s.parser.finish();
                    s.usage
                        .record_success_for(&s.model, &s.provider, &s.account_id, input, output);
                    Ok(None)
                }
            }
        },
    ))
}

/// Append `bytes` to `buf` and hand each complete line to `f`.
fn split_lines(buf: &mut Vec<u8>, bytes: &[u8], mut f: impl FnMut(&[u8])) {
    buf.extend_from_slice(bytes);
    while let Some(nl) = buf.iter().position(|&b| b == b'\n') {
        let line: Vec<u8> = buf.drain(..=nl).collect();
        f(&line);
    }
}

/// The JSON event carried by an SSE `data:` line, if it is one.
fn sse_event(line: &[u8]) -> Option<Value> {
    let line = String::from_utf8_lossy(line);
    let data = line.trim().strip_prefix("data:")?.trim_start();
    if data == "[DONE]" {
        return None;
    }
    serde_json::from_str(data).ok()
}

/// An Anthropic SSE `error` event.
fn anthropic_error_event(message: &str) -> Bytes {
    let event = json!({"type": "error", "error": {"type": "api_error", "message": message}});
    Bytes::from(format!("event: error\ndata: {event}\n\n"))
}

/// Anthropic clients read a stream until `message_stop` or `error`. An
/// upstream that closes the connection before either, or fails midway,
/// would leave them waiting; this ends such a stream with an `error` event.
pub(crate) fn terminate_anthropic_stream(inner: ByteStream) -> ByteStream {
    struct State {
        inner: ByteStream,
        buf: Vec<u8>,
        terminated: bool,
        closed: bool,
    }

    Box::pin(try_unfold(
        State {
            inner,
            buf: Vec::new(),
            terminated: false,
            closed: false,
        },
        |mut s| async move {
            if s.closed {
                return Ok(None);
            }
            match s.inner.next().await {
                Some(Ok(bytes)) => {
                    split_lines(&mut s.buf, &bytes, |line| {
                        if let Some(ev) = sse_event(line)
                            && matches!(
                                ev.get("type").and_then(Value::as_str),
                                Some("message_stop" | "error")
                            )
                        {
                            s.terminated = true;
                        }
                    });
                    Ok(Some((bytes, s)))
                }
                Some(Err(e)) => {
                    s.closed = true;
                    Ok(Some((anthropic_error_event(&e.to_string()), s)))
                }
                None if s.terminated => Ok(None),
                None => {
                    s.closed = true;
                    tracing::warn!("upstream closed the stream before message_stop");
                    Ok(Some((
                        anthropic_error_event("the upstream closed the stream before it finished"),
                        s,
                    )))
                }
            }
        },
    ))
}

/// Converts a `wreq::Response` into a [`ByteStream`].
pub(crate) fn response_to_stream(resp: wreq::Response) -> ByteStream {
    Box::pin(resp.bytes_stream().map(|r| {
        r.map_err(|e| {
            tracing::error!(error = %e, "response_to_stream: wreq byte stream error");
            ByokError::from(e)
        })
    }))
}

/// Reads `input_tokens` and `output_tokens` from an Anthropic Messages stream.
pub(crate) struct AnthropicParser {
    input: u64,
    output: u64,
}

impl AnthropicParser {
    pub(crate) fn new() -> Self {
        Self {
            input: 0,
            output: 0,
        }
    }
}

impl UsageParser for AnthropicParser {
    fn parse_line(&mut self, ev: &Value) {
        match ev.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                if let Some(v) = ev
                    .pointer("/message/usage/input_tokens")
                    .and_then(Value::as_u64)
                {
                    self.input = v;
                }
            }
            Some("message_delta") => {
                if let Some(v) = ev.pointer("/usage/output_tokens").and_then(Value::as_u64) {
                    self.output = v;
                }
            }
            _ => {}
        }
    }
    fn finish(self) -> (u64, u64) {
        (self.input, self.output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;

    async fn terminated(chunks: Vec<Result<&'static str, ByokError>>) -> String {
        let inner: ByteStream = Box::pin(stream::iter(
            chunks
                .into_iter()
                .map(|c| c.map(|s| Bytes::from_static(s.as_bytes()))),
        ));
        let out: Vec<_> = terminate_anthropic_stream(inner).collect().await;
        out.into_iter()
            .map(|c| String::from_utf8(c.unwrap().to_vec()).unwrap())
            .collect()
    }

    #[tokio::test]
    async fn a_finished_anthropic_stream_is_passed_through_untouched() {
        let body = "event: message_start\ndata: {\"type\":\"message_start\"}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
        assert_eq!(terminated(vec![Ok(body)]).await, body);
        let body = "event: error\ndata: {\"type\":\"error\",\"error\":{}}\n\n";
        assert_eq!(terminated(vec![Ok(body)]).await, body);
    }

    #[tokio::test]
    async fn a_truncated_anthropic_stream_ends_with_an_error_event() {
        // The terminal event split across chunks still counts.
        let out = terminated(vec![
            Ok("event: message_start\ndata: {\"type\":\"message_st"),
            Ok("art\"}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\"}\n\n"),
        ])
        .await;
        assert!(out.starts_with("event: message_start"));
        let tail = out.rsplit("\n\n").nth(1).unwrap();
        assert!(tail.starts_with("event: error\ndata: "), "{tail}");
        let ev: Value = serde_json::from_str(tail.split_once("data: ").unwrap().1).unwrap();
        assert_eq!(ev["type"], "error");
        assert_eq!(ev["error"]["type"], "api_error");

        // Only the terminal event's type counts, not any earlier event.
        let out = terminated(vec![Ok(
            "data: {\"type\":\"message_stop\"}\n\ndata: {\"type\":\"ping\"}\n\n",
        )])
        .await;
        assert!(
            !out.contains("event: error"),
            "message_stop seen earlier is enough"
        );
    }

    #[tokio::test]
    async fn a_failing_anthropic_stream_ends_cleanly_with_the_failure() {
        let out = terminated(vec![
            Ok("data: {\"type\":\"message_start\"}\n\n"),
            Err(ByokError::Http("connection reset".into())),
        ])
        .await;
        assert!(out.ends_with("\n\n"));
        assert!(out.contains("event: error"));
        assert!(out.contains("connection reset"));
    }

    #[tokio::test]
    async fn tap_usage_stream_parses_final_line_without_newline() {
        let usage = Arc::new(UsageRecorder::new(None));
        let inner: ByteStream = Box::pin(stream::iter([
            Ok(Bytes::from_static(
                b"data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12}}}\n\n",
            )),
            Ok(Bytes::from_static(
                br#"data: {"type":"message_delta","usage":{"output_tokens":7}}"#,
            )),
        ]));

        let chunks: Vec<_> = tap_usage_stream(
            inner,
            Arc::clone(&usage),
            "claude-test".to_owned(),
            "claude".to_owned(),
            "default".to_owned(),
            AnthropicParser::new(),
        )
        .collect()
        .await;

        assert_eq!(chunks.len(), 2);
        assert!(chunks.iter().all(Result::is_ok));
        let snapshot = usage.snapshot();
        assert_eq!(snapshot.success_requests, 1);
        assert_eq!(snapshot.input_tokens, 12);
        assert_eq!(snapshot.output_tokens, 7);
    }
}
