//! Utilities for handling extended thinking blocks across providers.
//!
//! Provides extraction of thinking content from Claude responses, injection
//! of thinking budget parameters, and per-provider thinking configuration
//! parsed from model name suffixes.

use byokey_types::{ProviderId, ThinkingCapability};
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
        const HARD_CAP: u32 = 128_000;
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
    /// Automatic / dynamic thinking (e.g. `model(auto)` or `model(-1)`).
    /// Claude 4.6: adaptive thinking; legacy Claude: enabled without budget.
    Auto,
    /// Disabled (e.g. `model(none)`).
    Disabled,
}

/// Thinking effort level for providers that support it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingLevel {
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
    Max,
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
    // Match upstream suffix.go exactly: only these exact values are accepted.
    match value {
        "none" => Some(ThinkingConfig::Disabled),
        "auto" | "-1" => Some(ThinkingConfig::Auto),
        "minimal" => Some(ThinkingConfig::Level(ThinkingLevel::Minimal)),
        "low" => Some(ThinkingConfig::Level(ThinkingLevel::Low)),
        "medium" => Some(ThinkingConfig::Level(ThinkingLevel::Medium)),
        "high" => Some(ThinkingConfig::Level(ThinkingLevel::High)),
        "xhigh" => Some(ThinkingConfig::Level(ThinkingLevel::XHigh)),
        "max" => Some(ThinkingConfig::Level(ThinkingLevel::Max)),
        _ => value.parse::<u32>().ok().map(ThinkingConfig::Budget),
    }
}

/// Apply thinking configuration to a request body based on the resolved provider.
///
/// Different providers use different mechanisms:
/// - **Claude (legacy)**: `thinking.type: "enabled"` + `thinking.budget_tokens`
/// - **Claude 4.6 (adaptive)**: `thinking.type: "adaptive"` + `output_config.effort`
/// - **Codex**: `reasoning.effort` (low/medium/high)
/// - **Gemini/Antigravity**: `generationConfig.thinkingConfig.thinkingBudget`
/// - **`OpenAI` compat (Copilot)**: `reasoning_effort` field
///
/// The `capability` parameter determines whether Claude uses adaptive thinking
/// (`Hybrid` = has both budget and levels) or legacy budget mode (`BudgetOnly`).
///
/// Returns the (possibly modified) request body with the thinking config applied.
#[must_use]
pub fn apply_thinking(
    mut body: Value,
    provider: &ProviderId,
    config: &ThinkingConfig,
    capability: Option<ThinkingCapability>,
) -> Value {
    let is_adaptive = capability == Some(ThinkingCapability::Hybrid);
    match (provider, config) {
        // Disabled: remove all thinking-related fields across all providers
        (_, ThinkingConfig::Disabled) => {
            if let Some(obj) = body.as_object_mut() {
                obj.remove("thinking");
                obj.remove("reasoning");
                obj.remove("reasoning_effort");
                obj.remove("output_config");
                if let Some(gc) = obj
                    .get_mut("generationConfig")
                    .and_then(Value::as_object_mut)
                {
                    gc.remove("thinkingConfig");
                }
            }
            body
        }

        // ── Auto mode ───────────────────────────────────────────────────
        // Claude adaptive: thinking.type="adaptive" (let API pick effort).
        // Claude legacy: thinking.type="enabled" without budget.
        (ProviderId::Claude, ThinkingConfig::Auto) => {
            if is_adaptive {
                body["thinking"] = json!({"type": "adaptive"});
            } else {
                body["thinking"] = json!({"type": "enabled"});
            }
            if let Some(obj) = body.as_object_mut() {
                obj.remove("output_config");
            }
            body
        }
        // ── Claude ──────────────────────────────────────────────────────
        (ProviderId::Claude, ThinkingConfig::Budget(budget)) => {
            ThinkingExtractor::inject_thinking(body, *budget)
        }
        (ProviderId::Claude, ThinkingConfig::Level(level)) => {
            if is_adaptive {
                // Claude 4.6: use adaptive thinking with effort
                body["thinking"] = json!({"type": "adaptive"});
                body["output_config"] = json!({"effort": level_to_claude_effort(*level)});
                body
            } else {
                // Legacy: convert level to budget
                let budget = level_to_budget(*level);
                ThinkingExtractor::inject_thinking(body, budget)
            }
        }

        // ── Codex ───────────────────────────────────────────────────────
        (ProviderId::Codex, ThinkingConfig::Budget(budget)) => {
            let effort = budget_to_effort(*budget);
            body["reasoning"] = json!({"effort": effort});
            body
        }
        (ProviderId::Codex, ThinkingConfig::Level(level)) => {
            let effort = level_to_effort(*level);
            body["reasoning"] = json!({"effort": effort});
            body
        }

        // ── Gemini / Antigravity ────────────────────────────────────────
        (ProviderId::Gemini | ProviderId::Antigravity, ThinkingConfig::Budget(budget)) => {
            body["generationConfig"]["thinkingConfig"]["thinkingBudget"] = json!(budget);
            body
        }
        (ProviderId::Gemini | ProviderId::Antigravity, ThinkingConfig::Level(level)) => {
            let budget = level_to_budget(*level);
            body["generationConfig"]["thinkingConfig"]["thinkingBudget"] = json!(budget);
            body
        }

        // ── Copilot / OpenAI compat ─────────────────────────────────────
        (ProviderId::Copilot, ThinkingConfig::Budget(budget)) => {
            let effort = budget_to_effort(*budget);
            body["reasoning_effort"] = json!(effort);
            body
        }
        // Any provider with Level (including Copilot fallthrough): reasoning_effort
        (_, ThinkingConfig::Level(level)) => {
            body["reasoning_effort"] = json!(level_to_effort(*level));
            body
        }
        // Other providers + Auto or unsupported Budget: passthrough
        (_, ThinkingConfig::Auto | ThinkingConfig::Budget(_)) => body,
    }
}

