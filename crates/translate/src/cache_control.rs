//! Automatic cache_control injection for Claude Messages API requests.

use serde_json::{Value, json};

/// Injects `cache_control: {type: "ephemeral"}` into a Claude request body
/// at up to three positions for optimal prompt caching:
/// 1. The last tool definition
/// 2. The last system content block
/// 3. The second-to-last user message
///
/// Positions that already have cache_control are skipped.
pub fn inject_cache_control(mut request: Value) -> Value {
    inject_tools_cache(&mut request);
    inject_system_cache(&mut request);
    inject_messages_cache(&mut request);
    request
}

fn inject_tools_cache(req: &mut Value) {
    if let Some(tools) = req.get_mut("tools").and_then(Value::as_array_mut) {
        if let Some(last) = tools.last_mut() {
            if last.get("cache_control").is_none() {
                last["cache_control"] = json!({"type": "ephemeral"});
            }
        }
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
            if let Some(arr) = system.as_array_mut() {
                if let Some(last) = arr.last_mut() {
                    if last.get("cache_control").is_none() {
                        last["cache_control"] = cache;
                    }
                }
            }
        }
        _ => {}
    }
}

fn inject_messages_cache(req: &mut Value) {
    let messages = match req.get_mut("messages").and_then(Value::as_array_mut) {
        Some(m) => m,
        None => return,
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

    if let Some(content) = msg.get_mut("content").and_then(Value::as_array_mut) {
        if let Some(last) = content.last_mut() {
            if last.get("cache_control").is_none() {
                last["cache_control"] = json!({"type": "ephemeral"});
            }
        }
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
        assert_eq!(
            content[0]["cache_control"],
            json!({"type": "ephemeral"})
        );
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
        assert_eq!(
            system[0]["cache_control"],
            json!({"type": "ephemeral"})
        );
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
        assert_eq!(
            system[1]["cache_control"],
            json!({"type": "ephemeral"})
        );
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
        assert_eq!(result["tools"][0]["cache_control"], json!({"type": "ephemeral"}));
        assert_eq!(result["system"][0]["cache_control"], json!({"type": "ephemeral"}));
        assert_eq!(
            result["messages"][0]["content"][0]["cache_control"],
            json!({"type": "ephemeral"})
        );
    }
}
