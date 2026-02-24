//! Utilities for handling extended thinking blocks across providers.
//!
//! Provides extraction of thinking content from Claude responses, injection
//! of thinking budget parameters, and per-provider thinking configuration
//! parsed from model name suffixes.

use byokey_types::ProviderId;
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
    ///
    /// Prefer [`parse_model_suffix`] for new code — it supports both the legacy
    /// `-thinking-N` format and the newer `model(value)` parenthetical syntax.
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

/// Result of parsing a model name with an optional thinking suffix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSuffix {
    /// The clean model name without any suffix.
    pub model: String,
    /// The parsed thinking configuration, if any.
    pub thinking: Option<ThinkingConfig>,
}

/// Thinking configuration parsed from a model suffix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThinkingConfig {
    /// Budget mode: specific token count (e.g. `model(16384)` or `model-thinking-16384`).
    Budget(u32),
    /// Level mode (e.g. `model(high)`, `model(low)`, `model(medium)`).
    Level(ThinkingLevel),
    /// Disabled (e.g. `model(none)`).
    Disabled,
}

/// Thinking effort level for providers that support it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingLevel {
    Low,
    Medium,
    High,
}

/// Parses a model name with optional thinking suffix.
///
/// Supported formats:
/// - `model(16384)` → Budget mode with 16384 tokens
/// - `model(high)` / `model(low)` / `model(medium)` → Level mode
/// - `model(none)` → Thinking disabled
/// - `model-thinking-16384` → Legacy budget mode (backward compat)
/// - `model` → No thinking config
#[must_use]
pub fn parse_model_suffix(model: &str) -> ModelSuffix {
    // Try parenthetical format first: model(value)
    if let Some(open) = model.rfind('(')
        && model.ends_with(')')
    {
        let base = &model[..open];
        let value = &model[open + 1..model.len() - 1];
        if let Some(config) = parse_thinking_value(value) {
            return ModelSuffix {
                model: base.to_string(),
                thinking: Some(config),
            };
        }
    }

    // Legacy format: model-thinking-N
    if let Some(idx) = model.rfind("-thinking-") {
        let suffix = &model[idx + "-thinking-".len()..];
        if let Ok(budget) = suffix.parse::<u32>() {
            return ModelSuffix {
                model: model[..idx].to_string(),
                thinking: Some(ThinkingConfig::Budget(budget)),
            };
        }
    }

    ModelSuffix {
        model: model.to_string(),
        thinking: None,
    }
}

fn parse_thinking_value(value: &str) -> Option<ThinkingConfig> {
    match value {
        "none" | "disabled" | "off" => Some(ThinkingConfig::Disabled),
        "low" => Some(ThinkingConfig::Level(ThinkingLevel::Low)),
        "medium" | "med" => Some(ThinkingConfig::Level(ThinkingLevel::Medium)),
        "high" => Some(ThinkingConfig::Level(ThinkingLevel::High)),
        _ => value.parse::<u32>().ok().map(ThinkingConfig::Budget),
    }
}

