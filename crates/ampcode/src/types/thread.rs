//! Thread types — matches the JSON schema of Ampcode thread files
//! (`~/.local/share/amp/threads/T-<uuid>.json`) and the API responses.
//!
//! All types implement `Serialize` + `Deserialize` and can round-trip
//! the wire JSON without loss.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Top-level thread ──────────────────────────────────────────────────────────

/// A complete Amp thread with all messages.
///
/// Corresponds directly to the JSON structure stored in
/// `~/.local/share/amp/threads/T-<uuid>.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    /// Schema version / mutation counter.
    pub v: u64,
    /// Thread identifier in `T-<uuid>` format.
    pub id: String,
    /// Creation timestamp (Unix epoch milliseconds).
    pub created: u64,
    /// Optional title set by the user or inferred by Amp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Agent mode string (e.g. `"smart"`, `"auto"`, `"code"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_mode: Option<String>,
    /// All messages in the thread, in order.
    #[serde(default)]
    pub messages: Vec<Message>,
    /// Relationships to other threads (handoff, fork, mention).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relationships: Vec<Relationship>,
    /// Environment context captured at thread start (opaque JSON).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<Value>,
    /// Next message ID counter (internal to Amp CLI).
    #[serde(default, rename = "nextMessageId")]
    pub next_message_id: u64,
}

/// Lightweight thread summary — only fields needed for listing.
///
/// Deserializes from the same full thread JSON but ignores the heavy
/// `content` fields, making it fast to scan large thread directories.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSummary {
    /// Thread identifier in `T-<uuid>` format.
    pub id: String,
    /// Creation timestamp (Unix epoch milliseconds).
    pub created: u64,
    /// Optional title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Agent mode string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_mode: Option<String>,
    /// Message stubs (only role + usage, no content bodies).
    #[serde(default)]
    pub messages: Vec<MessageStub>,
}

/// Lightweight message representation for summary scanning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageStub {
    /// `"user"`, `"assistant"`, or `"info"`.
    pub role: String,
    /// Usage data (present on assistant turns only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<UsageStub>,
}

/// Minimal usage fields for summary scanning.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageStub {
    /// Model identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Input tokens consumed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    /// Output tokens generated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
}

// ── Message ───────────────────────────────────────────────────────────────────

/// A single message within a thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    /// `"user"`, `"assistant"`, or `"info"`.
    pub role: String,
    /// Monotonically increasing message ID within the thread.
    pub message_id: u64,
    /// Content blocks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<ContentBlock>,
    /// Token usage (present on assistant turns only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// Message completion state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<MessageState>,
}

// ── Content blocks ────────────────────────────────────────────────────────────

/// A typed content block within a message.
///
/// Uses internally-tagged serde representation via the `type` field.
/// Unrecognised block types are preserved as [`ContentBlock::Unknown`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Plain text content.
    Text {
        /// The text content.
        text: String,
    },
    /// Model thinking / chain-of-thought.
    Thinking {
        /// The thinking text.
        thinking: String,
    },
    /// Redacted thinking (encrypted, opaque to the client).
    RedactedThinking {
        /// Base64-encoded redacted thinking data.
        data: String,
    },
    /// Tool invocation.
    ToolUse {
        /// Unique tool use ID.
        id: String,
        /// Tool name (e.g. `"Bash"`, `"Read"`).
        name: String,
        /// Tool input parameters.
        input: Value,
    },
    /// Tool execution result.
    ToolResult {
        /// ID of the tool use this result corresponds to.
        #[serde(rename = "toolUseID")]
        tool_use_id: String,
        /// Execution result.
        run: ToolRun,
    },
    /// A content block type not recognized by this library version.
    #[serde(other)]
    Unknown,
}

/// Tool execution result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRun {
    /// `"done"`, `"error"`, `"cancelled"`, `"rejected-by-user"`,
    /// or `"blocked-on-user"`.
    pub status: String,
    /// Tool output on success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Error details on failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}

/// Token usage for an assistant turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    /// Model identifier (e.g. `"claude-opus-4-6"`).
    pub model: String,
    /// Input tokens consumed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    /// Output tokens generated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    /// Tokens used to create the prompt cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u64>,
    /// Tokens read from the prompt cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<u64>,
    /// Total input tokens including cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_input_tokens: Option<u64>,
}

/// Message completion state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageState {
    /// State type (e.g. `"complete"`, `"streaming"`).
    #[serde(rename = "type")]
    pub state_type: String,
    /// Stop reason (e.g. `"end_turn"`, `"tool_use"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
}

