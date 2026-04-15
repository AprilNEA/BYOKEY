//! Claude request cloaking — system prompt injection with billing header,
//! agent identifier, metadata injection, and sensitive word obfuscation.
//!
//! The fingerprint algorithm replicates Claude Code's `utils/fingerprint.ts`:
//! `SHA256(SALT + msg[4] + msg[7] + msg[20] + version)[0..3]`.

use byokey_config::CloakConfig;
use sha2::{Digest as _, Sha256};

/// Default CLI version for billing header and User-Agent.
const DEFAULT_CLI_VERSION: &str = "2.1.109";

/// Salt used by Claude Code's fingerprint.ts — must match the backend validator.
const FINGERPRINT_SALT: &str = "59cf53e54c78";

/// Applies cloaking transformations to a Claude API request body.
///
/// 1. Prepends a billing header block and a Claude Code prefix block to the
///    system prompt.
/// 2. Injects `metadata.user_id` with device/account/session identifiers.
/// 3. In strict mode, discards all user-supplied system blocks.
/// 4. Obfuscates sensitive words by inserting a zero-width space after the
///    first character of each occurrence.
pub fn apply_cloaking(
    body: &mut serde_json::Value,
    config: &CloakConfig,
    device_id: &str,
    account_uuid: &str,
    session_id: &str,
) {
    let billing_block = make_billing_block(body);
    let prefix_block = make_prefix_block();

    // Normalise `system` to an array of content blocks.
    let existing_blocks = normalise_system(body);

    // Detect if client already sent Claude Code-style system prompt.
    let has_billing = existing_blocks.iter().any(is_billing_header_block);
    let has_prefix = existing_blocks.iter().any(is_prefix_block);

    let mut system_blocks = Vec::new();

    // 1. Billing header (position 0).
    if has_billing {
        if let Some(b) = existing_blocks.iter().find(|b| is_billing_header_block(b)) {
            system_blocks.push(b.clone());
        }
    } else {
        system_blocks.push(billing_block);
    }

    // 2. Prefix block (position 1).
    if has_prefix {
        if let Some(b) = existing_blocks.iter().find(|b| is_prefix_block(b)) {
            system_blocks.push(b.clone());
        }
    } else {
        system_blocks.push(prefix_block);
    }

    // 3. Remaining user blocks (skip already-handled billing/prefix).
    if !config.strict_mode {
        for block in &existing_blocks {
            if !is_billing_header_block(block) && !is_prefix_block(block) {
                system_blocks.push(block.clone());
            }
        }
    }

    body["system"] = serde_json::Value::Array(system_blocks);

    // 4. Inject metadata.user_id.
    inject_metadata_user_id(body, device_id, account_uuid, session_id);

    // 5. Obfuscate sensitive words.
    if !config.sensitive_words.is_empty() {
        obfuscate_sensitive_words(body, &config.sensitive_words);
    }
}

