//! SSE stream adapters: keepalives while the upstream is silent, and the
//! delivery of an Anthropic stream that accounts for it and always ends it
//! properly.

use std::time::Duration;

use axum::http::StatusCode;
use byokey_types::ByokError;
use byokey_types::traits::ByteStream;
use bytes::Bytes;
use futures_util::{Future, StreamExt as _, TryStreamExt as _, stream, stream::try_unfold};
use serde_json::Value;

use crate::error::{anthropic_envelope, describe_status};
use crate::exchange::Exchange;

/// Deliver an upstream's Anthropic SSE stream to the client, accounting
/// for it in `exchange`.
///
/// Bytes pass through unchanged, and the events in them tell `exchange`
/// the token usage, the stop reason and how the stream ended:
/// `message_stop` completes it, an `error` event fails it. Anthropic
/// clients read a stream until one of those two, so a stream whose upstream
/// fails or closes before either gets an `error` event appended, which
/// fails the exchange too. A client that goes away first drops the
/// exchange unfinished, which records it as abandoned.
pub(crate) fn deliver_anthropic_stream(inner: ByteStream, exchange: Exchange) -> ByteStream {
    struct State {
        inner: ByteStream,
        buf: Vec<u8>,
        /// `None` once the exchange has ended.
        exchange: Option<Exchange>,
        /// An `error` event was appended; nothing follows it.
        closed: bool,
    }

    impl State {
        /// Read one SSE line; a terminal event ends the exchange.
        fn read_line(&mut self, line: &[u8]) {
            let (Some(exchange), Some(event)) = (self.exchange.as_mut(), sse_event(line)) else {
                return;
            };
            match event.get("type").and_then(Value::as_str) {
                Some("message_stop") => {
                    if let Some(exchange) = self.exchange.take() {
                        exchange.complete();
                    }
                }
                Some("error") => {
                    if let Some(exchange) = self.exchange.take() {
                        exchange.fail_with_event(&event);
                    }
                }
                _ => exchange.read_event(&event),
            }
        }

        /// The `error` event that ends the stream with `err`, which also
        /// ends the exchange.
        fn close_with(&mut self, err: &ByokError) -> Bytes {
            self.closed = true;
            if let Some(exchange) = self.exchange.take() {
                exchange.fail(err);
            }
            anthropic_error_event(err)
        }
    }

    Box::pin(try_unfold(
        State {
            inner,
            buf: Vec::new(),
            exchange: Some(exchange),
            closed: false,
        },
        |mut s| async move {
            if s.closed {
                return Ok(None);
            }
            match s.inner.next().await {
                Some(Ok(bytes)) => {
                    if let Some(exchange) = s.exchange.as_mut() {
                        if bytes.as_ref() == KEEPALIVE {
                            exchange.kept_alive();
                        } else {
                            exchange.received();
                            let mut buf = std::mem::take(&mut s.buf);
                            split_lines(&mut buf, &bytes, |line| s.read_line(line));
                            s.buf = buf;
                        }
                    }
                    Ok(Some((bytes, s)))
                }
                Some(Err(e)) => {
                    let event = s.close_with(&e);
                    Ok(Some((event, s)))
                }
                None => {
                    // A last line without its newline still counts.
                    let rest = std::mem::take(&mut s.buf);
                    if !rest.is_empty() {
                        s.read_line(&rest);
                    }
                    if s.exchange.is_none() {
                        return Ok(None);
                    }
                    let event = s.close_with(&ByokError::Http(
                        "the upstream closed the stream before it finished".into(),
                    ));
                    Ok(Some((event, s)))
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
/// (see [`deliver_anthropic_stream`]) rather than as an HTTP status.
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

/// Converts a [`reqwest::Response`] into a [`ByteStream`].
///
/// A body error carries its full source chain: the top-level text ("error
/// decoding response body") does not say whether the connection was reset,
/// the peer sent GOAWAY, or a frame was malformed.
pub(crate) fn response_to_stream(resp: reqwest::Response) -> ByteStream {
    Box::pin(
        resp.bytes_stream()
            .map_err(|e| ByokError::Http(format!("{e}: {}", error_chain(&e)))),
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UsageRecorder;
    use crate::test_logs::{Logged, Logs};
    use byokey_types::ProviderId;
    use futures_util::stream;
    use std::sync::Arc;
    use tracing::Level;

    fn chunks(chunks: Vec<Result<&'static str, ByokError>>) -> ByteStream {
        Box::pin(stream::iter(
            chunks
                .into_iter()
                .map(|c| c.map(|s| Bytes::from_static(s.as_bytes()))),
        ))
    }

    fn exchange() -> Exchange {
        Exchange::start(
            &Arc::new(UsageRecorder::new(None)),
            ProviderId::Copilot,
            "m",
            "a",
        )
    }

    /// The single line an exchange logged when it ended.
    fn ended(logs: &Logs) -> Logged {
        let mut logged = logs.at_least(Level::INFO);
        assert_eq!(logged.len(), 1, "one line per exchange: {logged:?}");
        logged.remove(0)
    }

    /// What the client receives for `chunks`, and how the exchange ended.
    async fn delivered(chunks_in: Vec<Result<&'static str, ByokError>>) -> (String, Logged) {
        let logs = Logs::capture();
        let out: Vec<_> = deliver_anthropic_stream(chunks(chunks_in), exchange())
            .collect()
            .await;
        let text = out
            .into_iter()
            .map(|c| String::from_utf8(c.unwrap().to_vec()).unwrap())
            .collect();
        (text, ended(&logs))
    }

    #[tokio::test]
    async fn a_finished_anthropic_stream_is_passed_through_untouched() {
        let body = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12}}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
        let (out, ended) = delivered(vec![Ok(body)]).await;
        assert_eq!(out, body);
        assert_eq!(ended.field("outcome"), Some("completed"));
        assert_eq!(ended.field("input_tokens"), Some("12"));

        let body = "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n";
        let (out, ended) = delivered(vec![Ok(body)]).await;
        assert_eq!(out, body, "an upstream error event is terminal too");
        assert_eq!(ended.field("outcome"), Some("failed"));
        assert_eq!(ended.field("upstream_message"), Some("Overloaded"));
    }

    #[tokio::test]
    async fn a_client_that_leaves_early_abandons_the_exchange() {
        let logs = Logs::capture();
        let mut out = deliver_anthropic_stream(
            chunks(vec![
                Ok(": keepalive\n\n"),
                Ok("data: {\"type\":\"message_start\"}\n\n"),
                Ok("data: {\"type\":\"message_stop\"}\n\n"),
            ]),
            exchange(),
        );
        out.next().await.unwrap().unwrap();
        out.next().await.unwrap().unwrap();
        drop(out);
        let ended = ended(&logs);
        assert_eq!(ended.field("outcome"), Some("abandoned"));
        assert_eq!(ended.field("keepalives"), Some("1"));
        assert!(ended.field("first_byte_ms").is_some());
    }

    #[tokio::test]
    async fn a_truncated_anthropic_stream_ends_with_an_error_event() {
        // The terminal event split across chunks still counts.
        let (out, ended) = delivered(vec![
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
        assert_eq!(ended.field("outcome"), Some("failed"));
        assert_eq!(
            ended.field("error"),
            Some("http error: the upstream closed the stream before it finished")
        );

        // Only the terminal event's type counts, not any earlier event.
        let (out, ended) = delivered(vec![Ok(
            "data: {\"type\":\"message_stop\"}\n\ndata: {\"type\":\"ping\"}\n\n",
        )])
        .await;
        assert_eq!(ended.field("outcome"), Some("completed"));
        assert!(
            !out.contains("event: error"),
            "message_stop seen earlier is enough"
        );
    }

    #[tokio::test]
    async fn a_failing_anthropic_stream_ends_cleanly_with_the_failure() {
        let (out, ended) = delivered(vec![
            Ok("data: {\"type\":\"message_start\"}\n\n"),
            Err(ByokError::Http("connection reset".into())),
        ])
        .await;
        assert!(out.ends_with("\n\n"));
        assert!(out.contains("event: error"));
        assert!(out.contains("connection reset"));
        assert_eq!(ended.field("outcome"), Some("failed"));
        assert_eq!(ended.field("error"), Some("http error: connection reset"));
    }

    #[tokio::test]
    async fn an_upstream_error_envelope_is_forwarded_inside_the_stream() {
        let body = r#"{"type":"error","error":{"type":"rate_limit_error","message":"slow down"}}"#;
        let (out, ended) = delivered(vec![Err(ByokError::Upstream {
            status: 429,
            body: body.into(),
            retry_after: None,
        })])
        .await;
        let ev: Value = serde_json::from_str(out.split_once("data: ").unwrap().1.trim()).unwrap();
        assert_eq!(ev["error"]["type"], "rate_limit_error");
        assert_eq!(ev["error"]["message"], "slow down");
        assert_eq!(ended.field("outcome"), Some("rejected"));
        assert_eq!(ended.field("status"), Some("429"));
        assert_eq!(ended.field("upstream_message"), Some("slow down"));

        // A non-JSON body is described from its status.
        let (out, _) = delivered(vec![Err(ByokError::Upstream {
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
    async fn a_last_line_without_its_newline_still_counts() {
        let (_, ended) = delivered(vec![
            Ok("data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12}}}\n\n"),
            Ok(r#"data: {"type":"message_delta","usage":{"output_tokens":7}}"#),
        ])
        .await;
        assert_eq!(ended.field("output_tokens"), Some("7"));

        let (out, ended) = delivered(vec![Ok(r#"data: {"type":"message_stop"}"#)]).await;
        assert_eq!(ended.field("outcome"), Some("completed"));
        assert!(!out.contains("event: error"));
    }
}
