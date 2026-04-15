//! Amp thread reading — exposes local Amp CLI thread files via management API.
//!
//! Reads thread JSON files from `~/.local/share/amp/threads/` and serves them
//! as structured data through two endpoints:
//! - `GET /v0/management/amp/threads`      — paginated thread list (summaries)
//! - `GET /v0/management/amp/threads/{id}` — full thread detail with messages

use arc_swap::ArcSwap;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{self, File},
    io::BufReader,
    path::PathBuf,
    sync::Arc,
};
use utoipa::{IntoParams, ToSchema};

use crate::AppState;

// ── Threads directory resolution ─────────────────────────────────────

/// Resolve the Amp threads directory.
///
/// Amp CLI uses `~/.local/share/amp/threads/` on both macOS and Linux
/// (XDG data dir, not `~/Library`).
fn threads_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| String::from("/tmp"));
    PathBuf::from(home).join(".local/share/amp/threads")
}

/// Validate a thread ID to prevent path traversal.
/// Must match `T-` followed by hex digits and hyphens (UUID format).
fn is_valid_thread_id(id: &str) -> bool {
    id.starts_with("T-")
        && id.len() > 2
        && id[2..].chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

// ── Internal deserialization types (camelCase, matching Amp JSON) ─────

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawThreadSummary {
    id: String,
    created: u64,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    messages: Vec<RawMessageStub>,
    #[serde(default)]
    agent_mode: Option<String>,
}

#[derive(Deserialize)]
struct RawMessageStub {
    role: String,
    #[serde(default)]
    usage: Option<RawUsageStub>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawUsageStub {
    model: Option<String>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawThread {
    v: u64,
    id: String,
    created: u64,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    messages: Vec<RawMessage>,
    #[serde(default)]
    agent_mode: Option<String>,
    #[serde(default)]
    relationships: Vec<RawRelationship>,
    #[serde(default)]
    env: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMessage {
    role: String,
    message_id: u64,
    #[serde(default)]
    content: Vec<Value>,
    #[serde(default)]
    usage: Option<RawUsage>,
    #[serde(default)]
    state: Option<RawMessageState>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawUsage {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
    #[serde(default)]
    total_input_tokens: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMessageState {
    #[serde(rename = "type")]
    state_type: String,
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRelationship {
    #[serde(rename = "threadID")]
    thread_id: String,
    #[serde(rename = "type")]
    rel_type: String,
    #[serde(default)]
    role: Option<String>,
}

// ── API response types (snake_case, with ToSchema for OpenAPI) ───────

/// Paginated list of Amp thread summaries.
#[derive(Serialize, ToSchema)]
pub struct AmpThreadListResponse {
    /// Thread summaries (sorted by `created` descending).
    pub threads: Vec<AmpThreadSummary>,
    /// Total number of matching threads (before pagination).
    pub total: usize,
}

/// Summary of a single Amp thread (excludes message bodies).
#[derive(Serialize, ToSchema)]
pub struct AmpThreadSummary {
    pub id: String,
    /// Creation timestamp (Unix epoch milliseconds).
    pub created: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Number of messages in the thread.
    pub message_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_mode: Option<String>,
    /// Model used in the last assistant response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_model: Option<String>,
    /// Sum of input tokens across all assistant turns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_input_tokens: Option<u64>,
    /// Sum of output tokens across all assistant turns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_output_tokens: Option<u64>,
    /// File size on disk (bytes).
    pub file_size_bytes: u64,
}

/// Full Amp thread with all messages.
#[derive(Serialize, ToSchema)]
pub struct AmpThreadDetail {
    pub id: String,
    /// Mutation counter (incremented on every thread change).
    pub v: u64,
    /// Creation timestamp (Unix epoch milliseconds).
    pub created: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_mode: Option<String>,
    pub messages: Vec<AmpMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub relationships: Vec<AmpRelationship>,
    /// Thread environment context (opaque JSON).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<Value>,
}

/// A single message within an Amp thread.
#[derive(Serialize, ToSchema)]
pub struct AmpMessage {
    /// `"user"`, `"assistant"`, or `"info"`.
    pub role: String,
    pub message_id: u64,
    pub content: Vec<AmpContentBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<AmpUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<AmpMessageState>,
}

/// A content block within a message.
#[derive(Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AmpContentBlock {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        run: AmpToolRun,
    },
    /// Content block type not recognized by this parser.
    Unknown {
        #[serde(skip_serializing_if = "Option::is_none")]
        original_type: Option<String>,
    },
}

/// Tool execution result.
#[derive(Serialize, ToSchema)]
pub struct AmpToolRun {
    /// `"done"`, `"error"`, `"cancelled"`, `"rejected-by-user"`, or `"blocked-on-user"`.
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}

/// Token usage for an assistant turn.
#[derive(Serialize, ToSchema)]
pub struct AmpUsage {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_input_tokens: Option<u64>,
}

/// Assistant message state.
#[derive(Serialize, ToSchema)]
pub struct AmpMessageState {
    pub state_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
}

/// Relationship to another thread (handoff, fork, or mention).
#[derive(Serialize, ToSchema)]
pub struct AmpRelationship {
    pub thread_id: String,
    /// `"handoff"`, `"fork"`, or `"mention"`.
    pub rel_type: String,
    /// `"parent"` or `"child"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

/// Query parameters for the thread list endpoint.
#[derive(Deserialize, IntoParams, ToSchema)]
pub struct AmpThreadListQuery {
    /// Maximum threads to return (default 50, max 200).
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Number of threads to skip (default 0).
    #[serde(default)]
    pub offset: usize,
    /// Filter by whether threads have messages. Default `true` (hide empty).
    #[serde(default = "default_true")]
    pub has_messages: Option<bool>,
}

const fn default_limit() -> usize {
    50
}

#[allow(clippy::unnecessary_wraps)] // serde default requires matching return type
const fn default_true() -> Option<bool> {
    Some(true)
}

// ── Parsing logic ────────────────────────────────────────────────────

fn parse_summary(path: &std::path::Path) -> Option<AmpThreadSummary> {
    let file = File::open(path).ok()?;
    let file_size = file.metadata().ok()?.len();
    let raw: RawThreadSummary = serde_json::from_reader(BufReader::new(file)).ok()?;

    let mut last_model: Option<String> = None;
    let mut sum_input: u64 = 0;
    let mut sum_output: u64 = 0;
    let mut has_usage = false;

    for msg in &raw.messages {
        if msg.role == "assistant"
            && let Some(u) = &msg.usage
        {
            if let Some(m) = &u.model {
                last_model = Some(m.clone());
            }
            sum_input += u.input_tokens.unwrap_or(0);
            sum_output += u.output_tokens.unwrap_or(0);
            has_usage = true;
        }
    }

    Some(AmpThreadSummary {
        message_count: raw.messages.len(),
        id: raw.id,
        created: raw.created,
        title: raw.title,
        agent_mode: raw.agent_mode,
        last_model,
        total_input_tokens: has_usage.then_some(sum_input),
        total_output_tokens: has_usage.then_some(sum_output),
        file_size_bytes: file_size,
    })
}

/// Convert a raw JSON `Value` content block into a typed `AmpContentBlock`.
fn convert_content_block(v: &Value) -> AmpContentBlock {
    let block_type = v.get("type").and_then(Value::as_str).unwrap_or("");
    match block_type {
        "text" => AmpContentBlock::Text {
            text: v
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        },
        "thinking" | "redacted_thinking" => AmpContentBlock::Thinking {
            thinking: v
                .get("thinking")
                .or_else(|| v.get("data"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        },
        "tool_use" => AmpContentBlock::ToolUse {
            id: v
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            name: v
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            input: v.get("input").cloned().unwrap_or(Value::Null),
        },
        "tool_result" => {
            let run_val = v.get("run");
            AmpContentBlock::ToolResult {
                tool_use_id: v
                    .get("toolUseID")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                run: AmpToolRun {
                    status: run_val
                        .and_then(|r| r.get("status"))
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_string(),
                    result: run_val.and_then(|r| r.get("result")).cloned(),
                    error: run_val.and_then(|r| r.get("error")).cloned(),
                },
            }
        }
        _ => AmpContentBlock::Unknown {
            original_type: Some(block_type.to_string()),
        },
    }
}

fn convert_message(raw: RawMessage) -> AmpMessage {
    AmpMessage {
        role: raw.role,
        message_id: raw.message_id,
        content: raw.content.iter().map(convert_content_block).collect(),
        usage: raw.usage.map(|u| AmpUsage {
            model: u.model.unwrap_or_default(),
            input_tokens: u.input_tokens,
            output_tokens: u.output_tokens,
            cache_creation_input_tokens: u.cache_creation_input_tokens,
            cache_read_input_tokens: u.cache_read_input_tokens,
            total_input_tokens: u.total_input_tokens,
        }),
        state: raw.state.map(|s| AmpMessageState {
            state_type: s.state_type,
            stop_reason: s.stop_reason,
        }),
    }
}

fn parse_detail(path: &std::path::Path) -> Result<AmpThreadDetail, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let raw: RawThread =
        serde_json::from_reader(BufReader::new(file)).map_err(|e| e.to_string())?;

    Ok(AmpThreadDetail {
        id: raw.id,
        v: raw.v,
        created: raw.created,
        title: raw.title,
        agent_mode: raw.agent_mode,
        messages: raw.messages.into_iter().map(convert_message).collect(),
        relationships: raw
            .relationships
            .into_iter()
            .map(|r| AmpRelationship {
                thread_id: r.thread_id,
                rel_type: r.rel_type,
                role: r.role,
            })
            .collect(),
        env: raw.env,
    })
}

// ── In-memory thread index with file watching ───────────────────────

/// Pre-sorted, in-memory index of all Amp thread summaries.
///
/// Built once at startup by scanning `~/.local/share/amp/threads/`, then
/// kept up-to-date via `notify` file-system events.  The inner `ArcSwap`
/// allows lock-free reads from HTTP handlers while the watcher task
/// atomically swaps in a new snapshot on every change.
pub struct AmpThreadIndex {
    summaries: ArcSwap<Vec<AmpThreadSummary>>,
}

impl AmpThreadIndex {
    /// Build the initial index by scanning the threads directory.
    ///
    /// This performs synchronous filesystem I/O and should be called from
    /// within `spawn_blocking` or at startup before the server binds.
    /// Build the initial index by scanning the threads directory.
    ///
    /// This performs synchronous filesystem I/O and should be called from
    /// within `spawn_blocking` or at startup before the server binds.
    #[must_use]
    pub fn build() -> Self {
        let summaries = scan_all_summaries();
        Self {
            summaries: ArcSwap::from_pointee(summaries),
        }
    }

    /// Create an empty index (for tests or when the directory is absent).
    #[must_use]
    pub fn empty() -> Self {
        Self {
            summaries: ArcSwap::from_pointee(Vec::new()),
        }
    }

    /// Return a snapshot of all cached summaries (sorted by `created` desc).
    pub fn list(&self) -> arc_swap::Guard<Arc<Vec<AmpThreadSummary>>> {
        self.summaries.load()
    }

    /// Start background file watching.
    ///
    /// Watches `~/.local/share/amp/threads/` for create / modify / remove
    /// events and rebuilds the index on each change.  Events are debounced
    /// (500 ms) so rapid writes from Amp don't cause redundant re-scans.
    ///
    /// # Panics
    ///
    /// Panics if the OS file watcher cannot be created or the directory
    /// cannot be registered for watching.
    pub fn watch(self: &Arc<Self>) {
        use notify::{RecursiveMode, Watcher as _};

        let index = Arc::clone(self);
        let dir = threads_dir();

        tokio::task::spawn_blocking(move || {
            if !dir.is_dir() {
                tracing::debug!(path = %dir.display(), "amp threads dir not found, skipping watch");
                return;
            }

            let (tx, rx) = std::sync::mpsc::channel();

            let mut watcher =
                notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                    if let Ok(ev) = res {
                        // Only react to JSON file changes.
                        let dominated_by_json = ev.paths.iter().any(|p| {
                            p.extension()
                                .is_some_and(|e| e.eq_ignore_ascii_case("json"))
                        });
                        if dominated_by_json {
                            let _ = tx.send(());
                        }
                    }
                })
                .expect("failed to create file watcher");

            watcher
                .watch(&dir, RecursiveMode::NonRecursive)
                .expect("failed to watch amp threads directory");

            tracing::info!(path = %dir.display(), "watching amp threads directory");

            // Debounce: drain all pending signals, then rebuild once.
            while rx.recv().is_ok() {
                // Drain any events that arrived while we were scanning.
                while rx.try_recv().is_ok() {}

                // Small delay to let Amp finish writing.
                std::thread::sleep(std::time::Duration::from_millis(500));

                // Drain again after the delay.
                while rx.try_recv().is_ok() {}

                let new = scan_all_summaries();
                tracing::debug!(count = new.len(), "amp thread index rebuilt");
                index.summaries.store(Arc::new(new));
            }
        });
    }
}

/// Scan the threads directory and return all parseable summaries, sorted
/// by `created` descending (newest first).
fn scan_all_summaries() -> Vec<AmpThreadSummary> {
    let dir = threads_dir();
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut summaries: Vec<AmpThreadSummary> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with("T-")
                || !std::path::Path::new(&name)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
            {
                return None;
            }
            parse_summary(&entry.path())
        })
        .collect();

    summaries.sort_unstable_by(|a, b| b.created.cmp(&a.created));
    summaries
}

// ── Handlers ─────────────────────────────────────────────────────────

/// List Amp thread summaries.
#[utoipa::path(
    get,
    path = "/v0/management/amp/threads",
    params(AmpThreadListQuery),
    responses((status = 200, body = AmpThreadListResponse)),
    tag = "management"
)]
pub async fn list_threads(
    State(state): State<Arc<AppState>>,
    Query(q): Query<AmpThreadListQuery>,
) -> Json<AmpThreadListResponse> {
    let all = state.amp_threads.list();

    // Apply has_messages filter.
    let filtered: Vec<&AmpThreadSummary> = all
        .iter()
        .filter(|s| {
            if let Some(want) = q.has_messages {
                (s.message_count > 0) == want
            } else {
                true
            }
        })
        .collect();

    let total = filtered.len();
    let limit = q.limit.min(200);
    let offset = q.offset.min(total);

    let threads: Vec<AmpThreadSummary> = filtered
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(clone_summary)
        .collect();

    Json(AmpThreadListResponse { threads, total })
}

/// Cheap clone of a summary (all small fields, no deep Value trees).
fn clone_summary(s: &AmpThreadSummary) -> AmpThreadSummary {
    AmpThreadSummary {
        id: s.id.clone(),
        created: s.created,
        title: s.title.clone(),
        message_count: s.message_count,
        agent_mode: s.agent_mode.clone(),
        last_model: s.last_model.clone(),
        total_input_tokens: s.total_input_tokens,
        total_output_tokens: s.total_output_tokens,
        file_size_bytes: s.file_size_bytes,
    }
}

/// Get full Amp thread detail by ID.
#[utoipa::path(
    get,
    path = "/v0/management/amp/threads/{id}",
    params(("id" = String, Path, description = "Thread ID (e.g. T-019d38dd-45f9-7617-8e7f-03b730ba197a)")),
    responses(
        (status = 200, body = AmpThreadDetail),
        (status = 400, description = "Invalid thread ID format"),
        (status = 404, description = "Thread not found"),
    ),
    tag = "management"
)]
pub async fn get_thread(Path(id): Path<String>) -> Response {
    if !is_valid_thread_id(&id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": {"message": "invalid thread ID format", "type": "invalid_request_error"}
            })),
        )
            .into_response();
    }

    let path = threads_dir().join(format!("{id}.json"));

    let result = tokio::task::spawn_blocking(move || {
        if !path.exists() {
            return Err(StatusCode::NOT_FOUND);
        }
        parse_detail(&path).map_err(|e| {
            tracing::error!(error = %e, "failed to parse amp thread");
            StatusCode::INTERNAL_SERVER_ERROR
        })
    })
    .await
    .unwrap_or(Err(StatusCode::INTERNAL_SERVER_ERROR));

    match result {
        Ok(detail) => Json(detail).into_response(),
        Err(StatusCode::NOT_FOUND) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": {"message": "thread not found", "type": "not_found"}
            })),
        )
            .into_response(),
        Err(status) => (
            status,
            Json(serde_json::json!({
                "error": {"message": "failed to parse thread", "type": "server_error"}
            })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn valid_thread_ids() {
        assert!(is_valid_thread_id("T-019d38dd-45f9-7617-8e7f-03b730ba197a"));
        assert!(is_valid_thread_id("T-fc68e9f5-9621-4ee2-b8d9-d954ba656de4"));
        assert!(is_valid_thread_id("T-abcdef0123456789"));
    }

    #[test]
    fn invalid_thread_ids() {
        assert!(!is_valid_thread_id(""));
        assert!(!is_valid_thread_id("T-"));
        assert!(!is_valid_thread_id("../etc/passwd"));
        assert!(!is_valid_thread_id("T-../../foo"));
        assert!(!is_valid_thread_id("T-abc def"));
        assert!(!is_valid_thread_id("not-a-thread"));
    }

    #[test]
    fn parse_empty_thread_json() {
        let json_str =
            r#"{"v":0,"id":"T-test-1234","created":1711728000000,"messages":[],"nextMessageId":0}"#;
        let raw: RawThreadSummary = serde_json::from_str(json_str).unwrap();
        assert_eq!(raw.id, "T-test-1234");
        assert!(raw.messages.is_empty());
        assert!(raw.title.is_none());
    }

    #[test]
    fn parse_thread_with_messages() {
        let json_str = json!({
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
                        {"type": "thinking", "thinking": "hmm", "signature": "sig"},
                        {"type": "tool_use", "id": "toolu_01", "name": "Bash", "input": {"cmd": "ls"}, "complete": true},
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
                        "run": {"status": "done", "result": {"output": "file.txt", "exitCode": 0}}
                    }]
                }
            ],
            "agentMode": "smart",
            "title": "Test thread",
            "nextMessageId": 3
        });

        let raw: RawThread = serde_json::from_value(json_str).unwrap();
        assert_eq!(raw.messages.len(), 3);
        assert_eq!(raw.agent_mode.as_deref(), Some("smart"));

        // Test full conversion.
        let detail = AmpThreadDetail {
            id: raw.id.clone(),
            v: raw.v,
            created: raw.created,
            title: raw.title.clone(),
            agent_mode: raw.agent_mode.clone(),
            messages: raw.messages.into_iter().map(convert_message).collect(),
            relationships: Vec::new(),
            env: None,
        };

        assert_eq!(detail.messages.len(), 3);
        assert_eq!(detail.messages[0].role, "user");
        assert_eq!(detail.messages[1].role, "assistant");
        assert!(detail.messages[1].usage.is_some());

        let usage = detail.messages[1].usage.as_ref().unwrap();
        assert_eq!(usage.model, "claude-opus-4-6");
        assert_eq!(usage.input_tokens, Some(100));
        assert_eq!(usage.output_tokens, Some(50));

        // Verify content blocks.
        assert!(matches!(
            &detail.messages[1].content[0],
            AmpContentBlock::Thinking { .. }
        ));
        assert!(
            matches!(&detail.messages[1].content[1], AmpContentBlock::ToolUse { name, .. } if name == "Bash")
        );
        assert!(matches!(
            &detail.messages[2].content[0],
            AmpContentBlock::ToolResult { .. }
        ));
    }

    #[test]
    fn convert_unknown_content_block() {
        let block = json!({"type": "some_future_type", "data": 42});
        let result = convert_content_block(&block);
        assert!(
            matches!(result, AmpContentBlock::Unknown { original_type: Some(t) } if t == "some_future_type")
        );
    }

    #[test]
    fn summary_deserialization_skips_heavy_fields() {
        // Ensure RawThreadSummary doesn't fail on extra fields (content, env, etc.)
        let json_str = json!({
            "v": 100,
            "id": "T-skip-test",
            "created": 1_711_728_000_000_u64,
            "messages": [{
                "role": "user",
                "messageId": 0,
                "content": [{"type": "text", "text": "this should be skipped by summary parser"}],
                "userState": {"activeEditor": "foo.rs"},
                "fileMentions": {"files": []}
            }],
            "nextMessageId": 1,
            "env": {"initial": {"platform": {"os": "darwin"}}},
            "meta": {"traces": []},
            "~debug": {"something": true}
        });

        let raw: RawThreadSummary = serde_json::from_value(json_str).unwrap();
        assert_eq!(raw.id, "T-skip-test");
        assert_eq!(raw.messages.len(), 1);
    }
}
