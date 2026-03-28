//! Automatic `cache_control` injection for Claude Messages API requests.

use serde_json::{Value, json};

/// Injects `cache_control: {type: "ephemeral"}` into a Claude request body
/// at up to three positions for optimal prompt caching:
/// 1. The last tool definition
/// 2. The last system content block
/// 3. The second-to-last user message
///
/// Positions that already have `cache_control` are skipped.
#[must_use]
pub fn inject_cache_control(mut request: Value) -> Value {
    let had_cache = count_cache_controls(&request) > 0;
    if !had_cache {
        inject_tools_cache(&mut request);
        inject_system_cache(&mut request);
        inject_messages_cache(&mut request);
    }
    // Enforce Anthropic's cache_control block limit (max 4 breakpoints per request).
    enforce_cache_control_limit(&mut request, 4);
    // Normalize TTL values for prompt-caching-scope-2026-01-05:
    // a 1h-TTL block must not appear after a 5m-TTL block in evaluation order.
    normalize_cache_control_ttl(&mut request);
    request
}

fn inject_tools_cache(req: &mut Value) {
    if let Some(tools) = req.get_mut("tools").and_then(Value::as_array_mut)
        && let Some(last) = tools.last_mut()
        && last.get("cache_control").is_none()
    {
        last["cache_control"] = json!({"type": "ephemeral"});
    }
}

fn inject_system_cache(req: &mut Value) {
    let cache = json!({"type": "ephemeral"});

    match req.get_mut("system") {
        Some(system) if system.is_string() => {
            let text = system.as_str().unwrap_or_default().to_owned();
            *system = json!([{
                "type": "text",
                "text": text,
                "cache_control": cache,
            }]);
        }
        Some(system) if system.is_array() => {
            if let Some(arr) = system.as_array_mut()
                && let Some(last) = arr.last_mut()
                && last.get("cache_control").is_none()
            {
                last["cache_control"] = cache;
            }
        }
        _ => {}
    }
}

fn inject_messages_cache(req: &mut Value) {
    let Some(messages) = req.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };

    let user_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.get("role").and_then(Value::as_str) == Some("user"))
        .map(|(i, _)| i)
        .collect();

    if user_indices.len() < 2 {
        return;
    }

    let target_idx = user_indices[user_indices.len() - 2];
    let msg = &mut messages[target_idx];

    // Normalize content to array if it's a string
    if let Some(text) = msg.get("content").and_then(Value::as_str).map(String::from) {
        msg["content"] = json!([{"type": "text", "text": text}]);
    }

    if let Some(content) = msg.get_mut("content").and_then(Value::as_array_mut)
        && let Some(last) = content.last_mut()
        && last.get("cache_control").is_none()
    {
        last["cache_control"] = json!({"type": "ephemeral"});
    }
}

/// Count total `cache_control` blocks across tools, system, and message content.
fn count_cache_controls(req: &Value) -> usize {
    let mut count = 0;
    if let Some(tools) = req.get("tools").and_then(Value::as_array) {
        count += tools
            .iter()
            .filter(|t| t.get("cache_control").is_some())
            .count();
    }
    if let Some(system) = req.get("system").and_then(Value::as_array) {
        count += system
            .iter()
            .filter(|s| s.get("cache_control").is_some())
            .count();
    }
    if let Some(msgs) = req.get("messages").and_then(Value::as_array) {
        for msg in msgs {
            if let Some(content) = msg.get("content").and_then(Value::as_array) {
                count += content
                    .iter()
                    .filter(|c| c.get("cache_control").is_some())
                    .count();
            }
            // String content with cache_control at message level
            if msg.get("cache_control").is_some() {
                count += 1;
            }
        }
    }
    count
}

