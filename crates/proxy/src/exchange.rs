//! One upstream exchange: a request BYOKEY sent upstream on a client's
//! behalf, and what came back.
//!
//! An exchange is logged once, when it ends, under an `upstream` span that
//! names the provider, model and account, and the upstream's own request
//! id once its headers arrive. How it ended is the `outcome` field:
//!
//! - `completed` (`info`): the upstream finished its answer.
//! - `rejected` (`warn`): the upstream answered with an error status, with
//!   its error type and message.
//! - `failed` (`warn`): the exchange broke off: no connection, an `error`
//!   event mid-stream, a stream cut short or silent for too long.
//! - `abandoned` (`info`): the client went away first.
//!
//! The line also carries the time until the upstream's first byte, the
//! total time, the token counts and stop reason the upstream reported, and
//! how many keepalives BYOKEY wrote while the upstream was silent. Every
//! outcome but `abandoned` is counted in the usage statistics, with tokens
//! for `completed` only.

use byokey_types::{ByokError, ProviderId, Usage, UsageRecord};
use futures_util::{Future, FutureExt as _};
use serde_json::Value;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tracing::Span;

use crate::UsageRecorder;
use crate::error::UpstreamMessage;

/// A request in flight to an upstream. Dropping it before it ends means the
/// client went away.
pub(crate) struct Exchange {
    recorder: Arc<UsageRecorder>,
    provider: ProviderId,
    model: String,
    account_id: String,
    span: Span,
    started: Instant,
    /// Set by whichever comes first: the upstream's response headers, or
    /// its first streamed bytes.
    first_byte: Arc<OnceLock<Duration>>,
    report: Report,
    keepalives: u32,
    ended: bool,
}

impl Exchange {
    /// An exchange with `provider` for `model`, sent as `account_id`,
    /// starting now.
    pub(crate) fn start(
        recorder: &Arc<UsageRecorder>,
        provider: ProviderId,
        model: impl Into<String>,
        account_id: impl Into<String>,
    ) -> Self {
        let model = model.into();
        let account_id = account_id.into();
        let span = tracing::info_span!(
            "upstream",
            provider = %provider,
            model = %model,
            account = %account_id,
            attempt = tracing::field::Empty,
            initiator = tracing::field::Empty,
            upstream_request_id = tracing::field::Empty,
        );
        Self {
            recorder: Arc::clone(recorder),
            provider,
            model,
            account_id,
            span,
            started: Instant::now(),
            first_byte: Arc::default(),
            report: Report::default(),
            keepalives: 0,
            ended: false,
        }
    }

    /// Note that this is a retry, the `attempt`-th after the first.
    #[must_use]
    pub(crate) fn attempt(self, attempt: usize) -> Self {
        if attempt > 0 {
            self.span.record("attempt", attempt);
        }
        self
    }

    /// Note who started the request, as Copilot bills it: `user` for a
    /// prompt, `agent` for a tool-loop iteration.
    #[must_use]
    pub(crate) fn initiator(self, initiator: &str) -> Self {
        self.span.record("initiator", initiator);
        self
    }

    /// The span the exchange's log lines belong to.
    pub(crate) fn span(&self) -> &Span {
        &self.span
    }

    /// `pending`, noting when the upstream's headers arrive and the request
    /// id it gives the exchange (`request-id`, or `x-request-id`).
    pub(crate) fn track<F>(&self, pending: F) -> impl Future<Output = F::Output> + use<F>
    where
        F: Future<Output = reqwest::Result<reqwest::Response>>,
    {
        let span = self.span.clone();
        let first_byte = Arc::clone(&self.first_byte);
        let started = self.started;
        pending.inspect(move |result| {
            let Ok(resp) = result else { return };
            let _ = first_byte.set(started.elapsed());
            if let Some(id) = ["request-id", "x-request-id"]
                .into_iter()
                .find_map(|name| resp.headers().get(name)?.to_str().ok())
            {
                span.record("upstream_request_id", id);
            }
        })
    }

    /// The upstream sent bytes of its answer.
    pub(crate) fn received(&self) {
        let _ = self.first_byte.set(self.started.elapsed());
    }

    /// BYOKEY wrote a keepalive while the upstream was silent.
    pub(crate) fn kept_alive(&mut self) {
        self.keepalives += 1;
    }

    /// Take what a streamed event says about usage and the stop reason.
    pub(crate) fn read_event(&mut self, event: &Value) {
        self.report.read_event(event);
    }

    /// The upstream's answer arrived in full as `response`.
    pub(crate) fn complete_with(mut self, response: &Value) {
        self.report = Report::from_response(response);
        self.complete();
    }

    /// The upstream's streamed answer finished.
    pub(crate) fn complete(mut self) {
        self.end(&End::Completed);
    }

    /// The exchange failed with `err`: an upstream error status is a
    /// rejection, anything else a failure.
    pub(crate) fn fail(mut self, err: &ByokError) {
        self.end(&End::Failed(Failure::of(err)));
    }

