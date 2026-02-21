//! Merges adjacent same-role messages.
//!
//! Some providers (Gemini, Codex) reject consecutive messages with the same role.
//! This module provides a pure function to merge adjacent user/assistant messages.

use serde_json::{Value, json};

/// Merges adjacent messages with the same role.
///
/// Rules:
/// - `user` and `assistant` messages with the same role are merged if adjacent.
/// - `tool` and `function` role messages are never merged.
/// - String content is converted to a text block array before merging.
pub fn merge_adjacent_messages(messages: &[Value]) -> Vec<Value> {
    let mut result: Vec<Value> = Vec::new();

    for msg in messages {
        let role = msg.get("role").and_then(Value::as_str).unwrap_or("");

        // tool / function messages are never merged
        if role == "tool" || role == "function" {
            result.push(msg.clone());
            continue;
        }

        let should_merge = result
            .last()
            .and_then(|last| last.get("role").and_then(Value::as_str))
            .is_some_and(|last_role| last_role == role);

        if should_merge {
            let last = result.last_mut().unwrap();
            let existing = to_content_array(last.get("content").unwrap_or(&Value::Null));
            let incoming = to_content_array(msg.get("content").unwrap_or(&Value::Null));
            let merged: Vec<Value> = existing.into_iter().chain(incoming).collect();
            last["content"] = Value::Array(merged);
        } else {
            result.push(msg.clone());
        }
    }

    result
}

fn to_content_array(content: &Value) -> Vec<Value> {
    match content {
        Value::String(s) => vec![json!({"type": "text", "text": s})],
        Value::Array(arr) => arr.clone(),
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_no_merge_different_roles() {
        let msgs = vec![
            json!({"role": "user", "content": "Hello"}),
            json!({"role": "assistant", "content": "Hi"}),
        ];
        let result = merge_adjacent_messages(&msgs);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0]["content"], "Hello");
        assert_eq!(result[1]["content"], "Hi");
    }

    #[test]
    fn test_merge_adjacent_user() {
        let msgs = vec![
            json!({"role": "user", "content": "Hello"}),
            json!({"role": "user", "content": "World"}),
        ];
        let result = merge_adjacent_messages(&msgs);
        assert_eq!(result.len(), 1);
        let content = result[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["text"], "Hello");
        assert_eq!(content[1]["text"], "World");
    }

    #[test]
    fn test_no_merge_tool() {
        let msgs = vec![
            json!({"role": "tool", "content": "result1", "tool_call_id": "a"}),
            json!({"role": "tool", "content": "result2", "tool_call_id": "b"}),
        ];
        let result = merge_adjacent_messages(&msgs);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_empty() {
        let result = merge_adjacent_messages(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_string_content_merged() {
        let msgs = vec![
            json!({"role": "user", "content": "A"}),
            json!({"role": "user", "content": [{"type": "text", "text": "B"}]}),
        ];
        let result = merge_adjacent_messages(&msgs);
        assert_eq!(result.len(), 1);
        let content = result[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0], json!({"type": "text", "text": "A"}));
        assert_eq!(content[1], json!({"type": "text", "text": "B"}));
    }
}
