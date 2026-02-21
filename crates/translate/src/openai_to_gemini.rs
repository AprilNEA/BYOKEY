//! Translates `OpenAI` chat completion requests into Gemini `generateContent` format.

use byokey_types::{ByokError, RequestTranslator, traits::Result};
use serde_json::{Value, json};

use crate::merge_messages::merge_adjacent_messages;

/// Translator from `OpenAI` chat completion request format to Gemini `generateContent` format.
pub struct OpenAIToGemini;

impl RequestTranslator for OpenAIToGemini {
    /// Translates an `OpenAI` chat completion request into a Gemini `generateContent` request.
    ///
    /// System messages are extracted into the `systemInstruction` field.
    /// The `assistant` role is mapped to `model`.
    ///
    /// # Errors
    ///
    /// Returns [`ByokError::Translation`] if `messages` is missing.
    fn translate_request(&self, req: Value) -> Result<Value> {
        let messages = req
            .get("messages")
            .and_then(Value::as_array)
            .ok_or_else(|| ByokError::Translation("missing 'messages'".into()))?;

        // Merge adjacent same-role messages before translation
        let merged = merge_adjacent_messages(messages);

        let system_parts: Vec<&str> = merged
            .iter()
            .filter(|m| m.get("role").and_then(Value::as_str) == Some("system"))
            .filter_map(|m| m.get("content").and_then(Value::as_str))
            .collect();
        let system_text = system_parts.join("\n");

        let contents: Vec<Value> = merged
            .iter()
            .filter(|m| m.get("role").and_then(Value::as_str) != Some("system"))
            .map(translate_message)
            .collect();

        let mut generation_config = json!({});
        if let Some(max_tokens) = req.get("max_tokens").and_then(Value::as_u64) {
            generation_config["maxOutputTokens"] = json!(max_tokens);
        }
        if let Some(temp) = req.get("temperature") {
            generation_config["temperature"] = temp.clone();
        }

        let mut out = json!({
            "contents": contents,
            "generationConfig": generation_config,
        });

        if !system_text.is_empty() {
            out["systemInstruction"] = json!({ "parts": [{"text": system_text}] });
        }

        // Translate tools → functionDeclarations
        if let Some(tools) = req.get("tools").and_then(Value::as_array) {
            let declarations: Vec<Value> = tools
                .iter()
                .filter_map(|t| t.get("function"))
                .map(|f| {
                    let mut decl = json!({
                        "name": f.get("name").unwrap_or(&Value::Null),
                    });
                    if let Some(desc) = f.get("description") {
                        decl["description"] = desc.clone();
                    }
                    if let Some(params) = f.get("parameters") {
                        decl["parameters"] = params.clone();
                    }
                    decl
                })
                .collect();
            if !declarations.is_empty() {
                out["tools"] = json!([{"functionDeclarations": declarations}]);
            }
        }

        // Translate tool_choice → toolConfig
        if let Some(tc) = req.get("tool_choice") {
            let config = match tc.as_str() {
                Some("auto") => json!({"functionCallingConfig":{"mode":"AUTO"}}),
                Some("none") => json!({"functionCallingConfig":{"mode":"NONE"}}),
                _ => {
                    if let Some(name) = tc.pointer("/function/name").and_then(Value::as_str) {
                        json!({"functionCallingConfig":{"mode":"ANY","allowedFunctionNames":[name]}})
                    } else {
                        json!({"functionCallingConfig":{"mode":"AUTO"}})
                    }
                }
            };
            out["toolConfig"] = config;
        }

        Ok(out)
    }
}