    /// The upstream's stream ended with the `error` event `event`.
    pub(crate) fn fail_with_event(mut self, event: &Value) {
        self.end(&End::Failed(Failure::of_event(event)));
    }

    fn end(&mut self, end: &End) {
        self.ended = true;
        let failure = match end {
            End::Failed(failure) => Some(failure),
            End::Completed | End::Abandoned => None,
        };
        let report = &self.report;
        let first_byte_ms = self.first_byte.get().copied().map(millis);
        let duration_ms = millis(self.started.elapsed());
        let keepalives = (self.keepalives > 0).then_some(self.keepalives);
        let _entered = self.span.enter();
        macro_rules! log {
            ($level:ident, $outcome:literal, $message:literal) => {
                tracing::$level!(
                    outcome = $outcome,
                    status = failure.and_then(|f| f.status),
                    error_type = failure.and_then(|f| f.error_type.as_deref()),
                    upstream_message = failure.and_then(|f| f.upstream_message.as_deref()),
                    error = failure.and_then(|f| f.error.as_deref()),
                    first_byte_ms,
                    duration_ms,
                    input_tokens = report.input_tokens,
                    output_tokens = report.output_tokens,
                    cache_read_tokens = report.cache_read_tokens,
                    cache_write_tokens = report.cache_write_tokens,
                    stop_reason = report.stop_reason.as_deref(),
                    keepalives,
                    $message
                )
            };
        }
        match end {
            End::Completed => log!(info, "completed", "upstream finished"),
            End::Abandoned => log!(
                info,
                "abandoned",
                "client left before the upstream finished"
            ),
            End::Failed(f) if f.status.is_some() => {
                log!(warn, "rejected", "upstream refused the request");
            }
            End::Failed(_) => log!(warn, "failed", "upstream exchange failed"),
        }
        // The statistics count tokens for completed answers only; a failed
        // exchange's partial counts are in its log line.
        let (usage, success) = match end {
            End::Completed => (Usage::from(&self.report), true),
            End::Failed(_) => (Usage::default(), false),
            End::Abandoned => return,
        };
        self.recorder.record(UsageRecord {
            model: std::mem::take(&mut self.model),
            provider: self.provider,
            account_id: std::mem::take(&mut self.account_id),
            usage,
            success,
        });
    }
}

impl Drop for Exchange {
    fn drop(&mut self) {
        if !self.ended {
            self.end(&End::Abandoned);
        }
    }
}

/// `duration` in whole milliseconds.
fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

enum End {
    Completed,
    Failed(Failure),
    Abandoned,
}

/// Why an exchange failed. Text an upstream wrote is kept apart from
/// BYOKEY's own description: it is logged as `upstream_message`, which
/// stays out of Sentry.
struct Failure {
    /// The upstream's error status; `None` when it never sent one.
    status: Option<u16>,
    error_type: Option<String>,
    upstream_message: Option<String>,
    error: Option<String>,
}

impl Failure {
    fn of(err: &ByokError) -> Self {
        match err {
            ByokError::Upstream { status, body, .. } => {
                let upstream = UpstreamMessage::of(body);
                Self {
                    status: Some(*status),
                    error_type: upstream.error_type,
                    upstream_message: Some(upstream.message),
                    error: None,
                }
            }
            other => Self {
                status: None,
                error_type: None,
                upstream_message: None,
                error: Some(other.to_string()),
            },
        }
    }

    fn of_event(event: &Value) -> Self {
        let upstream = UpstreamMessage::of_envelope(event);
        Self {
            status: None,
            error_type: upstream.as_ref().and_then(|u| u.error_type.clone()),
            upstream_message: upstream.map(|u| u.message),
            error: None,
        }
    }
}

/// What the upstream reported about an answer: token counts as Anthropic's
/// Messages API reports them, and why the model stopped. A count is `None`
/// until the upstream reports it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Report {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_read_tokens: Option<u64>,
    cache_write_tokens: Option<u64>,
    stop_reason: Option<String>,
}

impl Report {
    /// The report of a complete (non-streaming) response.
    fn from_response(response: &Value) -> Self {
        let mut report = Self::default();
        if let Some(usage) = response.get("usage") {
            report.read_usage(usage);
        }
        report.read_stop_reason(response.get("stop_reason"));
        report
    }

    /// Take what a stream event says: the input and cache counts on
    /// `message_start`, the running totals and stop reason on
    /// `message_delta`.
    fn read_event(&mut self, event: &Value) {
        match event.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                if let Some(usage) = event.pointer("/message/usage") {
                    self.read_usage(usage);
                }
            }
            Some("message_delta") => {
                if let Some(usage) = event.get("usage") {
                    self.read_usage(usage);
                }
                self.read_stop_reason(event.pointer("/delta/stop_reason"));
            }
            _ => {}
        }
    }

    fn read_usage(&mut self, usage: &Value) {
        let count = |key: &str| usage.get(key).and_then(Value::as_u64);
        for (slot, key) in [
            (&mut self.input_tokens, "input_tokens"),
            (&mut self.output_tokens, "output_tokens"),
            (&mut self.cache_read_tokens, "cache_read_input_tokens"),
            (&mut self.cache_write_tokens, "cache_creation_input_tokens"),
        ] {
            if let Some(n) = count(key) {
                *slot = Some(n);
            }
        }
    }

    fn read_stop_reason(&mut self, stop_reason: Option<&Value>) {
        if let Some(reason) = stop_reason.and_then(Value::as_str) {
            self.stop_reason = Some(reason.to_owned());
        }
    }
}