/// Injects the billing header and Claude Code prefix into a request body's
/// `system` field without any other cloaking (no sensitive-word obfuscation,
/// no strict mode). Used by the passthrough handler where OAuth tokens
/// require the billing header to access Sonnet/Opus models.
///
/// Optionally injects `metadata.user_id` when identifiers are provided.
pub fn inject_billing_header(
    body: &mut serde_json::Value,
    device_id: Option<&str>,
    account_uuid: Option<&str>,
    session_id: Option<&str>,
) {
    let billing_block = make_billing_block(body);
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

    if let (Some(d), Some(a), Some(s)) = (device_id, account_uuid, session_id) {
        inject_metadata_user_id(body, d, a, s);
    }
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
fn make_billing_block(body: &serde_json::Value) -> serde_json::Value {
    let msg_text = extract_first_user_message_text(body);
    let fp = compute_fingerprint(&msg_text, DEFAULT_CLI_VERSION);

    let header = format!(
        "x-anthropic-billing-header: cc_version={DEFAULT_CLI_VERSION}.{fp}; cc_entrypoint=cli;"
    );

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

/// Obfuscates sensitive words in system blocks and message content blocks
/// by inserting a zero-width space (`\u{200B}`) after the first character of
/// each case-insensitive match.
fn obfuscate_sensitive_words(body: &mut serde_json::Value, words: &[String]) {
    // Sort words by length descending so longer matches take precedence.
    let mut sorted_words: Vec<&str> = words.iter().map(String::as_str).collect();
    sorted_words.sort_by_key(|w| std::cmp::Reverse(w.len()));

    // Obfuscate in system blocks.
    if let Some(system) = body.get_mut("system").and_then(|v| v.as_array_mut()) {
        for block in system {
            obfuscate_text_block(block, &sorted_words);
        }
    }

    // Obfuscate in messages.
    if let Some(messages) = body.get_mut("messages").and_then(|v| v.as_array_mut()) {
        for msg in messages {
            obfuscate_message_content(msg, &sorted_words);
        }
    }
}

/// Obfuscates sensitive words in a single text block's `text` field.
fn obfuscate_text_block(block: &mut serde_json::Value, words: &[&str]) {
    if block.get("type").and_then(|v| v.as_str()) == Some("text")
        && let Some(text) = block.get("text").and_then(|v| v.as_str()).map(String::from)
    {
        let result = obfuscate_string(&text, words);
        block["text"] = serde_json::Value::String(result);
    }
}

/// Obfuscates sensitive words in message content (handles both string and array content).
fn obfuscate_message_content(msg: &mut serde_json::Value, words: &[&str]) {
    match msg.get("content") {
        Some(serde_json::Value::String(_)) => {
            if let Some(text) = msg
                .get("content")
                .and_then(|v| v.as_str())
                .map(String::from)
            {
                let result = obfuscate_string(&text, words);
                msg["content"] = serde_json::Value::String(result);
            }
        }
        Some(serde_json::Value::Array(_)) => {
            if let Some(blocks) = msg.get_mut("content").and_then(|v| v.as_array_mut()) {
                for block in blocks {
                    obfuscate_text_block(block, words);
                }
            }
        }
        _ => {}
    }
}

/// Performs case-insensitive replacement of each word in the input string,
/// inserting a zero-width space after the first character of the word.
///
/// Uses char-based iteration to avoid byte-boundary issues when ZWS
/// (a multi-byte UTF-8 character) has been inserted in a previous pass.
fn obfuscate_string(input: &str, words: &[&str]) -> String {
    let mut result = input.to_string();
    for &word in words {
        if word.is_empty() {
            continue;
        }
        let lower_word: Vec<char> = word.to_lowercase().chars().collect();
        let result_chars: Vec<char> = result.chars().collect();
        let lower_chars: Vec<char> = result.to_lowercase().chars().collect();
        let mut output = String::with_capacity(result.len());
        let mut i = 0;
        while i < result_chars.len() {
            if i + lower_word.len() <= lower_chars.len()
                && lower_chars[i..i + lower_word.len()] == lower_word
            {
                // First char from original text, then ZWS, then the remaining matched chars.
                output.push(result_chars[i]);
                output.push('\u{200B}');
                for &c in &result_chars[i + 1..i + lower_word.len()] {
                    output.push(c);
                }
                i += lower_word.len();
            } else {
                output.push(result_chars[i]);
                i += 1;
            }
        }
        result = output;
    }
    result
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
        let block = make_billing_block(&body);

        let text = block["text"].as_str().unwrap();
        assert!(text.starts_with(&format!(
            "x-anthropic-billing-header: cc_version={DEFAULT_CLI_VERSION}."
        )));
        assert!(text.contains("; cc_entrypoint=cli;"));
        assert!(text.ends_with(';'));
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
        let block2 = make_billing_block(&body);
        assert_eq!(block, block2);
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

    const TEST_DEVICE_ID: &str = "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234";
    const TEST_ACCOUNT_UUID: &str = "test-account-uuid";
    const TEST_SESSION_ID: &str = "test-session-id";

    #[test]
    fn test_non_strict_mode_preserves_user_system() {
        let config = CloakConfig {
            enabled: true,
            strict_mode: false,
            sensitive_words: vec![],
        };
        let mut body = serde_json::json!({
            "system": "You are a helpful assistant.",
            "messages": []
        });
        apply_cloaking(
            &mut body,
            &config,
            TEST_DEVICE_ID,
            TEST_ACCOUNT_UUID,
            TEST_SESSION_ID,
        );

        let system = body["system"].as_array().unwrap();
        assert_eq!(system.len(), 3);
        // First block: billing header
        assert!(
            system[0]["text"]
                .as_str()
                .unwrap()
                .contains("x-anthropic-billing-header")
        );
        // Second block: Claude Code prefix
        assert!(system[1]["text"].as_str().unwrap().contains("Claude Code"));
        // Third block: user's original system prompt
        assert_eq!(system[2]["text"], "You are a helpful assistant.");

        // metadata.user_id should be set
        let user_id = body["metadata"]["user_id"].as_str().unwrap();
        assert!(user_id.contains(TEST_DEVICE_ID));
    }

    #[test]
    fn test_non_strict_mode_with_array_system() {
        let config = CloakConfig {
            enabled: true,
            strict_mode: false,
            sensitive_words: vec![],
        };
        let mut body = serde_json::json!({
            "system": [
                {"type": "text", "text": "Block A"},
                {"type": "text", "text": "Block B"}
            ],
            "messages": []
        });
        apply_cloaking(
            &mut body,
            &config,
            TEST_DEVICE_ID,
            TEST_ACCOUNT_UUID,
            TEST_SESSION_ID,
        );

        let system = body["system"].as_array().unwrap();
        assert_eq!(system.len(), 4); // billing + prefix + 2 user blocks
        assert_eq!(system[2]["text"], "Block A");
        assert_eq!(system[3]["text"], "Block B");
    }

    #[test]
    fn test_strict_mode_discards_user_system() {
        let config = CloakConfig {
            enabled: true,
            strict_mode: true,
            sensitive_words: vec![],
        };
        let mut body = serde_json::json!({
            "system": "You are a helpful assistant.",
            "messages": []
        });
        apply_cloaking(
            &mut body,
            &config,
            TEST_DEVICE_ID,
            TEST_ACCOUNT_UUID,
            TEST_SESSION_ID,
        );

        let system = body["system"].as_array().unwrap();
        assert_eq!(system.len(), 2); // Only billing + prefix
        assert!(
            system[0]["text"]
                .as_str()
                .unwrap()
                .contains("x-anthropic-billing-header")
        );
        assert!(system[1]["text"].as_str().unwrap().contains("Claude Code"));
    }

    #[test]
    fn test_no_system_field() {
        let config = CloakConfig {
            enabled: true,
            strict_mode: false,
            sensitive_words: vec![],
        };
        let mut body = serde_json::json!({
            "messages": []
        });
        apply_cloaking(
            &mut body,
            &config,
            TEST_DEVICE_ID,
            TEST_ACCOUNT_UUID,
            TEST_SESSION_ID,
        );

        let system = body["system"].as_array().unwrap();
        assert_eq!(system.len(), 2); // billing + prefix
    }

    #[test]
    fn test_existing_billing_header_not_duplicated() {
        let config = CloakConfig {
            enabled: true,
            strict_mode: false,
            sensitive_words: vec![],
        };
        let mut body = serde_json::json!({
            "system": [
                {"type": "text", "text": "x-anthropic-billing-header: cc_version=2.1.88.abc; cc_entrypoint=cli;"},
                {"type": "text", "text": "You are Claude Code, Anthropic's official CLI for Claude."},
                {"type": "text", "text": "User instructions"}
            ],
            "messages": []
        });
        apply_cloaking(
            &mut body,
            &config,
            TEST_DEVICE_ID,
            TEST_ACCOUNT_UUID,
            TEST_SESSION_ID,
        );

        let system = body["system"].as_array().unwrap();
        // Should keep existing billing + prefix, plus user instructions.
        assert_eq!(system.len(), 3);
        let billing_count = system.iter().filter(|b| is_billing_header_block(b)).count();
        assert_eq!(billing_count, 1);
    }

    #[test]
    fn test_sensitive_word_obfuscation_basic() {
        let result = obfuscate_string("Contact anthropic for help.", &["anthropic"]);
        assert_eq!(result, "Contact a\u{200B}nthropic for help.");
    }

    #[test]
    fn test_sensitive_word_obfuscation_case_insensitive() {
        let result = obfuscate_string("Contact Anthropic today.", &["anthropic"]);
        assert_eq!(result, "Contact A\u{200B}nthropic today.");
    }

    #[test]
    fn test_sensitive_word_obfuscation_multiple_occurrences() {
        let result = obfuscate_string("anthropic and ANTHROPIC", &["anthropic"]);
        assert_eq!(result, "a\u{200B}nthropic and A\u{200B}NTHROPIC");
    }

    #[test]
    fn test_sensitive_word_obfuscation_in_system_and_messages() {
        let config = CloakConfig {
            enabled: true,
            strict_mode: false,
            sensitive_words: vec!["secret".to_string()],
        };
        let mut body = serde_json::json!({
            "system": [
                {"type": "text", "text": "This is a secret system prompt."}
            ],
            "messages": [
                {"role": "user", "content": "Tell me the secret."},
                {"role": "user", "content": [
                    {"type": "text", "text": "Another secret here."}
                ]}
            ]
        });
        apply_cloaking(
            &mut body,
            &config,
            TEST_DEVICE_ID,
            TEST_ACCOUNT_UUID,
            TEST_SESSION_ID,
        );

        // Check system block (index 2 because billing + prefix are prepended).
        let system = body["system"].as_array().unwrap();
        assert!(
            system[2]["text"]
                .as_str()
                .unwrap()
                .contains("s\u{200B}ecret")
        );

        // Check message with string content.
        let msg0_content = body["messages"][0]["content"].as_str().unwrap();
        assert!(msg0_content.contains("s\u{200B}ecret"));

        // Check message with array content.
        let msg1_block = &body["messages"][1]["content"][0];
        assert!(
            msg1_block["text"]
                .as_str()
                .unwrap()
                .contains("s\u{200B}ecret")
        );
    }

    #[test]
    fn test_sensitive_word_longer_match_first() {
        // "anthropic" should be matched as a whole, not "ant" first.
        let result = obfuscate_string("anthropic ant", &["anthropic", "ant"]);
        assert_eq!(result, "a\u{200B}nthropic a\u{200B}nt");
    }

    #[test]
    fn test_obfuscate_empty_words() {
        let result = obfuscate_string("hello world", &[""]);
        assert_eq!(result, "hello world");
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