/// Relationship to another thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Relationship {
    /// Related thread ID.
    #[serde(rename = "threadID")]
    pub thread_id: String,
    /// Relationship type (`"handoff"`, `"fork"`, or `"mention"`).
    #[serde(rename = "type")]
    pub rel_type: String,
    /// Role in the relationship (`"parent"` or `"child"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deserialize_minimal_thread() {
        let json_str =
            r#"{"v":0,"id":"T-test-1234","created":1711728000000,"messages":[],"nextMessageId":0}"#;
        let thread: Thread = serde_json::from_str(json_str).unwrap();
        assert_eq!(thread.id, "T-test-1234");
        assert!(thread.messages.is_empty());
        assert!(thread.title.is_none());
        assert_eq!(thread.v, 0);
    }

    #[test]
    fn deserialize_full_thread_with_all_block_types() {
        let json_val = json!({
            "v": 5,
            "id": "T-test-5678",
            "created": 1_711_728_000_000_u64,
            "messages": [
                {
                    "role": "user",
                    "messageId": 0,
                    "content": [{"type": "text", "text": "hello"}]
                },
                {
                    "role": "assistant",
                    "messageId": 1,
                    "content": [
                        {"type": "thinking", "thinking": "hmm"},
                        {"type": "redacted_thinking", "data": "base64data=="},
                        {"type": "tool_use", "id": "toolu_01", "name": "Bash", "input": {"cmd": "ls"}},
                    ],
                    "usage": {
                        "model": "claude-opus-4-6",
                        "inputTokens": 100,
                        "outputTokens": 50,
                        "cacheCreationInputTokens": 10,
                        "cacheReadInputTokens": 5,
                        "totalInputTokens": 115
                    },
                    "state": {"type": "complete", "stopReason": "tool_use"}
                },
                {
                    "role": "user",
                    "messageId": 2,
                    "content": [{
                        "type": "tool_result",
                        "toolUseID": "toolu_01",
                        "run": {"status": "done", "result": {"output": "file.txt"}}
                    }]
                }
            ],
            "agentMode": "smart",
            "title": "Test thread",
            "nextMessageId": 3,
            "relationships": [{
                "threadID": "T-parent-0001",
                "type": "handoff",
                "role": "child"
            }]
        });

        let thread: Thread = serde_json::from_value(json_val).unwrap();
        assert_eq!(thread.messages.len(), 3);
        assert_eq!(thread.agent_mode.as_deref(), Some("smart"));
        assert_eq!(thread.relationships.len(), 1);
        assert_eq!(thread.relationships[0].thread_id, "T-parent-0001");

        // Content blocks
        assert!(
            matches!(&thread.messages[0].content[0], ContentBlock::Text { text } if text == "hello")
        );
        assert!(
            matches!(&thread.messages[1].content[0], ContentBlock::Thinking { thinking } if thinking == "hmm")
        );
        assert!(
            matches!(&thread.messages[1].content[1], ContentBlock::RedactedThinking { data } if data == "base64data==")
        );
        assert!(
            matches!(&thread.messages[1].content[2], ContentBlock::ToolUse { name, .. } if name == "Bash")
        );
        assert!(
            matches!(&thread.messages[2].content[0], ContentBlock::ToolResult { tool_use_id, .. } if tool_use_id == "toolu_01")
        );

        // Usage
        let usage = thread.messages[1].usage.as_ref().unwrap();
        assert_eq!(usage.model, "claude-opus-4-6");
        assert_eq!(usage.input_tokens, Some(100));
        assert_eq!(usage.output_tokens, Some(50));
        assert_eq!(usage.cache_creation_input_tokens, Some(10));
        assert_eq!(usage.total_input_tokens, Some(115));

        // State
        let state = thread.messages[1].state.as_ref().unwrap();
        assert_eq!(state.state_type, "complete");
        assert_eq!(state.stop_reason.as_deref(), Some("tool_use"));
    }

    #[test]
    fn unknown_content_block_type() {
        let json_val = json!({
            "v": 1, "id": "T-unk", "created": 0, "nextMessageId": 1,
            "messages": [{
                "role": "assistant", "messageId": 0,
                "content": [{"type": "some_future_type", "data": 42}]
            }]
        });
        let thread: Thread = serde_json::from_value(json_val).unwrap();
        assert!(matches!(
            &thread.messages[0].content[0],
            ContentBlock::Unknown
        ));
    }

    #[test]
    fn summary_skips_content() {
        let json_val = json!({
            "v": 100,
            "id": "T-skip-test",
            "created": 1_711_728_000_000_u64,
            "messages": [{
                "role": "user",
                "messageId": 0,
                "content": [{"type": "text", "text": "big payload here"}],
                "userState": {"activeEditor": "foo.rs"}
            }, {
                "role": "assistant",
                "messageId": 1,
                "content": [{"type": "text", "text": "response"}],
                "usage": {"model": "claude-opus-4-6", "inputTokens": 50, "outputTokens": 25}
            }],
            "nextMessageId": 2,
            "env": {"initial": {"platform": {"os": "darwin"}}}
        });

        let summary: ThreadSummary = serde_json::from_value(json_val).unwrap();
        assert_eq!(summary.id, "T-skip-test");
        assert_eq!(summary.messages.len(), 2);
        assert_eq!(summary.messages[0].role, "user");
        assert!(summary.messages[0].usage.is_none());
        assert_eq!(summary.messages[1].role, "assistant");
        let usage = summary.messages[1].usage.as_ref().unwrap();
        assert_eq!(usage.model.as_deref(), Some("claude-opus-4-6"));
        assert_eq!(usage.input_tokens, Some(50));
    }

    #[test]
    fn usage_optional_fields() {
        let json_val = json!({"model": "gpt-5"});
        let usage: Usage = serde_json::from_value(json_val).unwrap();
        assert_eq!(usage.model, "gpt-5");
        assert!(usage.input_tokens.is_none());
        assert!(usage.output_tokens.is_none());
        assert!(usage.cache_creation_input_tokens.is_none());
    }

    #[test]
    fn thread_roundtrip() {
        let json_val = json!({
            "v": 2, "id": "T-rt", "created": 1000, "nextMessageId": 1,
            "title": "roundtrip",
            "messages": [{"role": "user", "messageId": 0, "content": [{"type": "text", "text": "hi"}]}]
        });
        let thread: Thread = serde_json::from_value(json_val.clone()).unwrap();
        let reserialized = serde_json::to_value(&thread).unwrap();
        let thread2: Thread = serde_json::from_value(reserialized).unwrap();
        assert_eq!(thread.id, thread2.id);
        assert_eq!(thread.v, thread2.v);
        assert_eq!(thread.title, thread2.title);
        assert_eq!(thread.messages.len(), thread2.messages.len());
    }
}