/// Unified level→budget mapping used across all providers.
///
/// Values aligned with upstream `CLIProxyAPI` `convert.go`.
fn level_to_budget(level: ThinkingLevel) -> u32 {
    match level {
        ThinkingLevel::Minimal => 512,
        ThinkingLevel::Low => 1_024,
        ThinkingLevel::Medium => 8_192,
        ThinkingLevel::High => 24_576,
        ThinkingLevel::XHigh => 32_768,
        ThinkingLevel::Max => 128_000,
    }
}

/// Maps a budget to an effort string using upstream threshold-based conversion.
///
/// Thresholds (from upstream `convert.go`):
/// - ≤512   → minimal (maps to "low" effort)
/// - ≤1024  → low
/// - ≤8192  → medium
/// - ≤24576 → high
/// - >24576 → high (xhigh maps to high for effort providers)
fn budget_to_effort(budget: u32) -> &'static str {
    if budget <= 1_024 {
        "low"
    } else if budget <= 8_192 {
        "medium"
    } else {
        "high"
    }
}

/// Maps a thinking level to a Claude adaptive effort string.
///
/// Claude 4.6 supports: low, medium, high, max.
fn level_to_claude_effort(level: ThinkingLevel) -> &'static str {
    match level {
        ThinkingLevel::Minimal | ThinkingLevel::Low => "low",
        ThinkingLevel::Medium => "medium",
        ThinkingLevel::High | ThinkingLevel::XHigh => "high",
        ThinkingLevel::Max => "max",
    }
}

