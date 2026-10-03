//! Stable output-item identities for Copilot's per-event opaque IDs.

use std::collections::BTreeMap;

use serde_json::Value;

/// Retain the first upstream item ID for each output index in one response.
#[derive(Default)]
pub(super) struct ItemIds(BTreeMap<u64, String>);

impl ItemIds {
    /// Normalize item references without changing response IDs or tool-call IDs.
    pub(super) fn normalize(&mut self, event: &mut Value) -> bool {
        let mut changed = false;
        if let Some(index) = event.get("output_index").and_then(Value::as_u64) {
            let first_id = event
                .get("item")
                .and_then(|item| item.get("id"))
                .or_else(|| event.get("item_id"))
                .and_then(Value::as_str);
            if let Some(first_id) = first_id {
                let id = self.0.entry(index).or_insert_with(|| first_id.to_owned());
                changed |= replace(event.get_mut("item_id"), id);
                changed |= replace(
                    event.get_mut("item").and_then(|item| item.get_mut("id")),
                    id,
                );
            }
        }
        if let Some(output) = event
            .get_mut("response")
            .and_then(|response| response.get_mut("output"))
            .and_then(Value::as_array_mut)
        {
            for (index, item) in (0..).zip(output) {
                if let Some(id) = self.0.get(&index) {
                    changed |= replace(item.get_mut("id"), id);
                }
            }
        }
        changed
    }
}

fn replace(value: Option<&mut Value>, id: &str) -> bool {
    let Some(value) = value else { return false };
    if value.as_str() == Some(id) {
        return false;
    }
    *value = Value::String(id.to_owned());
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deltas_keep_their_output_identity_after_another_item_starts() {
        let mut ids = ItemIds::default();
        ids.normalize(&mut json!({"output_index": 0, "item": {"id": "message-first"}}));
        ids.normalize(&mut json!({"output_index": 1, "item": {"id": "tool-first"}}));
        let mut delta = json!({
            "type": "response.output_text.delta", "output_index": 0,
            "item_id": "message-later", "delta": "你好", "future_field": 42,
        });

        ids.normalize(&mut delta);

        assert_eq!(
            delta,
            json!({
                "type": "response.output_text.delta", "output_index": 0,
                "item_id": "message-first", "delta": "你好", "future_field": 42,
            })
        );
    }

    #[test]
    fn terminal_output_preserves_distinct_messages_and_tool_call_ids() {
        let mut ids = ItemIds::default();
        ids.normalize(&mut json!({"output_index": 0, "item": {"id": "message-first"}}));
        ids.normalize(&mut json!({"output_index": 2, "item": {"id": "message-second"}}));
        ids.normalize(&mut json!({"output_index": 1, "item": {"id": "tool-first"}}));
        let mut completed = json!({
            "type": "response.completed",
            "response": {"id": "response-final", "previous_response_id": "response-previous", "output": [
                {"id": "message-final", "type": "message", "text": "Repeated text"},
                {"id": "tool-final", "type": "function_call", "call_id": "opaque-call-id", "name": "lookup", "arguments": "{\"city\":\"Paris\"}"},
                {"id": "message-second-final", "type": "message", "text": "Repeated text"},
            ]},
        });

        ids.normalize(&mut completed);

        assert_eq!(
            completed,
            json!({
                "type": "response.completed",
                "response": {"id": "response-final", "previous_response_id": "response-previous", "output": [
                    {"id": "message-first", "type": "message", "text": "Repeated text"},
                    {"id": "tool-first", "type": "function_call", "call_id": "opaque-call-id", "name": "lookup", "arguments": "{\"city\":\"Paris\"}"},
                    {"id": "message-second", "type": "message", "text": "Repeated text"},
                ]},
            })
        );
    }
}