/// Enforce Anthropic's max `cache_control` breakpoints (currently 4).
///
/// Removal priority (strip lowest-value first):
/// 1. System blocks earliest-first (preserve last)
/// 2. Tool blocks earliest-first (preserve last)
/// 3. Message content blocks earliest-first
fn enforce_cache_control_limit(req: &mut Value, max_blocks: usize) {
    let total = count_cache_controls(req);
    if total <= max_blocks {
        return;
    }
    let mut excess = total - max_blocks;

    // Phase 1: strip system blocks (earliest first, preserve last)
    if let Some(system) = req.get_mut("system").and_then(Value::as_array_mut) {
        let last_cc = system
            .iter()
            .rposition(|s| s.get("cache_control").is_some());
        for (i, item) in system.iter_mut().enumerate() {
            if excess == 0 {
                break;
            }
            if Some(i) != last_cc && item.get("cache_control").is_some() {
                item.as_object_mut().unwrap().remove("cache_control");
                excess -= 1;
            }
        }
    }
    if excess == 0 {
        return;
    }

    // Phase 2: strip tool blocks (earliest first, preserve last)
    if let Some(tools) = req.get_mut("tools").and_then(Value::as_array_mut) {
        let last_cc = tools.iter().rposition(|t| t.get("cache_control").is_some());
        for (i, item) in tools.iter_mut().enumerate() {
            if excess == 0 {
                break;
            }
            if Some(i) != last_cc && item.get("cache_control").is_some() {
                item.as_object_mut().unwrap().remove("cache_control");
                excess -= 1;
            }
        }
    }
    if excess == 0 {
        return;
    }

    // Phase 3: strip message content blocks (earliest first)
    if let Some(msgs) = req.get_mut("messages").and_then(Value::as_array_mut) {
        for msg in msgs.iter_mut() {
            if excess == 0 {
                break;
            }
            if let Some(content) = msg.get_mut("content").and_then(Value::as_array_mut) {
                for item in content.iter_mut() {
                    if excess == 0 {
                        break;
                    }
                    if item.get("cache_control").is_some() {
                        item.as_object_mut().unwrap().remove("cache_control");
                        excess -= 1;
                    }
                }
            }
        }
    }
}

/// Normalize `cache_control` TTL values for prompt-caching-scope-2026-01-05.
///
/// Evaluation order: tools → system → messages.
/// Once a 5m (300s / default) TTL is seen, all subsequent blocks with higher
/// TTL (e.g. 3600s / 1h) must be downgraded by removing the `ttl` field
/// (which defaults to 5m).
fn normalize_cache_control_ttl(req: &mut Value) {
    let mut seen_short = false;
    let mut modified = false;

    // Walk tools → system → message content in evaluation order
    let sections: Vec<&str> = vec!["tools", "system"];
    for section in &sections {
        if let Some(arr) = req.get_mut(section).and_then(Value::as_array_mut) {
            for item in arr.iter_mut() {
                if normalize_ttl_block(item, &mut seen_short) {
                    modified = true;
                }
            }
        }
    }

    if let Some(msgs) = req.get_mut("messages").and_then(Value::as_array_mut) {
        for msg in msgs.iter_mut() {
            if let Some(content) = msg.get_mut("content").and_then(Value::as_array_mut) {
                for item in content.iter_mut() {
                    if normalize_ttl_block(item, &mut seen_short) {
                        modified = true;
                    }
                }
            }
        }
    }

    let _ = modified; // suppress unused warning
}

