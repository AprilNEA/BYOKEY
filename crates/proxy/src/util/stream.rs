//! SSE stream adapters: token usage tapping, keepalives while the upstream
//! is silent, and Anthropic stream termination.

use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use byokey_types::ByokError;
use byokey_types::traits::ByteStream;
use bytes::Bytes;
use futures_util::{Future, StreamExt as _, TryStreamExt as _, stream, stream::try_unfold};
use serde_json::Value;

use crate::UsageRecorder;
use crate::error::{anthropic_envelope, describe_status};

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

/// An Anthropic SSE `error` event for `err`. An upstream error envelope is
/// forwarded as is; anything else is described from the status, or is an
/// `api_error` carrying the error text.
fn anthropic_error_event(err: &ByokError) -> Bytes {
    let payload = match err {
        ByokError::Upstream { status, body, .. } => serde_json::from_str::<Value>(body)
            .ok()
            .filter(|v| v.get("error").is_some_and(Value::is_object))
            .unwrap_or_else(|| {
                let status = StatusCode::from_u16(*status).unwrap_or(StatusCode::BAD_GATEWAY);
                let (error_type, _) = describe_status(status);
                anthropic_envelope(
                    error_type,
                    &format!("upstream error: status={status}, body={body}"),
                )
            }),
        other => anthropic_envelope("api_error", &other.to_string()),
    };
    Bytes::from(format!("event: error\ndata: {payload}\n\n"))
}

/// Comment line Anthropic clients ignore but count as activity.
const KEEPALIVE: &[u8] = b": keepalive\n\n";

/// Keep an SSE stream moving while its upstream is silent.
///
/// Claude Code shows a retry banner after 20 s without a byte and cannot be
/// told to wait longer for a gateway that sends nothing; Anthropic's own
/// guidance is for the gateway to write keepalive comments. This writes one
/// after every `interval` of silence and gives up with an error after
/// `limit` of silence in a row: a live upstream sends `ping` events far more
/// often than that, so a longer silence is a dead stream.
pub(crate) fn keep_alive(inner: ByteStream, interval: Duration, limit: Duration) -> ByteStream {
    struct State {
        inner: ByteStream,
        interval: Duration,
        limit: Duration,
        silent: Duration,
    }

    // `try_unfold` ends the stream after an `Err`, so no flag is needed.
    Box::pin(try_unfold(
        State {
            inner,
            interval,
            limit,
            silent: Duration::ZERO,
        },
        |mut s| async move {
            match tokio::time::timeout(s.interval, s.inner.next()).await {
                Ok(Some(Ok(bytes))) => {
                    s.silent = Duration::ZERO;
                    Ok(Some((bytes, s)))
                }
                Ok(Some(Err(e))) => Err(e),
                Ok(None) => Ok(None),
                Err(_elapsed) => {
                    s.silent += s.interval;
                    if s.silent >= s.limit {
                        return Err(ByokError::Http(format!(
                            "the upstream sent nothing for {}s",
                            s.silent.as_secs()
                        )));
                    }
                    Ok(Some((Bytes::from_static(KEEPALIVE), s)))
                }
            }
        },
    ))
}

/// The body of a streamed response whose upstream has not answered yet: a
/// keepalive comment right away, then the upstream body once it answers, or
/// the error it answered with.
///
/// The response headers go to the client before the upstream's arrive, so
/// an upstream failure is reported inside the stream as an `error` event
/// (see [`terminate_anthropic_stream`]) rather than as an HTTP status.
pub(crate) fn deferred_stream(
    pending: impl Future<Output = reqwest::Result<reqwest::Response>> + Send + 'static,
) -> ByteStream {
    let body = async move {
        let resp = pending.await.map_err(ByokError::from)?;
        if resp.status().is_success() {
            Ok(response_to_stream(resp))
        } else {
            Err(ByokError::from_response(resp).await)
        }
    };
    Box::pin(
        stream::once(async { Ok(Bytes::from_static(KEEPALIVE)) })
            .chain(stream::once(body).try_flatten()),
    )
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
                    Ok(Some((anthropic_error_event(&e), s)))
                }
                None if s.terminated => Ok(None),
                None => {
                    s.closed = true;
                    tracing::warn!("upstream closed the stream before message_stop");
                    Ok(Some((
                        anthropic_error_event(&ByokError::Http(
                            "the upstream closed the stream before it finished".into(),
                        )),
                        s,
                    )))
                }
            }
        },
    ))
}

/// Converts a [`reqwest::Response`] into a [`ByteStream`].
///
/// A body error is logged with its full source chain: the top-level text
/// ("error decoding response body") does not say whether the connection was
/// reset, the peer sent GOAWAY, or a frame was malformed.
pub(crate) fn response_to_stream(resp: reqwest::Response) -> ByteStream {
    Box::pin(resp.bytes_stream().map(|r| {
        r.map_err(|e| {
            tracing::error!(error = %e, causes = %error_chain(&e), "upstream byte stream error");
            ByokError::Http(format!("{e}: {}", error_chain(&e)))
        })
    }))
}

