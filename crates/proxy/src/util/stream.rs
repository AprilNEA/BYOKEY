//! Generic SSE stream tapping for token usage extraction.

use std::sync::Arc;

use byokey_types::ByokError;
use byokey_types::traits::ByteStream;
use futures_util::{StreamExt as _, stream::try_unfold};
use serde_json::Value;

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
                    s.buf.extend_from_slice(&bytes);
                    while let Some(nl) = s.buf.iter().position(|&b| b == b'\n') {
                        let line: Vec<u8> = s.buf.drain(..=nl).collect();
                        let line = String::from_utf8_lossy(&line);
                        let line = line.trim();
                        if let Some(data) = line.strip_prefix("data: ")
                            && data != "[DONE]"
                            && let Ok(ev) = serde_json::from_str::<Value>(data)
                        {
                            s.parser.parse_line(&ev);
                        }
                    }
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
                    let (input, output) = s.parser.finish();
                    s.usage
                        .record_success_for(&s.model, &s.provider, &s.account_id, input, output);
                    Ok(None)
                }
            }
        },
    ))
}

/// Converts an `rquest::Response` into a [`ByteStream`].
pub(crate) fn response_to_stream(resp: rquest::Response) -> ByteStream {
    Box::pin(resp.bytes_stream().map(|r| {
        r.map_err(|e| {
            tracing::error!(error = %e, "response_to_stream: rquest byte stream error");
            ByokError::from(e)
        })
    }))
}

// ── Parser implementations ──────────────────────────────────────────

pub(crate) struct OpenAIParser {
    input: u64,
    output: u64,
}

impl OpenAIParser {
    pub(crate) fn new() -> Self {
        Self {
            input: 0,
            output: 0,
        }
    }
}

impl UsageParser for OpenAIParser {
    fn parse_line(&mut self, ev: &Value) {
        if let Some(usage) = ev.get("usage") {
            if let Some(v) = usage.get("prompt_tokens").and_then(Value::as_u64) {
                self.input = v;
            }
            if let Some(v) = usage.get("completion_tokens").and_then(Value::as_u64) {
                self.output = v;
            }
        }
    }
    fn finish(self) -> (u64, u64) {
        (self.input, self.output)
    }
}

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

pub(crate) struct CodexParser {
    input: u64,
    output: u64,
}

impl CodexParser {
    pub(crate) fn new() -> Self {
        Self {
            input: 0,
            output: 0,
        }
    }
}

impl UsageParser for CodexParser {
    fn parse_line(&mut self, ev: &Value) {
        if ev.get("type").and_then(Value::as_str) == Some("response.completed") {
            if let Some(v) = ev
                .pointer("/response/usage/input_tokens")
                .and_then(Value::as_u64)
            {
                self.input = v;
            }
            if let Some(v) = ev
                .pointer("/response/usage/output_tokens")
                .and_then(Value::as_u64)
            {
                self.output = v;
            }
        }
    }
    fn finish(self) -> (u64, u64) {
        (self.input, self.output)
    }
}

pub(crate) struct GeminiParser {
    input: u64,
    output: u64,
}

impl GeminiParser {
    pub(crate) fn new() -> Self {
        Self {
            input: 0,
            output: 0,
        }
    }
}

impl UsageParser for GeminiParser {
    fn parse_line(&mut self, ev: &Value) {
        if ev.get("usageMetadata").is_some() {
            if let Some(v) = ev
                .pointer("/usageMetadata/promptTokenCount")
                .and_then(Value::as_u64)
            {
                self.input = v;
            }
            if let Some(v) = ev
                .pointer("/usageMetadata/candidatesTokenCount")
                .and_then(Value::as_u64)
            {
                self.output = v;
            }
        }
    }
    fn finish(self) -> (u64, u64) {
        (self.input, self.output)
    }
}
