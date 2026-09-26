//! What Anthropic expects from a Claude Code OAuth session: the billing
//! header and Claude Code prefix at the top of the system prompt, a
//! `metadata.user_id`, and title-cased built-in tool names.
//!
//! The fingerprint algorithm replicates Claude Code's `utils/fingerprint.ts`:
//! `SHA256(SALT + msg[4] + msg[7] + msg[20] + version)[0..3]`.
//!
//! # `cc_entrypoint`
//!
//! `derive_cc_entrypoint` maps an incoming `User-Agent` to the correct
//! `cc_entrypoint` value in the billing header:
//! - `claude-cli` / `claude-code` in UA → `"cli"`
//! - `vscode` / `Code/` in UA → `"vscode"`
//! - Any other UA → `"local-agent"`
//! - `None` (executor path has no incoming UA) → `"cli"`
//!
//! # `cc_workload`
//!
//! Clients may tag traffic with `X-Byokey-Claude-Workload: <value>`.
//! The value is validated against `[A-Za-z0-9_-]+` and appended as
//! `cc_workload=<value>;` after `cc_entrypoint` in the billing header.
//! Invalid or missing values are silently dropped.

use sha2::{Digest as _, Sha256};

/// Default CLI version for billing header and User-Agent.
const DEFAULT_CLI_VERSION: &str = "2.1.109";

/// Salt used by Claude Code's fingerprint.ts — must match the backend validator.
const FINGERPRINT_SALT: &str = "59cf53e54c78";

/// Derives the `cc_entrypoint` value from an incoming `User-Agent` header.
///
/// Rules (case-insensitive, checked in order):
/// 1. UA contains `claude-cli` or `claude-code` → `"cli"`
/// 2. UA contains `vscode` or `Code/` → `"vscode"`
/// 3. Any other non-empty UA → `"local-agent"`
/// 4. `None` (executor path, no incoming UA) → `"cli"`
#[must_use]
pub fn derive_cc_entrypoint(user_agent: Option<&str>) -> &'static str {
    let Some(ua) = user_agent else {
        return "cli";
    };
    let lower = ua.to_lowercase();
    if lower.contains("claude-cli") || lower.contains("claude-code") {
        "cli"
    } else if lower.contains("vscode") || ua.contains("Code/") {
        "vscode"
    } else {
        "local-agent"
    }
}