/// The `source()` chain of `err`, innermost last, joined with `: `.
pub(crate) fn error_chain(err: &(dyn std::error::Error + 'static)) -> String {
    let mut parts = Vec::new();
    let mut cur = err.source();
    while let Some(e) = cur {
        parts.push(e.to_string());
        cur = e.source();
    }
    if parts.is_empty() {
        "no further cause".to_owned()
    } else {
        parts.join(": ")
    }
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
    async fn an_upstream_error_envelope_is_forwarded_inside_the_stream() {
        let body = r#"{"type":"error","error":{"type":"rate_limit_error","message":"slow down"}}"#;
        let out = terminated(vec![Err(ByokError::Upstream {
            status: 429,
            body: body.into(),
            retry_after: None,
        })])
        .await;
        let ev: Value = serde_json::from_str(out.split_once("data: ").unwrap().1.trim()).unwrap();
        assert_eq!(ev["error"]["type"], "rate_limit_error");
        assert_eq!(ev["error"]["message"], "slow down");

        // A non-JSON body is described from its status.
        let out = terminated(vec![Err(ByokError::Upstream {
            status: 503,
            body: "<html>busy</html>".into(),
            retry_after: None,
        })])
        .await;
        let ev: Value = serde_json::from_str(out.split_once("data: ").unwrap().1.trim()).unwrap();
        assert_eq!(ev["error"]["type"], "api_error");
        assert!(
            ev["error"]["message"]
                .as_str()
                .unwrap()
                .contains("status=503")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn silence_is_bridged_with_keepalives_and_cut_off_eventually() {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Result<Bytes, ByokError>>();
        let inner: ByteStream = Box::pin(tokio_stream::wrappers::UnboundedReceiverStream::new(rx));
        let mut out = keep_alive(inner, Duration::from_secs(10), Duration::from_secs(30));

        // Data resets the silence clock.
        tx.send(Ok(Bytes::from_static(b"data: 1\n\n"))).unwrap();
        assert_eq!(out.next().await.unwrap().unwrap(), "data: 1\n\n");
        // 10 s of silence: a keepalive, twice.
        assert_eq!(out.next().await.unwrap().unwrap(), KEEPALIVE);
        assert_eq!(out.next().await.unwrap().unwrap(), KEEPALIVE);
        // Data again, then the clock starts over.
        tx.send(Ok(Bytes::from_static(b"data: 2\n\n"))).unwrap();
        assert_eq!(out.next().await.unwrap().unwrap(), "data: 2\n\n");
        assert_eq!(out.next().await.unwrap().unwrap(), KEEPALIVE);
        assert_eq!(out.next().await.unwrap().unwrap(), KEEPALIVE);
        // 30 s of silence in a row: the stream fails and ends.
        let err = out.next().await.unwrap().unwrap_err();
        assert!(matches!(err, ByokError::Http(m) if m.contains("30s")));
        assert!(out.next().await.is_none());
        drop(tx);
    }

    #[tokio::test]
    async fn a_deferred_upstream_answers_after_a_keepalive() {
        let ok: reqwest::Response = axum::http::Response::builder()
            .body("data: hi\n\n")
            .unwrap()
            .into();
        let out: Vec<_> = deferred_stream(async { Ok(ok) }).collect().await;
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].as_ref().unwrap(), KEEPALIVE);
        assert_eq!(out[1].as_ref().unwrap(), "data: hi\n\n");

        let denied: reqwest::Response = axum::http::Response::builder()
            .status(429)
            .body(r#"{"type":"error","error":{"type":"rate_limit_error","message":"no"}}"#)
            .unwrap()
            .into();
        let out: Vec<_> = deferred_stream(async { Ok(denied) }).collect().await;
        assert_eq!(out.len(), 2);
        assert!(matches!(
            &out[1],
            Err(ByokError::Upstream { status: 429, .. })
        ));
    }

    #[test]
    fn error_chains_name_every_cause() {
        #[derive(Debug)]
        struct Layer(&'static str, Option<Box<Layer>>);
        impl std::fmt::Display for Layer {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.0)
            }
        }
        impl std::error::Error for Layer {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                self.1.as_deref().map(|l| l as _)
            }
        }
        let chained = Layer(
            "error decoding response body",
            Some(Box::new(Layer(
                "stream error",
                Some(Box::new(Layer("connection reset by peer", None))),
            ))),
        );
        assert_eq!(
            error_chain(&chained),
            "stream error: connection reset by peer"
        );
        assert_eq!(error_chain(&Layer("alone", None)), "no further cause");
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
