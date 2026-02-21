//! Translates Codex (`OpenAI` Responses API) responses into `OpenAI` chat completion format.
//!
//! The Codex Responses API returns a `response` object (extracted from the
//! `response.completed` SSE event). This translator converts it to the standard
//! `OpenAI` chat completion response format.

use byokey_types::{traits::Result, ResponseTranslator};
use serde_json::{json, Value};

/// Translator from Codex Responses API `response` object to `OpenAI` chat completion format.
pub struct CodexToOpenAI;

impl ResponseTranslator for CodexToOpenAI {
    /// Translates a Codex `response` object (the value of `response.completed.response`)
    /// into an `OpenAI` chat completion response.
    ///
    /// # Errors
    ///
    /// Currently infallible; returns a best-effort translation even for unexpected shapes.
    fn translate_response(&self, res: Value) -> Result<Value> {
        let output = res.get("output").and_then(Value::as_array);

        // Extract visible text from the first "message" output item.
        let text = output
            .and_then(|arr| {
                arr.iter()
                    .find(|item| item.get("type").and_then(Value::as_str) == Some("message"))
            })
            .and_then(|msg| msg.get("content").and_then(Value::as_array))
            .and_then(|parts| {
                parts
                    .iter()
                    .find(|p| p.get("type").and_then(Value::as_str) == Some("output_text"))
            })
            .and_then(|p| p.get("text").and_then(Value::as_str))
            .unwrap_or("");

        // Extract reasoning summary from the "reasoning" output item (o4-mini, o3).
        let reasoning = output
            .and_then(|arr| {
                arr.iter()
                    .find(|item| item.get("type").and_then(Value::as_str) == Some("reasoning"))
            })
            .and_then(|item| item.get("summary").and_then(Value::as_array))
            .and_then(|parts| {
                parts
                    .iter()
                    .find(|p| p.get("type").and_then(Value::as_str) == Some("summary_text"))
            })
            .and_then(|p| p.get("text").and_then(Value::as_str));

        let id = res
            .get("id")
            .and_then(Value::as_str)
            .map_or_else(|| "chatcmpl-codex".to_string(), |s| format!("chatcmpl-{s}"));

        let model = res
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();

        let input_tokens = res
            .pointer("/usage/input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let output_tokens = res
            .pointer("/usage/output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);

        // Collect function_call output items → OpenAI tool_calls
        let tool_calls: Vec<Value> = output
            .map(|arr| {
                arr.iter()
                    .filter(|item| {
                        item.get("type").and_then(Value::as_str) == Some("function_call")
                    })
                    .map(|item| {
                        let call_id = item.get("call_id").and_then(Value::as_str).unwrap_or("");
                        let name = item.get("name").and_then(Value::as_str).unwrap_or("");
                        let arguments = item.get("arguments").and_then(Value::as_str).unwrap_or("");
                        json!({
                            "id": call_id,
                            "type": "function",
                            "function": {"name": name, "arguments": arguments}
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let has_tool_calls = !tool_calls.is_empty();

        let mut message = if let Some(r) = reasoning {
            json!({"role": "assistant", "content": text, "reasoning_content": r})
        } else {
            json!({"role": "assistant", "content": text})
        };

        if has_tool_calls {
            message["tool_calls"] = json!(tool_calls);
            if text.is_empty() {
                message["content"] = Value::Null;
            }
        }

        let finish_reason = if has_tool_calls { "tool_calls" } else { "stop" };

        Ok(json!({
            "id": id,
            "object": "chat.completion",
            "model": model,
            "choices": [{
                "index": 0,
                "message": message,
                "finish_reason": finish_reason
            }],
            "usage": {
                "prompt_tokens": input_tokens,
                "completion_tokens": output_tokens,
                "total_tokens": input_tokens + output_tokens
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "id": "resp_abc123",
            "model": "o4-mini",
            "output": [{
                "id": "item_1",
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "Hello there!"}]
            }],
            "usage": {"input_tokens": 10, "output_tokens": 5, "total_tokens": 15}
        })
    }

    #[test]
    fn test_basic() {
        let out = CodexToOpenAI.translate_response(sample()).unwrap();
        assert_eq!(out["choices"][0]["message"]["content"], "Hello there!");
        assert_eq!(out["choices"][0]["finish_reason"], "stop");
    }

    #[test]
    fn test_id_prefixed() {
        let out = CodexToOpenAI.translate_response(sample()).unwrap();
        assert!(out["id"].as_str().unwrap().starts_with("chatcmpl-"));
    }

    #[test]
    fn test_usage() {
        let out = CodexToOpenAI.translate_response(sample()).unwrap();
        assert_eq!(out["usage"]["prompt_tokens"], 10);
        assert_eq!(out["usage"]["completion_tokens"], 5);
        assert_eq!(out["usage"]["total_tokens"], 15);
    }

    #[test]
    fn test_model_forwarded() {
        let out = CodexToOpenAI.translate_response(sample()).unwrap();
        assert_eq!(out["model"], "o4-mini");
    }

    #[test]
    fn test_empty_output() {
        let res = json!({"id": "resp_x", "model": "o4-mini", "output": []});
        let out = CodexToOpenAI.translate_response(res).unwrap();
        assert_eq!(out["choices"][0]["message"]["content"], "");
    }

    #[test]
    fn test_function_call_output() {
        let res = json!({
            "id": "resp_fc",
            "model": "o4-mini",
            "output": [
                {
                    "type": "function_call",
                    "id": "fc_abc",
                    "call_id": "call_1",
                    "name": "get_weather",
                    "arguments": "{\"city\":\"Tokyo\"}"
                }
            ],
            "usage": {"input_tokens": 10, "output_tokens": 5, "total_tokens": 15}
        });
        let out = CodexToOpenAI.translate_response(res).unwrap();
        let choice = &out["choices"][0];
        assert_eq!(choice["finish_reason"], "tool_calls");
        assert!(choice["message"]["content"].is_null());
        let tool_calls = choice["message"]["tool_calls"].as_array().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0]["id"], "call_1");
        assert_eq!(tool_calls[0]["type"], "function");
        assert_eq!(tool_calls[0]["function"]["name"], "get_weather");
        assert_eq!(
            tool_calls[0]["function"]["arguments"],
            "{\"city\":\"Tokyo\"}"
        );
    }
}
