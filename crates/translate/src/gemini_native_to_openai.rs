//! Translates Gemini native `generateContent` requests into `OpenAI` chat completion format.
//!
//! This is the reverse of [`super::openai_to_gemini`]: given a request already in
//! Google's native format we produce an `OpenAI`-compatible request so it can be
//! forwarded to an `OpenAI`-compatible backend (e.g. GitHub Copilot).
//!
//! # Usage
//!
//! ```ignore
//! use byokey_translate::GeminiNativeRequest;
//!
//! let openai_req: Value = GeminiNativeRequest { body: &body, model: "gemini-2.5-pro" }
//!     .try_into()?;
//! ```

use byokey_types::ByokError;
use serde_json::{Value, json};

/// A Gemini native `generateContent` request paired with the target model name.
///
/// Implements `TryFrom` → [`Value`] to produce an `OpenAI`-compatible chat
/// completion request.
pub struct GeminiNativeRequest<'a> {
    pub body: &'a Value,
    pub model: &'a str,
}

impl TryFrom<GeminiNativeRequest<'_>> for Value {
    type Error = ByokError;

    #[allow(clippy::too_many_lines)]
    fn try_from(req: GeminiNativeRequest<'_>) -> std::result::Result<Self, Self::Error> {
        let contents = req
            .body
            .get("contents")
            .and_then(Value::as_array)
            .ok_or_else(|| ByokError::Translation("missing 'contents'".into()))?;

        let mut messages: Vec<Value> = Vec::new();

        // systemInstruction → system message
        if let Some(instruction) = req.body.get("systemInstruction") {
            let text = extract_text_from_parts(instruction);
            if !text.is_empty() {
                messages.push(json!({"role": "system", "content": text}));
            }
        }

        // contents → messages
        for content in contents {
            translate_content(content, &mut messages);
        }

        let mut out = json!({
            "model": req.model,
            "messages": messages,
        });

        // generationConfig → max_tokens, temperature
        if let Some(gen_config) = req.body.get("generationConfig") {
            if let Some(max_tokens) = gen_config.get("maxOutputTokens").and_then(Value::as_u64) {
                out["max_tokens"] = json!(max_tokens);
            }
            if let Some(temp) = gen_config.get("temperature") {
                out["temperature"] = temp.clone();
            }
        }

        // tools → OpenAI tools format
        if let Some(tools) = req.body.get("tools").and_then(Value::as_array) {
            let openai_tools: Vec<Value> = tools
                .iter()
                .filter_map(|t| t.get("functionDeclarations").and_then(Value::as_array))
                .flatten()
                .map(|decl| {
                    let mut func = json!({"name": decl.get("name").unwrap_or(&Value::Null)});
                    if let Some(desc) = decl.get("description") {
                        func["description"] = desc.clone();
                    }
                    if let Some(params) = decl.get("parameters") {
                        func["parameters"] = params.clone();
                    }
                    json!({"type": "function", "function": func})
                })
                .collect();
            if !openai_tools.is_empty() {
                out["tools"] = json!(openai_tools);
            }
        }

        // toolConfig → tool_choice
        if let Some(mode) = req
            .body
            .pointer("/toolConfig/functionCallingConfig/mode")
            .and_then(Value::as_str)
        {
            let tool_choice = match mode {
                "NONE" => json!("none"),
                "ANY" => {
                    if let Some(names) = req
                        .body
                        .pointer("/toolConfig/functionCallingConfig/allowedFunctionNames")
                        .and_then(Value::as_array)
                        && let Some(first) = names.first().and_then(Value::as_str)
                    {
                        json!({"type": "function", "function": {"name": first}})
                    } else {
                        json!("auto")
                    }
                }
                // "AUTO" and anything else
                _ => json!("auto"),
            };
            out["tool_choice"] = tool_choice;
        }

        Ok(out)
    }
}

/// Translate a single Gemini `content` object into one or more `OpenAI` messages.
fn translate_content(content: &Value, messages: &mut Vec<Value>) {
    let role = content
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("user");
    let parts = content.get("parts").and_then(Value::as_array);

    let Some(parts) = parts else {
        return;
    };

    // Check for functionCall parts → assistant with tool_calls
    let function_calls: Vec<&Value> = parts
        .iter()
        .filter(|p| p.get("functionCall").is_some())
        .collect();

    if !function_calls.is_empty() {
        let text_parts: String = parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("");

        let tool_calls: Vec<Value> = function_calls
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let fc = &p["functionCall"];
                let name = fc.get("name").and_then(Value::as_str).unwrap_or("");
                let args = fc.get("args").cloned().unwrap_or(json!({}));
                json!({
                    "id": format!("call_{i}"),
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": serde_json::to_string(&args).unwrap_or_default()
                    }
                })
            })
            .collect();

        let mut msg = json!({
            "role": "assistant",
            "tool_calls": tool_calls
        });
        if !text_parts.is_empty() {
            msg["content"] = json!(text_parts);
        }
        messages.push(msg);
        return;
    }

    // Check for functionResponse parts → tool messages
    let function_responses: Vec<&Value> = parts
        .iter()
        .filter(|p| p.get("functionResponse").is_some())
        .collect();

    if !function_responses.is_empty() {
        for p in function_responses {
            let fr = &p["functionResponse"];
            let name = fr.get("name").and_then(Value::as_str).unwrap_or("");
            let response = fr
                .get("response")
                .and_then(|r| r.get("result"))
                .and_then(Value::as_str)
                .unwrap_or("");
            messages.push(json!({
                "role": "tool",
                "tool_call_id": format!("{name}-0"),
                "content": response
            }));
        }
        return;
    }

    // Normal text content
    let openai_role = if role == "model" { "assistant" } else { "user" };
    let text = parts
        .iter()
        .filter_map(|p| p.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("");

    messages.push(json!({"role": openai_role, "content": text}));
}