/// Apply thinking configuration to a request body based on the resolved provider.
///
/// Different providers use different mechanisms:
/// - **Claude**: `thinking.type` + `thinking.budget_tokens`, adjust `max_tokens`
/// - **Codex**: `reasoning.effort` (low/medium/high)
/// - **Gemini/Antigravity**: `generationConfig.thinkingConfig.thinkingBudget`
/// - **`OpenAI` compat (Copilot)**: `reasoning_effort` field
///
/// Returns the (possibly modified) request body with the thinking config applied.
#[must_use]
pub fn apply_thinking(mut body: Value, provider: &ProviderId, config: &ThinkingConfig) -> Value {
    match (provider, config) {
        // Disabled: remove thinking fields
        (_, ThinkingConfig::Disabled) => {
            if let Some(obj) = body.as_object_mut() {
                obj.remove("thinking");
                obj.remove("reasoning_effort");
                if let Some(gc) = obj
                    .get_mut("generationConfig")
                    .and_then(Value::as_object_mut)
                {
                    gc.remove("thinkingConfig");
                }
            }
            body
        }
        // Claude: budget tokens
        (ProviderId::Claude, ThinkingConfig::Budget(budget)) => {
            ThinkingExtractor::inject_thinking(body, *budget)
        }
        (ProviderId::Claude, ThinkingConfig::Level(level)) => {
            let budget = match level {
                ThinkingLevel::Low => 4096,
                ThinkingLevel::Medium => 10_000,
                ThinkingLevel::High => 32_000,
            };
            ThinkingExtractor::inject_thinking(body, budget)
        }
        // Codex: reasoning effort
        (ProviderId::Codex, ThinkingConfig::Budget(budget)) => {
            let effort = if *budget <= 4096 {
                "low"
            } else if *budget <= 16_384 {
                "medium"
            } else {
                "high"
            };
            body["reasoning"] = json!({"effort": effort});
            body
        }
        (ProviderId::Codex, ThinkingConfig::Level(level)) => {
            let effort = level_to_str(*level);
            body["reasoning"] = json!({"effort": effort});
            body
        }
        // Gemini / Antigravity: thinkingConfig.thinkingBudget
        (ProviderId::Gemini | ProviderId::Antigravity, ThinkingConfig::Budget(budget)) => {
            body["generationConfig"]["thinkingConfig"]["thinkingBudget"] = json!(budget);
            body
        }
        (ProviderId::Gemini | ProviderId::Antigravity, ThinkingConfig::Level(level)) => {
            let budget = match level {
                ThinkingLevel::Low => 4096,
                ThinkingLevel::Medium => 16_384,
                ThinkingLevel::High => 32_768,
            };
            body["generationConfig"]["thinkingConfig"]["thinkingBudget"] = json!(budget);
            body
        }
        // Copilot / OpenAI compat / Other providers: reasoning_effort
        (ProviderId::Copilot, ThinkingConfig::Budget(budget)) => {
            let effort = if *budget <= 4096 {
                "low"
            } else if *budget <= 16_384 {
                "medium"
            } else {
                "high"
            };
            body["reasoning_effort"] = json!(effort);
            body
        }
        // Any provider with Level (including Copilot fallthrough): reasoning_effort
        (_, ThinkingConfig::Level(level)) => {
            body["reasoning_effort"] = json!(level_to_str(*level));
            body
        }
        (_, ThinkingConfig::Budget(_)) => body,
    }
}