/// Validates a `cc_workload` value.  Returns `true` iff the value matches
/// `[A-Za-z0-9_-]+` (non-empty).
fn is_valid_workload(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Prepends the billing header and Claude Code prefix to a request body's
/// `system` field, keeping any the client already sent, and sets
/// `metadata.user_id`. OAuth tokens need the billing header to reach Sonnet
/// and Opus.
///
/// `entrypoint` should be the result of [`derive_cc_entrypoint`].
/// `workload` is the optional `X-Byokey-Claude-Workload` header value; invalid
/// values (not matching `[A-Za-z0-9_-]+`) are silently dropped.
pub fn inject_billing_header(
    body: &mut serde_json::Value,
    device_id: &str,
    account_uuid: &str,
    session_id: &str,
    entrypoint: &str,
    workload: Option<&str>,
) {
    let billing_block = make_billing_block(body, entrypoint, workload);
    let prefix_block = make_prefix_block();

    let existing_blocks = normalise_system(body);

    let has_billing = existing_blocks.iter().any(is_billing_header_block);
    let has_prefix = existing_blocks.iter().any(is_prefix_block);

    let mut blocks = Vec::new();
    if has_billing {
        if let Some(b) = existing_blocks.iter().find(|b| is_billing_header_block(b)) {
            blocks.push(b.clone());
        }
    } else {
        blocks.push(billing_block);
    }
    if has_prefix {
        if let Some(b) = existing_blocks.iter().find(|b| is_prefix_block(b)) {
            blocks.push(b.clone());
        }
    } else {
        blocks.push(prefix_block);
    }
    for block in &existing_blocks {
        if !is_billing_header_block(block) && !is_prefix_block(block) {
            blocks.push(block.clone());
        }
    }
    body["system"] = serde_json::Value::Array(blocks);
    inject_metadata_user_id(body, device_id, account_uuid, session_id);
}

/// Extract the text of the first user message for fingerprint computation.
fn extract_first_user_message_text(body: &serde_json::Value) -> String {
    let Some(messages) = body.get("messages").and_then(|v| v.as_array()) else {
        return String::new();
    };
    let Some(first_user) = messages
        .iter()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
    else {
        return String::new();
    };
    match first_user.get("content") {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .find(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
            .and_then(|b| b.get("text").and_then(|t| t.as_str()))
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    }
}

/// Compute the 3-char fingerprint matching Claude Code's `utils/fingerprint.ts`.
///
/// Algorithm: `SHA256(SALT + msg[4] + msg[7] + msg[20] + version)[0..3]`
fn compute_fingerprint(message_text: &str, version: &str) -> String {
    let chars: Vec<char> = message_text.chars().collect();
    let indices = [4, 7, 20];
    let extracted: String = indices
        .iter()
        .map(|&i| chars.get(i).copied().unwrap_or('0'))
        .collect();
    let input = format!("{FINGERPRINT_SALT}{extracted}{version}");
    let hash = Sha256::digest(input.as_bytes());
    hex::encode(hash)[..3].to_string()
}

/// Generates the billing header content block using the real Claude Code
/// fingerprint algorithm.
///
/// `entrypoint` is appended as `cc_entrypoint=<entrypoint>;`.
/// `workload`, when `Some` and valid (`[A-Za-z0-9_-]+`), is appended as
/// `cc_workload=<workload>;` immediately after `cc_entrypoint`.
fn make_billing_block(
    body: &serde_json::Value,
    entrypoint: &str,
    workload: Option<&str>,
) -> serde_json::Value {
    let msg_text = extract_first_user_message_text(body);
    let fp = compute_fingerprint(&msg_text, DEFAULT_CLI_VERSION);

    let mut header = format!(
        "x-anthropic-billing-header: cc_version={DEFAULT_CLI_VERSION}.{fp}; cc_entrypoint={entrypoint};"
    );

    if let Some(wl) = workload
        && is_valid_workload(wl)
    {
        use std::fmt::Write as _;
        let _ = write!(header, " cc_workload={wl};");
    }

    serde_json::json!({
        "type": "text",
        "text": header
    })
}

/// Generates the Claude Code prefix block.
fn make_prefix_block() -> serde_json::Value {
    serde_json::json!({
        "type": "text",
        "text": "You are Claude Code, Anthropic's official CLI for Claude."
    })
}

/// Check if a system block is the billing header.
fn is_billing_header_block(block: &serde_json::Value) -> bool {
    block
        .get("text")
        .and_then(|t| t.as_str())
        .is_some_and(|s| s.contains("x-anthropic-billing-header"))
}

/// Check if a system block is the Claude Code prefix.
fn is_prefix_block(block: &serde_json::Value) -> bool {
    block
        .get("text")
        .and_then(|t| t.as_str())
        .is_some_and(|s| s.contains("You are Claude Code"))
}

/// Inject `metadata.user_id` matching Claude Code's identity format.
fn inject_metadata_user_id(
    body: &mut serde_json::Value,
    device_id: &str,
    account_uuid: &str,
    session_id: &str,
) {
    let user_id = serde_json::json!({
        "device_id": device_id,
        "account_uuid": account_uuid,
        "session_id": session_id,
    });
    if body.get("metadata").is_none() {
        body["metadata"] = serde_json::json!({});
    }
    body["metadata"]["user_id"] = serde_json::Value::String(user_id.to_string());
}

/// Normalises the `system` field to a `Vec` of content block values.
///
/// - If `system` is a string, converts it to `[{"type": "text", "text": "..."}]`.
/// - If `system` is already an array, returns the elements.
/// - Otherwise returns an empty vec.
fn normalise_system(body: &mut serde_json::Value) -> Vec<serde_json::Value> {
    match body.get("system") {
        Some(serde_json::Value::String(s)) => {
            let text = s.clone();
            vec![serde_json::json!({"type": "text", "text": text})]
        }
        Some(serde_json::Value::Array(arr)) => arr.clone(),
        _ => Vec::new(),
    }
}

// ── OAuth tool name remapping ────────────────────────────────────────────────

/// Tool name mapping: lowercase (client-side) → title case (sent to Anthropic).
///
/// Remapping tool names avoids third-party fingerprint detection when using
/// OAuth tokens. Only Claude Code built-in tools are mapped; custom tool names
/// pass through unchanged.
const TOOL_RENAME_MAP: &[(&str, &str)] = &[
    ("bash", "Bash"),
    ("read", "Read"),
    ("write", "Write"),
    ("edit", "Edit"),
    ("glob", "Glob"),
    ("grep", "Grep"),
    ("task", "Task"),
    ("webfetch", "WebFetch"),
    ("todowrite", "TodoWrite"),
    ("todoread", "TodoRead"),
    ("notebookedit", "NotebookEdit"),
    ("question", "Question"),
    ("skill", "Skill"),
    ("ls", "LS"),
];

fn forward_rename(name: &str) -> Option<&'static str> {
    TOOL_RENAME_MAP
        .iter()
        .find(|(k, _)| *k == name)
        .map(|(_, v)| *v)
}

fn reverse_rename(name: &str) -> Option<&'static str> {
    TOOL_RENAME_MAP
        .iter()
        .find(|(_, v)| *v == name)
        .map(|(k, _)| *k)
}

