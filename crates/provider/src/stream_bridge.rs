//! Bridge between aigw's canonical `StreamEvent` and `OpenAI` SSE chunk format.
//!
//! Provides conversion from [`aigw_core::model::StreamEvent`] to OpenAI-format
//! `chat.completion.chunk` SSE bytes, suitable for proxying to downstream clients.

use aigw::anthropic as _; // ensure the crate is linked
use serde_json::json;

/// Re-export canonical types used by stream conversion.
pub use aigw_core::model::{FinishReason, StreamEvent, Usage};

/// Mutable context maintained across a single streaming response.
///
/// Populated by [`ResponseMeta`](StreamEvent::ResponseMeta) and referenced by
/// subsequent events to produce consistent `id` / `model` fields.
pub struct SseContext {
    /// OpenAI-format response ID (e.g. `"chatcmpl-msg_01"`).
    pub id: String,
    /// Model identifier.
    pub model: String,
}

impl Default for SseContext {
    fn default() -> Self {
        Self {
            id: "chatcmpl-unknown".to_owned(),
            model: "unknown".to_owned(),
        }
    }
}

/// Convert a single [`StreamEvent`] into `OpenAI` SSE chunk bytes.
///
/// Returns `None` if the event produces no output (should be skipped).
/// The caller is responsible for writing the returned bytes to the output stream.
#[allow(clippy::too_many_lines)]
pub fn stream_event_to_sse(event: &StreamEvent, ctx: &mut SseContext) -> Option<Vec<u8>> {
    match event {
        StreamEvent::ResponseMeta { id, model } => {
            ctx.id = format!("chatcmpl-{id}");
            ctx.model.clone_from(model);
            let chunk = json!({
                "id": &ctx.id,
                "object": "chat.completion.chunk",
                "model": &ctx.model,
                "choices": [{
                    "index": 0,
                    "delta": {"role": "assistant", "content": ""},
                    "finish_reason": null
                }]
            });
            Some(format!("data: {chunk}\n\n").into_bytes())
        }

        StreamEvent::ContentDelta(text) => {
            let chunk = json!({
                "id": &ctx.id,
                "object": "chat.completion.chunk",
                "model": &ctx.model,
                "choices": [{
                    "index": 0,
                    "delta": {"content": text},
                    "finish_reason": null
                }]
            });
            Some(format!("data: {chunk}\n\n").into_bytes())
        }

        StreamEvent::ReasoningDelta(text) => {
            let chunk = json!({
                "id": &ctx.id,
                "object": "chat.completion.chunk",
                "model": &ctx.model,
                "choices": [{
                    "index": 0,
                    "delta": {"reasoning_content": text},
                    "finish_reason": null
                }]
            });
            Some(format!("data: {chunk}\n\n").into_bytes())
        }

        // Reasoning block lifecycle: ReasoningStart carries no client-visible
        // payload (the OpenAI Chat SSE format has no equivalent); ReasoningEnd
        // surfaces the integrity signature on the same field BYOKEY's clients
        // already consume from the deprecated ReasoningSignature path.
        StreamEvent::ReasoningStart { .. } => None,

        #[allow(deprecated)]
        StreamEvent::ReasoningEnd { signature, .. }
        | StreamEvent::ReasoningSignature(signature) => {
            let chunk = json!({
                "id": &ctx.id,
                "object": "chat.completion.chunk",
                "model": &ctx.model,
                "choices": [{
                    "index": 0,
                    "delta": {"reasoning_signature": signature},
                    "finish_reason": null
                }]
            });
            Some(format!("data: {chunk}\n\n").into_bytes())
        }

        StreamEvent::ToolCallStart { index, id, name } => {
            let chunk = json!({
                "id": &ctx.id,
                "object": "chat.completion.chunk",
                "model": &ctx.model,
                "choices": [{"index": 0, "delta": {
                    "tool_calls": [{"index": index, "id": id, "type": "function", "function": {"name": name, "arguments": ""}}]
                }, "finish_reason": null}]
            });
            Some(format!("data: {chunk}\n\n").into_bytes())
        }

        StreamEvent::ToolCallDelta { index, arguments } => {
            let chunk = json!({
                "id": &ctx.id,
                "object": "chat.completion.chunk",
                "model": &ctx.model,
                "choices": [{"index": 0, "delta": {
                    "tool_calls": [{"index": index, "function": {"arguments": arguments}}]
                }, "finish_reason": null}]
            });
            Some(format!("data: {chunk}\n\n").into_bytes())
        }

        StreamEvent::Finish(reason) => {
            let reason_str = match reason {
                FinishReason::Stop => "stop",
                FinishReason::Length => "length",
                FinishReason::ToolCalls => "tool_calls",
                FinishReason::ContentFilter => "content_filter",
                FinishReason::Unknown(s) => s.as_str(),
            };
            let chunk = json!({
                "id": &ctx.id,
                "object": "chat.completion.chunk",
                "model": &ctx.model,
                "choices": [{
                    "index": 0,
                    "delta": {},
                    "finish_reason": reason_str
                }]
            });
            Some(format!("data: {chunk}\n\n").into_bytes())
        }

        StreamEvent::Usage(usage) => {
            let chunk = json!({
                "id": &ctx.id,
                "object": "chat.completion.chunk",
                "model": &ctx.model,
                "choices": [],
                "usage": {
                    "prompt_tokens": usage.prompt_tokens.unwrap_or(0),
                    "completion_tokens": usage.completion_tokens.unwrap_or(0),
                    "total_tokens": usage.total_tokens.unwrap_or(0)
                }
            });
            Some(format!("data: {chunk}\n\n").into_bytes())
        }

        StreamEvent::Done => Some(b"data: [DONE]\n\n".to_vec()),
    }
}