fn level_to_str(level: ThinkingLevel) -> &'static str {
    match level {
        ThinkingLevel::Low => "low",
        ThinkingLevel::Medium => "medium",
        ThinkingLevel::High => "high",
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

    // --- parse_model_suffix tests ---

    #[test]
    fn test_suffix_budget_parens() {
        let s = parse_model_suffix("claude-opus-4-5(16384)");
        assert_eq!(s.model, "claude-opus-4-5");
        assert_eq!(s.thinking, Some(ThinkingConfig::Budget(16384)));
    }

    #[test]
    fn test_suffix_level_high() {
        let s = parse_model_suffix("model(high)");
        assert_eq!(s.model, "model");
        assert_eq!(s.thinking, Some(ThinkingConfig::Level(ThinkingLevel::High)));
    }

    #[test]
    fn test_suffix_level_low() {
        let s = parse_model_suffix("model(low)");
        assert_eq!(s.model, "model");
        assert_eq!(s.thinking, Some(ThinkingConfig::Level(ThinkingLevel::Low)));
    }

    #[test]
    fn test_suffix_level_medium() {
        let s = parse_model_suffix("model(medium)");
        assert_eq!(s.model, "model");
        assert_eq!(
            s.thinking,
            Some(ThinkingConfig::Level(ThinkingLevel::Medium))
        );
    }

    #[test]
    fn test_suffix_level_med_alias() {
        let s = parse_model_suffix("model(med)");
        assert_eq!(
            s.thinking,
            Some(ThinkingConfig::Level(ThinkingLevel::Medium))
        );
    }

    #[test]
    fn test_suffix_disabled_none() {
        let s = parse_model_suffix("model(none)");
        assert_eq!(s.model, "model");
        assert_eq!(s.thinking, Some(ThinkingConfig::Disabled));
    }

    #[test]
    fn test_suffix_disabled_aliases() {
        assert_eq!(
            parse_model_suffix("m(disabled)").thinking,
            Some(ThinkingConfig::Disabled)
        );
        assert_eq!(
            parse_model_suffix("m(off)").thinking,
            Some(ThinkingConfig::Disabled)
        );
    }

    #[test]
    fn test_suffix_legacy_budget() {
        let s = parse_model_suffix("claude-opus-4-5-thinking-10000");
        assert_eq!(s.model, "claude-opus-4-5");
        assert_eq!(s.thinking, Some(ThinkingConfig::Budget(10000)));
    }

    #[test]
    fn test_suffix_no_thinking() {
        let s = parse_model_suffix("claude-opus-4-5");
        assert_eq!(s.model, "claude-opus-4-5");
        assert!(s.thinking.is_none());
    }

    #[test]
    fn test_suffix_invalid_parens() {
        let s = parse_model_suffix("model(invalid)");
        assert_eq!(s.model, "model(invalid)");
        assert!(s.thinking.is_none());
    }

    #[test]
    fn test_suffix_empty_parens() {
        let s = parse_model_suffix("model()");
        assert_eq!(s.model, "model()");
        assert!(s.thinking.is_none());
    }

    // --- apply_thinking tests ---

    #[test]
    fn test_apply_claude_budget() {
        let body = json!({"model": "claude-opus-4-5", "max_tokens": 100});
        let out = apply_thinking(body, &ProviderId::Claude, &ThinkingConfig::Budget(10000));
        assert_eq!(out["thinking"]["type"], "enabled");
        assert_eq!(out["thinking"]["budget_tokens"], 10000);
        assert!(out["max_tokens"].as_u64().unwrap() > 10000);
    }

    #[test]
    fn test_apply_claude_level_high() {
        let body = json!({"model": "m", "max_tokens": 100});
        let out = apply_thinking(
            body,
            &ProviderId::Claude,
            &ThinkingConfig::Level(ThinkingLevel::High),
        );
        assert_eq!(out["thinking"]["type"], "enabled");
        assert_eq!(out["thinking"]["budget_tokens"], 32_000);
    }

    #[test]
    fn test_apply_codex_level_high() {
        let body = json!({"model": "o4-mini"});
        let out = apply_thinking(
            body,
            &ProviderId::Codex,
            &ThinkingConfig::Level(ThinkingLevel::High),
        );
        assert_eq!(out["reasoning"]["effort"], "high");
    }

    #[test]
    fn test_apply_codex_budget() {
        let body = json!({"model": "o4-mini"});
        let out = apply_thinking(body, &ProviderId::Codex, &ThinkingConfig::Budget(2000));
        assert_eq!(out["reasoning"]["effort"], "low");
    }

    #[test]
    fn test_apply_gemini_budget() {
        let body = json!({"model": "gemini-2.0-flash"});
        let out = apply_thinking(body, &ProviderId::Gemini, &ThinkingConfig::Budget(16384));
        assert_eq!(
            out["generationConfig"]["thinkingConfig"]["thinkingBudget"],
            16384
        );
    }

    #[test]
    fn test_apply_gemini_level() {
        let body = json!({"model": "gemini-2.0-flash"});
        let out = apply_thinking(
            body,
            &ProviderId::Gemini,
            &ThinkingConfig::Level(ThinkingLevel::Medium),
        );
        assert_eq!(
            out["generationConfig"]["thinkingConfig"]["thinkingBudget"],
            16_384
        );
    }

    #[test]
    fn test_apply_copilot_level_low() {
        let body = json!({"model": "gpt-4o"});
        let out = apply_thinking(
            body,
            &ProviderId::Copilot,
            &ThinkingConfig::Level(ThinkingLevel::Low),
        );
        assert_eq!(out["reasoning_effort"], "low");
    }

    #[test]
    fn test_apply_disabled_removes_fields() {
        let body = json!({
            "model": "m",
            "thinking": {"type": "enabled", "budget_tokens": 1000},
            "reasoning_effort": "high",
            "generationConfig": {"thinkingConfig": {"thinkingBudget": 1000}}
        });
        let out = apply_thinking(body, &ProviderId::Claude, &ThinkingConfig::Disabled);
        assert!(out.get("thinking").is_none());
        assert!(out.get("reasoning_effort").is_none());
        assert!(
            out["generationConfig"]
                .as_object()
                .unwrap()
                .get("thinkingConfig")
                .is_none()
        );
    }

    #[test]
    fn test_apply_other_provider_level() {
        let body = json!({"model": "qwen-max"});
        let out = apply_thinking(
            body,
            &ProviderId::Qwen,
            &ThinkingConfig::Level(ThinkingLevel::Medium),
        );
        assert_eq!(out["reasoning_effort"], "medium");
    }

    #[test]
    fn test_apply_other_provider_budget_noop() {
        let body = json!({"model": "qwen-max"});
        let out = apply_thinking(
            body.clone(),
            &ProviderId::Qwen,
            &ThinkingConfig::Budget(8000),
        );
        assert_eq!(out, body);
    }
}