/// Renames known tool names in a Claude request body.
pub fn remap_tool_names_request(body: &mut serde_json::Value) {
    rename_in_value(body, forward_rename);
}

/// Reverses tool name remapping in a Claude response body.
pub fn reverse_remap_tool_names_response(body: &mut serde_json::Value) {
    rename_in_value(body, reverse_rename);
}

/// Applies a name-mapping function to tool names throughout a JSON value.
///
/// Covers `tools[].name`, `tool_choice.name`, `messages[].content[].name`
/// (where `type == "tool_use"`), and `content[].name` (response bodies).
fn rename_in_value(body: &mut serde_json::Value, map_fn: fn(&str) -> Option<&'static str>) {
    // tools[].name
    if let Some(tools) = body.get_mut("tools").and_then(|v| v.as_array_mut()) {
        for tool in tools {
            rename_field(tool, "name", map_fn);
        }
    }

    // tool_choice.name
    if let Some(tc) = body.get_mut("tool_choice") {
        rename_field(tc, "name", map_fn);
    }

    // messages[].content[] where type == "tool_use"
    if let Some(messages) = body.get_mut("messages").and_then(|v| v.as_array_mut()) {
        for msg in messages {
            rename_tool_use_blocks(msg.get_mut("content"), map_fn);
        }
    }

    // response content[] where type == "tool_use"
    rename_tool_use_blocks(body.get_mut("content"), map_fn);
}

fn rename_tool_use_blocks(
    content: Option<&mut serde_json::Value>,
    map_fn: fn(&str) -> Option<&'static str>,
) {
    let Some(arr) = content.and_then(|v| v.as_array_mut()) else {
        return;
    };
    for block in arr {
        if block.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
            rename_field(block, "name", map_fn);
        }
    }
}

fn rename_field(
    obj: &mut serde_json::Value,
    field: &str,
    map_fn: fn(&str) -> Option<&'static str>,
) {
    if let Some(name) = obj.get(field).and_then(|v| v.as_str())
        && let Some(mapped) = map_fn(name)
    {
        obj[field] = serde_json::Value::String(mapped.to_string());
    }
}