/// Check a single block's `cache_control.ttl`. Returns true if modified.
fn normalize_ttl_block(item: &mut Value, seen_short: &mut bool) -> bool {
    let Some(cc) = item.get("cache_control") else {
        return false;
    };
    let ttl = cc.get("ttl").and_then(Value::as_u64);
    match ttl {
        // No TTL or TTL ≤ 300 → this is a "short" (5m/default) block
        None | Some(0..=300) => {
            *seen_short = true;
            false
        }
        // TTL > 300 (e.g. 3600 = 1h) → downgrade if we've seen a short block
        Some(_) if *seen_short => {
            if let Some(cc_obj) = item.get_mut("cache_control").and_then(Value::as_object_mut) {
                cc_obj.remove("ttl");
            }
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_user_messages_no_inject() {
        let req = json!({
            "messages": [
                {"role": "user", "content": "hello"}
            ]
        });
        let result = inject_cache_control(req);
        // Only one user message — should NOT inject cache_control on messages
        let msg = &result["messages"][0];
        assert!(msg["content"].is_string(), "content should remain a string");
    }

    #[test]
    fn test_two_user_messages_inject_first() {
        let req = json!({
            "messages": [
                {"role": "user", "content": "first"},
                {"role": "assistant", "content": "reply"},
                {"role": "user", "content": "second"}
            ]
        });
        let result = inject_cache_control(req);
        // First user message should have cache_control injected
        let first_user = &result["messages"][0];
        let content = first_user["content"].as_array().expect("should be array");
        assert_eq!(content[0]["cache_control"], json!({"type": "ephemeral"}));
        // Second user message should be untouched
        let second_user = &result["messages"][2];
        assert!(second_user["content"].is_string());
    }

    #[test]
    fn test_tools_cache_injected() {
        let req = json!({
            "tools": [
                {"name": "tool_a"},
                {"name": "tool_b"}
            ],
            "messages": []
        });
        let result = inject_cache_control(req);
        assert_eq!(
            result["tools"][1]["cache_control"],
            json!({"type": "ephemeral"})
        );
        // First tool should be untouched
        assert!(result["tools"][0].get("cache_control").is_none());
    }

    #[test]
    fn test_system_string_converted_to_array() {
        let req = json!({
            "system": "You are helpful.",
            "messages": []
        });
        let result = inject_cache_control(req);
        let system = result["system"].as_array().expect("should be array");
        assert_eq!(system.len(), 1);
        assert_eq!(system[0]["type"], "text");
        assert_eq!(system[0]["text"], "You are helpful.");
        assert_eq!(system[0]["cache_control"], json!({"type": "ephemeral"}));
    }

    #[test]
    fn test_system_array_last_item_injected() {
        let req = json!({
            "system": [
                {"type": "text", "text": "first"},
                {"type": "text", "text": "second"}
            ],
            "messages": []
        });
        let result = inject_cache_control(req);
        let system = result["system"].as_array().unwrap();
        assert!(system[0].get("cache_control").is_none());
        assert_eq!(system[1]["cache_control"], json!({"type": "ephemeral"}));
    }

    #[test]
    fn test_skip_if_already_has_cache_control() {
        let req = json!({
            "tools": [
                {"name": "t", "cache_control": {"type": "ephemeral"}}
            ],
            "system": [
                {"type": "text", "text": "sys", "cache_control": {"type": "ephemeral"}}
            ],
            "messages": [
                {"role": "user", "content": [
                    {"type": "text", "text": "hi", "cache_control": {"type": "ephemeral"}}
                ]},
                {"role": "assistant", "content": "ok"},
                {"role": "user", "content": "bye"}
            ]
        });
        let result = inject_cache_control(req.clone());
        // Nothing should be modified — all already have cache_control
        assert_eq!(
            result["tools"][0]["cache_control"],
            json!({"type": "ephemeral"})
        );
        assert_eq!(
            result["system"][0]["cache_control"],
            json!({"type": "ephemeral"})
        );
        assert_eq!(
            result["messages"][0]["content"][0]["cache_control"],
            json!({"type": "ephemeral"})
        );
    }

    #[test]
    fn test_enforce_limit_strips_excess() {
        let req = json!({
            "tools": [
                {"name": "a", "cache_control": {"type": "ephemeral"}},
                {"name": "b", "cache_control": {"type": "ephemeral"}},
                {"name": "c", "cache_control": {"type": "ephemeral"}}
            ],
            "system": [
                {"type": "text", "text": "s1", "cache_control": {"type": "ephemeral"}},
                {"type": "text", "text": "s2", "cache_control": {"type": "ephemeral"}}
            ],
            "messages": []
        });
        // 5 cache_control blocks → should be reduced to 4
        let result = inject_cache_control(req);
        assert_eq!(count_cache_controls(&result), 4);
    }

    #[test]
    fn test_enforce_limit_under_limit_no_change() {
        let req = json!({
            "tools": [
                {"name": "a", "cache_control": {"type": "ephemeral"}}
            ],
            "system": [
                {"type": "text", "text": "s1", "cache_control": {"type": "ephemeral"}}
            ],
            "messages": []
        });
        let result = inject_cache_control(req);
        assert_eq!(count_cache_controls(&result), 2);
    }

    #[test]
    fn test_normalize_ttl_downgrades_after_short() {
        let mut req = json!({
            "tools": [
                {"name": "a", "cache_control": {"type": "ephemeral"}},
                {"name": "b", "cache_control": {"type": "ephemeral", "ttl": 3600}}
            ],
            "system": [],
            "messages": []
        });
        normalize_cache_control_ttl(&mut req);
        // First tool has no TTL (5m default) → short seen
        // Second tool had TTL 3600 → should be stripped
        assert!(req["tools"][1]["cache_control"].get("ttl").is_none());
    }

    #[test]
    fn test_normalize_ttl_no_downgrade_if_no_short() {
        let mut req = json!({
            "tools": [
                {"name": "a", "cache_control": {"type": "ephemeral", "ttl": 3600}},
                {"name": "b", "cache_control": {"type": "ephemeral", "ttl": 3600}}
            ],
            "system": [],
            "messages": []
        });
        normalize_cache_control_ttl(&mut req);
        // No short TTL seen → both keep their 3600
        assert_eq!(req["tools"][0]["cache_control"]["ttl"], 3600);
        assert_eq!(req["tools"][1]["cache_control"]["ttl"], 3600);
    }
}