impl From<&Report> for Usage {
    fn from(report: &Report) -> Self {
        Self {
            input_tokens: report.input_tokens.unwrap_or(0),
            output_tokens: report.output_tokens.unwrap_or(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_logs::Logs;
    use serde_json::json;
    use tracing::Level;

    fn exchange(recorder: &Arc<UsageRecorder>) -> Exchange {
        Exchange::start(recorder, ProviderId::Copilot, "claude-haiku-4.5", "work")
    }

    #[test]
    fn a_streamed_report_keeps_the_last_totals_and_the_stop_reason() {
        let mut report = Report::default();
        for event in [
            json!({"type": "message_start", "message": {"usage": {
                "input_tokens": 9, "output_tokens": 3,
                "cache_read_input_tokens": 1200, "cache_creation_input_tokens": 0
            }}}),
            json!({"type": "content_block_delta", "delta": {"text": "hi"}}),
            // `message_delta` carries running totals, not increments.
            json!({"type": "message_delta", "delta": {"stop_reason": null}, "usage": {"output_tokens": 5}}),
            json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 14}}),
        ] {
            report.read_event(&event);
        }
        assert_eq!(
            report,
            Report {
                input_tokens: Some(9),
                output_tokens: Some(14),
                cache_read_tokens: Some(1200),
                cache_write_tokens: Some(0),
                stop_reason: Some("end_turn".into()),
            }
        );

        let whole = Report::from_response(&json!({
            "stop_reason": "max_tokens",
            "usage": {"input_tokens": 12, "output_tokens": 7}
        }));
        assert_eq!(
            Usage::from(&whole),
            Usage {
                input_tokens: 12,
                output_tokens: 7
            }
        );
        assert_eq!(whole.stop_reason.as_deref(), Some("max_tokens"));
        assert_eq!(whole.cache_read_tokens, None, "not reported, not zero");
    }

    #[test]
    fn each_exchange_is_logged_once_with_how_it_ended() {
        let logs = Logs::capture();
        let recorder = Arc::new(UsageRecorder::new(None));

        exchange(&recorder).complete_with(&json!({
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 9, "output_tokens": 8, "cache_read_input_tokens": 0}
        }));
        exchange(&recorder).fail(&ByokError::Upstream {
            status: 400,
            body: r#"{"error":{"message":"The use of the web search tool is not supported.","code":"unsupported_value"}}"#.into(),
            retry_after: None,
        });
        exchange(&recorder).fail(&ByokError::Http(
            "the upstream sent nothing for 120s".into(),
        ));
        exchange(&recorder).fail_with_event(&json!({
            "type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}
        }));
        let mut abandoned = exchange(&recorder);
        abandoned.kept_alive();
        drop(abandoned);

        let logged = logs.at_least(Level::INFO);
        let outcomes: Vec<_> = logged
            .iter()
            .map(|e| (e.level, e.field("outcome").unwrap()))
            .collect();
        assert_eq!(
            outcomes,
            [
                (Level::INFO, "completed"),
                (Level::WARN, "rejected"),
                (Level::WARN, "failed"),
                (Level::WARN, "failed"),
                (Level::INFO, "abandoned"),
            ]
        );
        let [completed, rejected, silent, overloaded, abandoned] = &logged[..] else {
            unreachable!()
        };
        assert_eq!(completed.field("output_tokens"), Some("8"));
        assert_eq!(completed.field("cache_read_tokens"), Some("0"));
        assert_eq!(completed.field("stop_reason"), Some("end_turn"));
        assert!(completed.field("duration_ms").is_some());
        assert_eq!(rejected.field("status"), Some("400"));
        assert_eq!(rejected.field("error_type"), Some("unsupported_value"));
        assert_eq!(
            rejected.field("upstream_message"),
            Some("The use of the web search tool is not supported.")
        );
        assert_eq!(
            silent.field("error"),
            Some("http error: the upstream sent nothing for 120s")
        );
        assert_eq!(
            silent.field("upstream_message"),
            None,
            "BYOKEY wrote that one"
        );
        assert_eq!(overloaded.field("error_type"), Some("overloaded_error"));
        assert_eq!(overloaded.field("upstream_message"), Some("Overloaded"));
        assert_eq!(abandoned.field("keepalives"), Some("1"));

        let snap = recorder.snapshot();
        assert_eq!(
            (snap.success_requests, snap.failure_requests),
            (1, 3),
            "an abandoned exchange is not counted"
        );
        assert_eq!((snap.input_tokens, snap.output_tokens), (9, 8));
    }
}