/// Reverses tool name remapping in a single SSE event (streaming response).
///
/// Looks for `content_block.name` and remaps it back to lowercase.
pub fn reverse_remap_tool_name_sse(event: &mut serde_json::Value) {
    if let Some(cb) = event.get_mut("content_block") {
        rename_field(cb, "name", reverse_rename);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_billing_header_format() {
        let body = serde_json::json!({
            "messages": [
                {"role": "user", "content": "Hello, world! This is a test message."}
            ]
        });
        let block = make_billing_block(&body, "cli", None);

        let text = block["text"].as_str().unwrap();
        assert!(text.starts_with(&format!(
            "x-anthropic-billing-header: cc_version={DEFAULT_CLI_VERSION}."
        )));
        assert!(text.contains("; cc_entrypoint=cli;"));
        // No cch field in the real format.
        assert!(!text.contains("cch="));

        // Verify fingerprint is deterministic and 3 chars.
        let version_prefix = format!("cc_version={DEFAULT_CLI_VERSION}.");
        let version_start = text.find(&version_prefix).unwrap() + version_prefix.len();
        let version_end = text[version_start..].find(';').unwrap() + version_start;
        let fp = &text[version_start..version_end];
        assert_eq!(fp.len(), 3);
        assert!(u16::from_str_radix(fp, 16).is_ok());

        // Same input → same fingerprint.
        let block2 = make_billing_block(&body, "cli", None);
        assert_eq!(block, block2);
    }

    #[test]
    fn test_billing_header_with_workload() {
        let body = serde_json::json!({"messages": [{"role": "user", "content": "hi"}]});
        let block = make_billing_block(&body, "cli", Some("agentic"));
        let text = block["text"].as_str().unwrap();
        assert!(text.contains("cc_entrypoint=cli;"));
        assert!(text.contains("cc_workload=agentic;"));
        // workload follows entrypoint
        let ep_pos = text.find("cc_entrypoint=cli;").unwrap();
        let wl_pos = text.find("cc_workload=agentic;").unwrap();
        assert!(wl_pos > ep_pos);
    }

    #[test]
    fn test_billing_header_invalid_workload_dropped() {
        let body = serde_json::json!({"messages": [{"role": "user", "content": "hi"}]});
        // Spaces and special chars are invalid.
        let block = make_billing_block(&body, "cli", Some("bad value!"));
        let text = block["text"].as_str().unwrap();
        assert!(!text.contains("cc_workload="));
    }

    #[test]
    fn test_billing_header_empty_workload_dropped() {
        let body = serde_json::json!({"messages": [{"role": "user", "content": "hi"}]});
        let block = make_billing_block(&body, "cli", Some(""));
        let text = block["text"].as_str().unwrap();
        assert!(!text.contains("cc_workload="));
    }

    #[test]
    fn test_derive_cc_entrypoint() {
        // None → cli
        assert_eq!(derive_cc_entrypoint(None), "cli");

        // claude-cli UA → cli
        assert_eq!(
            derive_cc_entrypoint(Some("claude-cli/2.1.109 (external, cli)")),
            "cli"
        );
        // claude-code UA (case-insensitive) → cli
        assert_eq!(derive_cc_entrypoint(Some("Claude-Code/1.0")), "cli");

        // VSCode UA → vscode
        assert_eq!(
            derive_cc_entrypoint(Some("vscode/1.107.0 (external)")),
            "vscode"
        );
        // Code/ UA shape → vscode
        assert_eq!(
            derive_cc_entrypoint(Some("GitHubCopilotChat/0.35.0 Code/1.107.0")),
            "vscode"
        );

        // Unknown UA → local-agent
        assert_eq!(derive_cc_entrypoint(Some("curl/7.88.0")), "local-agent");
        assert_eq!(
            derive_cc_entrypoint(Some("python-httpx/0.27.0")),
            "local-agent"
        );
    }

    #[test]
    fn test_fingerprint_algorithm() {
        // Verify the exact algorithm: SHA256(SALT + msg[4] + msg[7] + msg[20] + version)[0..3]
        let fp = compute_fingerprint("Hello, world! This is a test message.", DEFAULT_CLI_VERSION);
        assert_eq!(fp.len(), 3);
        assert!(u16::from_str_radix(&fp, 16).is_ok());

        // Short message — missing indices use '0'.
        let fp_short = compute_fingerprint("Hi", DEFAULT_CLI_VERSION);
        assert_eq!(fp_short.len(), 3);

        // Empty message — all '0's.
        let fp_empty = compute_fingerprint("", DEFAULT_CLI_VERSION);
        assert_eq!(fp_empty.len(), 3);
    }

    #[test]
    fn test_prefix_block() {
        let block = make_prefix_block();
        assert_eq!(block["type"], "text");
        assert_eq!(
            block["text"],
            "You are Claude Code, Anthropic's official CLI for Claude."
        );
    }

    #[test]
    fn injected_header_and_prefix_come_first_and_are_not_duplicated() {
        let mut body = serde_json::json!({
            "system": "You are a helpful assistant.",
            "messages": [{"role": "user", "content": "hi"}]
        });
        inject_billing_header(&mut body, "dev", "acct", "sess", "cli", None);
        let system = body["system"].as_array().unwrap();
        assert_eq!(system.len(), 3);
        assert!(is_billing_header_block(&system[0]));
        assert!(is_prefix_block(&system[1]));
        assert_eq!(system[2]["text"], "You are a helpful assistant.");
        let user_id = body["metadata"]["user_id"].as_str().unwrap();
        assert!(user_id.contains("dev") && user_id.contains("acct") && user_id.contains("sess"));

        // A client that already sends Claude Code's blocks keeps them.
        inject_billing_header(&mut body, "dev", "acct", "sess", "cli", None);
        let system = body["system"].as_array().unwrap();
        assert_eq!(system.len(), 3);
        assert_eq!(
            system.iter().filter(|b| is_billing_header_block(b)).count(),
            1
        );
    }

    #[test]
    fn test_remap_tool_names_request() {
        let mut body = serde_json::json!({
            "tools": [
                {"name": "bash", "description": "run bash"},
                {"name": "read", "description": "read file"},
                {"name": "custom_tool", "description": "custom"}
            ],
            "messages": [
                {"role": "assistant", "content": [
                    {"type": "tool_use", "name": "bash", "id": "t1", "input": {}}
                ]},
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "t1", "content": "ok"}
                ]}
            ],
            "tool_choice": {"type": "tool", "name": "read"}
        });
        remap_tool_names_request(&mut body);
        assert_eq!(body["tools"][0]["name"], "Bash");
        assert_eq!(body["tools"][1]["name"], "Read");
        assert_eq!(body["tools"][2]["name"], "custom_tool");
        assert_eq!(body["messages"][0]["content"][0]["name"], "Bash");
        assert_eq!(body["tool_choice"]["name"], "Read");
    }

    #[test]
    fn test_reverse_remap_tool_names() {
        let mut body = serde_json::json!({
            "content": [
                {"type": "tool_use", "name": "Bash", "id": "t1", "input": {}},
                {"type": "tool_use", "name": "Read", "id": "t2", "input": {}},
                {"type": "tool_use", "name": "CustomTool", "id": "t3", "input": {}},
                {"type": "text", "text": "hello"}
            ]
        });
        reverse_remap_tool_names_response(&mut body);
        assert_eq!(body["content"][0]["name"], "bash");
        assert_eq!(body["content"][1]["name"], "read");
        assert_eq!(body["content"][2]["name"], "CustomTool"); // unknown, untouched
    }
}