/// Extract concatenated text from a Gemini `parts` container (e.g. `systemInstruction`).
fn extract_text_from_parts(container: &Value) -> String {
    container
        .get("parts")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_basic_contents() {
        let req = json!({
            "contents": [{"role": "user", "parts": [{"text": "Hello"}]}]
        });
        let out: Value = GeminiNativeRequest {
            body: &req,
            model: "gemini-2.5-pro",
        }
        .try_into()
        .unwrap();
        assert_eq!(out["model"], "gemini-2.5-pro");
        assert_eq!(out["messages"][0]["role"], "user");
        assert_eq!(out["messages"][0]["content"], "Hello");
    }

    #[test]
    fn test_model_becomes_assistant() {
        let req = json!({
            "contents": [
                {"role": "user", "parts": [{"text": "Hi"}]},
                {"role": "model", "parts": [{"text": "Hello!"}]}
            ]
        });
        let out: Value = GeminiNativeRequest {
            body: &req,
            model: "gemini-2.5-pro",
        }
        .try_into()
        .unwrap();
        assert_eq!(out["messages"][0]["role"], "user");
        assert_eq!(out["messages"][1]["role"], "assistant");
        assert_eq!(out["messages"][1]["content"], "Hello!");
    }

    #[test]
    fn test_system_instruction() {
        let req = json!({
            "systemInstruction": {"parts": [{"text": "Be concise."}]},
            "contents": [{"role": "user", "parts": [{"text": "Hi"}]}]
        });
        let out: Value = GeminiNativeRequest {
            body: &req,
            model: "gemini-2.5-pro",
        }
        .try_into()
        .unwrap();
        assert_eq!(out["messages"][0]["role"], "system");
        assert_eq!(out["messages"][0]["content"], "Be concise.");
        assert_eq!(out["messages"][1]["role"], "user");
    }

    #[test]
    fn test_generation_config() {
        let req = json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}],
            "generationConfig": {"maxOutputTokens": 512, "temperature": 0.7}
        });
        let out: Value = GeminiNativeRequest {
            body: &req,
            model: "gemini-2.5-pro",
        }
        .try_into()
        .unwrap();
        assert_eq!(out["max_tokens"], 512);
        assert_eq!(out["temperature"], 0.7);
    }

    #[test]
    fn test_tools_translation() {
        let req = json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}],
            "tools": [{"functionDeclarations": [{
                "name": "get_weather",
                "description": "Get the weather",
                "parameters": {"type": "object", "properties": {"location": {"type": "string"}}}
            }]}]
        });
        let out: Value = GeminiNativeRequest {
            body: &req,
            model: "gemini-2.5-pro",
        }
        .try_into()
        .unwrap();
        assert_eq!(out["tools"][0]["type"], "function");
        assert_eq!(out["tools"][0]["function"]["name"], "get_weather");
    }

    #[test]
    fn test_tool_config_auto() {
        let req = json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}],
            "toolConfig": {"functionCallingConfig": {"mode": "AUTO"}}
        });
        let out: Value = GeminiNativeRequest {
            body: &req,
            model: "gemini-2.5-pro",
        }
        .try_into()
        .unwrap();
        assert_eq!(out["tool_choice"], "auto");
    }

    #[test]
    fn test_tool_config_none() {
        let req = json!({
            "contents": [{"role": "user", "parts": [{"text": "hi"}]}],
            "toolConfig": {"functionCallingConfig": {"mode": "NONE"}}
        });
        let out: Value = GeminiNativeRequest {
            body: &req,
            model: "gemini-2.5-pro",
        }
        .try_into()
        .unwrap();
        assert_eq!(out["tool_choice"], "none");
    }

    #[test]
    fn test_function_call_parts() {
        let req = json!({
            "contents": [
                {"role": "user", "parts": [{"text": "weather?"}]},
                {"role": "model", "parts": [{"functionCall": {"name": "get_weather", "args": {"location": "NYC"}}}]}
            ]
        });
        let out: Value = GeminiNativeRequest {
            body: &req,
            model: "gemini-2.5-pro",
        }
        .try_into()
        .unwrap();
        let msg = &out["messages"][1];
        assert_eq!(msg["role"], "assistant");
        assert_eq!(msg["tool_calls"][0]["function"]["name"], "get_weather");
    }

    #[test]
    fn test_function_response_parts() {
        let req = json!({
            "contents": [
                {"role": "user", "parts": [{"functionResponse": {"name": "get_weather", "response": {"result": "72°F"}}}]}
            ]
        });
        let out: Value = GeminiNativeRequest {
            body: &req,
            model: "gemini-2.5-pro",
        }
        .try_into()
        .unwrap();
        assert_eq!(out["messages"][0]["role"], "tool");
        assert_eq!(out["messages"][0]["content"], "72°F");
    }

    #[test]
    fn test_missing_contents_error() {
        let req = json!({"systemInstruction": {"parts": [{"text": "hi"}]}});
        let result: std::result::Result<Value, _> = GeminiNativeRequest {
            body: &req,
            model: "test",
        }
        .try_into();
        assert!(result.is_err());
    }
}