/// Convert a batch of [`StreamEvent`]s into a single byte buffer.
///
/// Events that produce no output are silently skipped.
pub fn stream_events_to_sse(events: &[StreamEvent], ctx: &mut SseContext) -> Vec<u8> {
    let mut out = Vec::new();
    for event in events {
        if let Some(bytes) = stream_event_to_sse(event, ctx) {
            out.extend_from_slice(&bytes);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_meta_sets_context() {
        let mut ctx = SseContext::default();
        let event = StreamEvent::ResponseMeta {
            id: "msg_01".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
        };
        let bytes = stream_event_to_sse(&event, &mut ctx).unwrap();
        let line = String::from_utf8(bytes).unwrap();
        assert!(line.contains("chatcmpl-msg_01"));
        assert!(line.contains("claude-sonnet-4-20250514"));
        assert!(line.contains(r#""role":"assistant"#));
        assert_eq!(ctx.id, "chatcmpl-msg_01");
        assert_eq!(ctx.model, "claude-sonnet-4-20250514");
    }

    #[test]
    fn content_delta_produces_chunk() {
        let mut ctx = SseContext {
            id: "chatcmpl-test".into(),
            model: "test-model".into(),
        };
        let event = StreamEvent::ContentDelta("Hello".to_string());
        let bytes = stream_event_to_sse(&event, &mut ctx).unwrap();
        let line = String::from_utf8(bytes).unwrap();
        assert!(line.contains(r#""content":"Hello"#));
    }

    #[test]
    fn reasoning_delta_emits_reasoning_content() {
        let mut ctx = SseContext {
            id: "chatcmpl-test".into(),
            model: "o4-mini".into(),
        };
        let event = StreamEvent::ReasoningDelta("thinking...".to_string());
        let bytes = stream_event_to_sse(&event, &mut ctx).unwrap();
        let line = String::from_utf8(bytes).unwrap();
        assert!(line.contains(r#""reasoning_content":"thinking..."#));
    }

    #[test]
    #[allow(deprecated)]
    fn reasoning_signature_emits_reasoning_signature() {
        let mut ctx = SseContext {
            id: "chatcmpl-test".into(),
            model: "o4-mini".into(),
        };
        let event = StreamEvent::ReasoningSignature("opaque_sig".to_string());
        let bytes = stream_event_to_sse(&event, &mut ctx).unwrap();
        let line = String::from_utf8(bytes).unwrap();
        assert!(line.contains(r#""reasoning_signature":"opaque_sig"#));
    }

    #[test]
    fn tool_call_start_produces_chunk() {
        let mut ctx = SseContext::default();
        let event = StreamEvent::ToolCallStart {
            index: 0,
            id: "toolu_01".into(),
            name: "get_weather".into(),
        };
        let bytes = stream_event_to_sse(&event, &mut ctx).unwrap();
        let line = String::from_utf8(bytes).unwrap();
        assert!(line.contains("toolu_01"));
        assert!(line.contains("get_weather"));
        assert!(line.contains(r#""type":"function"#));
    }

    #[test]
    fn finish_reason_mapping() {
        let mut ctx = SseContext::default();
        for (reason, expected) in [
            (FinishReason::Stop, "stop"),
            (FinishReason::Length, "length"),
            (FinishReason::ToolCalls, "tool_calls"),
            (FinishReason::ContentFilter, "content_filter"),
        ] {
            let event = StreamEvent::Finish(reason);
            let bytes = stream_event_to_sse(&event, &mut ctx).unwrap();
            let line = String::from_utf8(bytes).unwrap();
            assert!(
                line.contains(&format!(r#""finish_reason":"{expected}""#)),
                "expected {expected} in: {line}"
            );
        }
    }

    #[test]
    fn usage_chunk() {
        let mut ctx = SseContext::default();
        let event = StreamEvent::Usage(Usage {
            prompt_tokens: Some(25),
            completion_tokens: Some(15),
            total_tokens: Some(40),
            extra: serde_json::Map::default(),
        });
        let bytes = stream_event_to_sse(&event, &mut ctx).unwrap();
        let line = String::from_utf8(bytes).unwrap();
        assert!(line.contains(r#""prompt_tokens":25"#));
        assert!(line.contains(r#""completion_tokens":15"#));
    }

    #[test]
    fn done_event() {
        let mut ctx = SseContext::default();
        let event = StreamEvent::Done;
        let bytes = stream_event_to_sse(&event, &mut ctx).unwrap();
        assert_eq!(bytes, b"data: [DONE]\n\n");
    }

    #[test]
    fn batch_conversion() {
        let mut ctx = SseContext::default();
        let events = vec![
            StreamEvent::ResponseMeta {
                id: "msg".into(),
                model: "m".into(),
            },
            StreamEvent::ContentDelta("Hi".into()),
            StreamEvent::Done,
        ];
        let out = stream_events_to_sse(&events, &mut ctx);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("chatcmpl-msg"));
        assert!(text.contains(r#""content":"Hi"#));
        assert!(text.ends_with("data: [DONE]\n\n"));
    }
}