/// Translates a single `OpenAI` message to Gemini content format.
fn translate_message(m: &Value) -> Value {
    let role = m.get("role").and_then(Value::as_str).unwrap_or("user");

    // assistant with tool_calls → model with functionCall parts
    if role == "assistant"
        && let Some(tool_calls) = m.get("tool_calls").and_then(Value::as_array)
    {
        let mut parts: Vec<Value> = Vec::new();

        // Include text content if present
        if let Some(text) = m.get("content").and_then(Value::as_str)
            && !text.is_empty()
        {
            parts.push(json!({"text": text}));
        }

        for tc in tool_calls {
            let name = tc
                .pointer("/function/name")
                .and_then(Value::as_str)
                .unwrap_or("");
            let args_str = tc
                .pointer("/function/arguments")
                .and_then(Value::as_str)
                .unwrap_or("{}");
            let args: Value = serde_json::from_str(args_str).unwrap_or_else(|_| json!({}));
            parts.push(json!({"functionCall": {"name": name, "args": args}}));
        }

        return json!({"role": "model", "parts": parts});
    }

    // tool role → user with functionResponse part
    if role == "tool" {
        let tool_call_id = m.get("tool_call_id").and_then(Value::as_str).unwrap_or("");
        let name = tool_call_id
            .split_once('-')
            .map_or(tool_call_id, |(prefix, _)| prefix);
        let content = m.get("content").and_then(Value::as_str).unwrap_or("");
        return json!({
            "role": "user",
            "parts": [{"functionResponse": {"name": name, "response": {"result": content}}}]
        });
    }

    // Default: map role and content
    let gemini_role = match role {
        "assistant" => "model",
        _ => "user",
    };

    let parts = match m.get("content") {
        Some(Value::Array(arr)) => arr
            .iter()
            .map(|item| {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    json!({"text": text})
                } else {
                    json!({"text": item.to_string()})
                }
            })
            .collect(),
        Some(Value::String(s)) => vec![json!({"text": s})],
        _ => vec![json!({"text": ""})],
    };

    json!({"role": gemini_role, "parts": parts})
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_basic_contents() {
        let req = json!({
            "messages": [{"role": "user", "content": "Hello"}]
        });
        let out = OpenAIToGemini.translate_request(req).unwrap();
        assert_eq!(out["contents"][0]["role"], "user");
        assert_eq!(out["contents"][0]["parts"][0]["text"], "Hello");
    }

    #[test]
    fn test_assistant_becomes_model() {
        let req = json!({
            "messages": [
                {"role": "user", "content": "Hi"},
                {"role": "assistant", "content": "Hello!"}
            ]
        });
        let out = OpenAIToGemini.translate_request(req).unwrap();
        assert_eq!(out["contents"][1]["role"], "model");
    }

    #[test]
    fn test_system_to_instruction() {
        let req = json!({
            "messages": [
                {"role": "system", "content": "Be concise."},
                {"role": "user", "content": "Hi"}
            ]
        });
        let out = OpenAIToGemini.translate_request(req).unwrap();
        assert_eq!(out["systemInstruction"]["parts"][0]["text"], "Be concise.");
        assert_eq!(out["contents"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_max_tokens_mapping() {
        let req = json!({
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 512
        });
        let out = OpenAIToGemini.translate_request(req).unwrap();
        assert_eq!(out["generationConfig"]["maxOutputTokens"], 512);
    }

    #[test]
    fn test_no_system_no_instruction_field() {
        let req = json!({ "messages": [{"role": "user", "content": "hi"}] });
        let out = OpenAIToGemini.translate_request(req).unwrap();
        assert!(out.get("systemInstruction").is_none());
    }

    #[test]
    fn test_tools_to_function_declarations() {
        let req = json!({
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "description": "Get the weather",
                        "parameters": {
                            "type": "object",
                            "properties": {
                                "location": {"type": "string"}
                            }
                        }
                    }
                }
            ]
        });
        let out = OpenAIToGemini.translate_request(req).unwrap();
        let decls = &out["tools"][0]["functionDeclarations"];
        assert_eq!(decls[0]["name"], "get_weather");
        assert_eq!(decls[0]["description"], "Get the weather");
        assert!(decls[0]["parameters"]["properties"]["location"].is_object());
    }

    #[test]
    fn test_tool_choice_auto() {
        let req = json!({
            "messages": [{"role": "user", "content": "hi"}],
            "tool_choice": "auto"
        });
        let out = OpenAIToGemini.translate_request(req).unwrap();
        assert_eq!(out["toolConfig"]["functionCallingConfig"]["mode"], "AUTO");
    }

    #[test]
    fn test_tool_choice_none() {
        let req = json!({
            "messages": [{"role": "user", "content": "hi"}],
            "tool_choice": "none"
        });
        let out = OpenAIToGemini.translate_request(req).unwrap();
        assert_eq!(out["toolConfig"]["functionCallingConfig"]["mode"], "NONE");
    }

    #[test]
    fn test_tool_choice_specific_function() {
        let req = json!({
            "messages": [{"role": "user", "content": "hi"}],
            "tool_choice": {"type": "function", "function": {"name": "get_weather"}}
        });
        let out = OpenAIToGemini.translate_request(req).unwrap();
        assert_eq!(out["toolConfig"]["functionCallingConfig"]["mode"], "ANY");
        assert_eq!(
            out["toolConfig"]["functionCallingConfig"]["allowedFunctionNames"][0],
            "get_weather"
        );
    }

    #[test]
    fn test_tool_calls_to_function_call_part() {
        let req = json!({
            "messages": [
                {"role": "user", "content": "What is the weather?"},
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_123",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{\"location\":\"NYC\"}"
                        }
                    }]
                }
            ]
        });
        let out = OpenAIToGemini.translate_request(req).unwrap();
        let model_msg = &out["contents"][1];
        assert_eq!(model_msg["role"], "model");
        let fc = &model_msg["parts"][0]["functionCall"];
        assert_eq!(fc["name"], "get_weather");
        assert_eq!(fc["args"]["location"], "NYC");
    }

    #[test]
    fn test_tool_result_to_function_response() {
        let req = json!({
            "messages": [
                {"role": "user", "content": "What is the weather?"},
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_123",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{\"location\":\"NYC\"}"
                        }
                    }]
                },
                {
                    "role": "tool",
                    "tool_call_id": "get_weather-abc123",
                    "content": "72°F and sunny"
                }
            ]
        });
        let out = OpenAIToGemini.translate_request(req).unwrap();
        let tool_msg = &out["contents"][2];
        assert_eq!(tool_msg["role"], "user");
        let fr = &tool_msg["parts"][0]["functionResponse"];
        assert_eq!(fr["name"], "get_weather");
        assert_eq!(fr["response"]["result"], "72°F and sunny");
    }
}