/// Maps a thinking level to an effort string for providers that use
/// discrete effort values (Codex, Copilot, OpenAI-compat).
fn level_to_effort(level: ThinkingLevel) -> &'static str {
    match level {
        ThinkingLevel::Minimal | ThinkingLevel::Low => "low",
        ThinkingLevel::Medium => "medium",
        ThinkingLevel::High | ThinkingLevel::XHigh | ThinkingLevel::Max => "high",
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
        let out = ThinkingExtractor::inject_thinking(req, 200_000);
        assert_eq!(out["thinking"]["budget_tokens"], 128_000);
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
    fn test_suffix_med_not_accepted() {
        // upstream only accepts exact "medium", not "med"
        let s = parse_model_suffix("model(med)");
        assert!(s.thinking.is_none());
    }

    #[test]
    fn test_suffix_disabled_none() {
        let s = parse_model_suffix("model(none)");
        assert_eq!(s.model, "model");
        assert_eq!(s.thinking, Some(ThinkingConfig::Disabled));
    }

    #[test]
    fn test_suffix_disabled_off_not_accepted() {
        // upstream only accepts "none", not "disabled" or "off"
        assert!(parse_model_suffix("m(disabled)").thinking.is_none());
        assert!(parse_model_suffix("m(off)").thinking.is_none());
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
        let out = apply_thinking(
            body,
            &ProviderId::Claude,
            &ThinkingConfig::Budget(10000),
            None,
        );
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
            None,
        );
        assert_eq!(out["thinking"]["type"], "enabled");
        assert_eq!(out["thinking"]["budget_tokens"], 24_576);
    }

    #[test]
    fn test_apply_codex_level_high() {
        let body = json!({"model": "o4-mini"});
        let out = apply_thinking(
            body,
            &ProviderId::Codex,
            &ThinkingConfig::Level(ThinkingLevel::High),
            None,
        );
        assert_eq!(out["reasoning"]["effort"], "high");
    }

    #[test]
    fn test_apply_codex_budget_low() {
        let body = json!({"model": "o4-mini"});
        let out = apply_thinking(body, &ProviderId::Codex, &ThinkingConfig::Budget(500), None);
        assert_eq!(out["reasoning"]["effort"], "low");
    }

    #[test]
    fn test_apply_codex_budget_medium() {
        let body = json!({"model": "o4-mini"});
        let out = apply_thinking(
            body,
            &ProviderId::Codex,
            &ThinkingConfig::Budget(2000),
            None,
        );
        assert_eq!(out["reasoning"]["effort"], "medium");
    }

    #[test]
    fn test_apply_gemini_budget() {
        let body = json!({"model": "gemini-2.0-flash"});
        let out = apply_thinking(
            body,
            &ProviderId::Gemini,
            &ThinkingConfig::Budget(16384),
            None,
        );
        assert_eq!(
            out["generationConfig"]["thinkingConfig"]["thinkingBudget"],
            16384
        );
    }

    #[test]
    fn test_apply_gemini_level_medium() {
        let body = json!({"model": "gemini-2.0-flash"});
        let out = apply_thinking(
            body,
            &ProviderId::Gemini,
            &ThinkingConfig::Level(ThinkingLevel::Medium),
            None,
        );
        assert_eq!(
            out["generationConfig"]["thinkingConfig"]["thinkingBudget"],
            8_192
        );
    }

    #[test]
    fn test_apply_claude_level_max() {
        let body = json!({"model": "m", "max_tokens": 100});
        let out = apply_thinking(
            body,
            &ProviderId::Claude,
            &ThinkingConfig::Level(ThinkingLevel::Max),
            None,
        );
        assert_eq!(out["thinking"]["type"], "enabled");
        assert_eq!(out["thinking"]["budget_tokens"], 128_000);
    }

    #[test]
    fn test_suffix_level_minimal() {
        let s = parse_model_suffix("model(minimal)");
        assert_eq!(s.model, "model");
        assert_eq!(
            s.thinking,
            Some(ThinkingConfig::Level(ThinkingLevel::Minimal))
        );
    }

    #[test]
    fn test_suffix_level_max() {
        let s = parse_model_suffix("model(max)");
        assert_eq!(s.model, "model");
        assert_eq!(s.thinking, Some(ThinkingConfig::Level(ThinkingLevel::Max)));
    }

    #[test]
    fn test_suffix_level_xhigh() {
        let s = parse_model_suffix("model(xhigh)");
        assert_eq!(s.model, "model");
        assert_eq!(
            s.thinking,
            Some(ThinkingConfig::Level(ThinkingLevel::XHigh))
        );
    }

    #[test]
    fn test_suffix_auto() {
        let s = parse_model_suffix("model(auto)");
        assert_eq!(s.model, "model");
        assert_eq!(s.thinking, Some(ThinkingConfig::Auto));
    }

    #[test]
    fn test_suffix_minus_one_is_auto() {
        let s = parse_model_suffix("model(-1)");
        assert_eq!(s.model, "model");
        assert_eq!(s.thinking, Some(ThinkingConfig::Auto));
    }

    #[test]
    fn test_apply_claude_auto() {
        let body = json!({"model": "claude-opus-4-5", "max_tokens": 8192});
        let out = apply_thinking(body, &ProviderId::Claude, &ThinkingConfig::Auto, None);
        assert_eq!(out["thinking"]["type"], "enabled");
        // Auto mode: no budget_tokens specified
        assert!(out["thinking"].get("budget_tokens").is_none());
    }

    #[test]
    fn test_apply_codex_auto_passthrough() {
        let body = json!({"model": "o4-mini"});
        let out = apply_thinking(
            body.clone(),
            &ProviderId::Codex,
            &ThinkingConfig::Auto,
            None,
        );
        assert_eq!(out, body);
    }

    #[test]
    fn test_apply_copilot_level_low() {
        let body = json!({"model": "gpt-4o"});
        let out = apply_thinking(
            body,
            &ProviderId::Copilot,
            &ThinkingConfig::Level(ThinkingLevel::Low),
            None,
        );
        assert_eq!(out["reasoning_effort"], "low");
    }

    #[test]
    fn test_apply_disabled_removes_fields() {
        let body = json!({
            "model": "m",
            "thinking": {"type": "enabled", "budget_tokens": 1000},
            "reasoning": {"effort": "high"},
            "reasoning_effort": "high",
            "output_config": {"effort": "high"},
            "generationConfig": {"thinkingConfig": {"thinkingBudget": 1000}}
        });
        let out = apply_thinking(body, &ProviderId::Claude, &ThinkingConfig::Disabled, None);
        assert!(out.get("thinking").is_none());
        assert!(out.get("reasoning").is_none());
        assert!(out.get("reasoning_effort").is_none());
        assert!(out.get("output_config").is_none());
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
            None,
        );
        assert_eq!(out["reasoning_effort"], "medium");
    }

    #[test]
    fn test_apply_claude_adaptive_level() {
        let body = json!({"model": "claude-opus-4-6"});
        let out = apply_thinking(
            body,
            &ProviderId::Claude,
            &ThinkingConfig::Level(ThinkingLevel::High),
            Some(ThinkingCapability::Hybrid),
        );
        assert_eq!(out["thinking"]["type"], "adaptive");
        assert_eq!(out["output_config"]["effort"], "high");
    }

    #[test]
    fn test_apply_claude_adaptive_auto() {
        let body = json!({"model": "claude-opus-4-6"});
        let out = apply_thinking(
            body,
            &ProviderId::Claude,
            &ThinkingConfig::Auto,
            Some(ThinkingCapability::Hybrid),
        );
        assert_eq!(out["thinking"]["type"], "adaptive");
        assert!(out.get("output_config").is_none());
    }

    #[test]
    fn test_apply_claude_adaptive_max_effort() {
        let body = json!({"model": "claude-opus-4-6"});
        let out = apply_thinking(
            body,
            &ProviderId::Claude,
            &ThinkingConfig::Level(ThinkingLevel::Max),
            Some(ThinkingCapability::Hybrid),
        );
        assert_eq!(out["output_config"]["effort"], "max");
    }

    #[test]
    fn test_apply_other_provider_budget_noop() {
        let body = json!({"model": "qwen-max"});
        let out = apply_thinking(
            body.clone(),
            &ProviderId::Qwen,
            &ThinkingConfig::Budget(8000),
            None,
        );
        assert_eq!(out, body);
    }
}
