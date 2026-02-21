//! Utilities for handling Claude extended thinking blocks.
//!
//! Provides extraction of thinking content from Claude responses and injection
//! of thinking budget parameters into Claude requests.

use serde_json::{Value, json};

/// Handles extraction and injection of Claude extended thinking blocks.
pub struct ThinkingExtractor;

impl ThinkingExtractor {
    /// Converts Claude response content blocks (including thinking blocks) into a single string.
    ///
    /// Thinking blocks are wrapped in `<thinking>...</thinking>` tags and placed before the main text.
    pub fn extract_to_openai_content(content_blocks: &[Value]) -> String {
        let mut parts = Vec::new();
        for block in content_blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("thinking") => {
                    if let Some(t) = block.get("thinking").and_then(Value::as_str) {
                        parts.push(format!("<thinking>\n{t}\n</thinking>"));
                    }
                }
                Some("text") => {
                    if let Some(t) = block.get("text").and_then(Value::as_str) {
                        parts.push(t.to_string());
                    }
                }
                _ => {}
            }
        }
        parts.join("\n\n")
    }

    /// Parses a thinking budget from a model name with the format `<model>-thinking-<N>`.
    ///
    /// Returns `(clean_model_name, Option<budget_tokens>)`.
    #[must_use]
    pub fn parse_thinking_model(model: &str) -> (&str, Option<u32>) {
        if let Some(idx) = model.rfind("-thinking-") {
            let suffix = &model[idx + "-thinking-".len()..];
            if let Ok(budget) = suffix.parse::<u32>() {
                return (&model[..idx], Some(budget));
            }
        }
        (model, None)
    }

    /// Injects a thinking budget into a Claude request body.
    ///
    /// Applies a hard cap of 32,000 tokens and ensures `max_tokens` is large enough
    /// to accommodate the thinking budget plus headroom.
    pub fn inject_thinking(mut req: Value, budget_tokens: u32) -> Value {
        const HARD_CAP: u32 = 32_000;
        let effective = budget_tokens.min(HARD_CAP);
        let headroom = (effective / 10).max(1024);
        let min_max = effective + headroom;
        let current = u32::try_from(req.get("max_tokens").and_then(Value::as_u64).unwrap_or(0))
            .unwrap_or(u32::MAX);
        if current <= effective {
            req["max_tokens"] = json!(min_max);
        }
        req["thinking"] = json!({ "type": "enabled", "budget_tokens": effective });
        req
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_text_only() {
        let blocks = vec![json!({"type": "text", "text": "Hello"})];
        assert_eq!(
            ThinkingExtractor::extract_to_openai_content(&blocks),
            "Hello"
        );
    }

    #[test]
    fn test_extract_thinking_and_text() {
        let blocks = vec![
            json!({"type": "thinking", "thinking": "Let me think..."}),
            json!({"type": "text", "text": "The answer is 42."}),
        ];
        let r = ThinkingExtractor::extract_to_openai_content(&blocks);
        assert!(r.contains("<thinking>"));
        assert!(r.contains("Let me think..."));
        assert!(r.contains("</thinking>"));
        assert!(r.contains("The answer is 42."));
    }

    #[test]
    fn test_extract_empty() {
        assert_eq!(ThinkingExtractor::extract_to_openai_content(&[]), "");
    }

    #[test]
    fn test_parse_with_budget() {
        let (m, b) = ThinkingExtractor::parse_thinking_model("claude-opus-4-5-thinking-10000");
        assert_eq!(m, "claude-opus-4-5");
        assert_eq!(b, Some(10000));
    }

    #[test]
    fn test_parse_no_budget() {
        let (m, b) = ThinkingExtractor::parse_thinking_model("claude-opus-4-5");
        assert_eq!(m, "claude-opus-4-5");
        assert!(b.is_none());
    }

    #[test]
    fn test_parse_invalid_suffix() {
        let (m, b) = ThinkingExtractor::parse_thinking_model("claude-thinking-abc");
        assert_eq!(m, "claude-thinking-abc");
        assert!(b.is_none());
    }

    #[test]
    fn test_inject_sets_budget() {
        let req = json!({"model": "m", "max_tokens": 50000});
        let out = ThinkingExtractor::inject_thinking(req, 10000);
        assert_eq!(out["thinking"]["type"], "enabled");
        assert_eq!(out["thinking"]["budget_tokens"], 10000);
        assert_eq!(out["max_tokens"], 50000); // large enough, not modified
    }

    #[test]
    fn test_inject_bumps_max_tokens() {
        let req = json!({"model": "m", "max_tokens": 100});
        let out = ThinkingExtractor::inject_thinking(req, 10000);
        assert!(out["max_tokens"].as_u64().unwrap() > 10000);
    }

    #[test]
    fn test_inject_hard_cap() {
        let req = json!({"model": "m", "max_tokens": 999_999});
        let out = ThinkingExtractor::inject_thinking(req, 100_000);
        assert_eq!(out["thinking"]["budget_tokens"], 32_000);
    }
}
