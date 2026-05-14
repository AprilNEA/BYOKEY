//! Parse BYOKEY-specific model-name UX conventions into canonical
//! [`aigw_core::model::ThinkingRequest`].
//!
//! BYOKEY clients can express thinking config in the model name itself:
//!
//! - `model(16384)` — token budget (decimal integer).
//! - `model(high)` / `model(low)` / `model(medium)` / `model(minimal)` /
//!   `model(xhigh)` / `model(max)` — discrete level.
//! - `model(auto)` / `model(-1)` — let the API decide.
//! - `model(none)` — disable thinking.
//! - `model-thinking-N` — legacy budget syntax (predates the parenthetical
//!   form; kept for backward compatibility).
//!
//! [`parse_model_suffix`] strips any matching suffix and returns the
//! clean model name plus the canonical [`ThinkingRequest`]. The proxy
//! pipeline writes that into `ChatRequest.thinking` so each provider's
//! aigw [`ThinkingProjector`] can translate it to the wire field for
//! that provider (Anthropic `thinking.type`, `OpenAI` Responses
//! `reasoning.effort`, `OpenAI` Chat Completions `reasoning_effort`,
//! Gemini `generationConfig.thinkingConfig.thinkingBudget`).
//!
//! [`ThinkingProjector`]: aigw_core::translate::ThinkingProjector

use aigw_core::model::{ThinkingLevel, ThinkingRequest};

/// Result of parsing a model name with an optional thinking suffix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSuffix {
    /// The clean model name without any suffix.
    pub model: String,
    /// Canonical thinking request, if a suffix was matched.
    pub thinking: Option<ThinkingRequest>,
}

/// Parses a model name with optional thinking suffix.
///
/// Returns the clean model name in `ModelSuffix.model` (suffix stripped),
/// and `Some(ThinkingRequest)` in `.thinking` only when a recognised
/// suffix was matched. Unknown parenthetical values like `model(foo)`
/// don't strip — the full name is returned and `.thinking` is `None`.
#[must_use]
pub fn parse_model_suffix(model: &str) -> ModelSuffix {
    // Try parenthetical format first: model(value)
    if let Some(open) = model.rfind('(')
        && model.ends_with(')')
    {
        let base = &model[..open];
        let value = &model[open + 1..model.len() - 1];
        if let Some(thinking) = parse_thinking_value(value) {
            return ModelSuffix {
                model: base.to_string(),
                thinking: Some(thinking),
            };
        }
    }

    // Legacy format: model-thinking-N
    if let Some(idx) = model.rfind("-thinking-") {
        let suffix = &model[idx + "-thinking-".len()..];
        if let Ok(budget_tokens) = suffix.parse::<u32>() {
            return ModelSuffix {
                model: model[..idx].to_string(),
                thinking: Some(ThinkingRequest::Budget { budget_tokens }),
            };
        }
    }

    ModelSuffix {
        model: model.to_string(),
        thinking: None,
    }
}

fn parse_thinking_value(value: &str) -> Option<ThinkingRequest> {
    // Match upstream CLIProxyAPI suffix.go: only these exact values are
    // accepted. Unknown values fall through and the suffix is left intact.
    match value {
        "none" => Some(ThinkingRequest::Disabled),
        "auto" | "-1" => Some(ThinkingRequest::Auto),
        "minimal" => Some(ThinkingRequest::Level {
            level: ThinkingLevel::Minimal,
        }),
        "low" => Some(ThinkingRequest::Level {
            level: ThinkingLevel::Low,
        }),
        "medium" => Some(ThinkingRequest::Level {
            level: ThinkingLevel::Medium,
        }),
        "high" => Some(ThinkingRequest::Level {
            level: ThinkingLevel::High,
        }),
        "xhigh" => Some(ThinkingRequest::Level {
            level: ThinkingLevel::XHigh,
        }),
        "max" => Some(ThinkingRequest::Level {
            level: ThinkingLevel::Max,
        }),
        _ => value
            .parse::<u32>()
            .ok()
            .map(|budget_tokens| ThinkingRequest::Budget { budget_tokens }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_in_parens() {
        let s = parse_model_suffix("claude-opus-4-5(16384)");
        assert_eq!(s.model, "claude-opus-4-5");
        assert_eq!(
            s.thinking,
            Some(ThinkingRequest::Budget {
                budget_tokens: 16_384
            })
        );
    }

    #[test]
    fn level_high() {
        let s = parse_model_suffix("model(high)");
        assert_eq!(s.model, "model");
        assert_eq!(
            s.thinking,
            Some(ThinkingRequest::Level {
                level: ThinkingLevel::High
            })
        );
    }

    #[test]
    fn level_minimal() {
        let s = parse_model_suffix("model(minimal)");
        assert_eq!(
            s.thinking,
            Some(ThinkingRequest::Level {
                level: ThinkingLevel::Minimal
            })
        );
    }

    #[test]
    fn level_max() {
        let s = parse_model_suffix("model(max)");
        assert_eq!(
            s.thinking,
            Some(ThinkingRequest::Level {
                level: ThinkingLevel::Max
            })
        );
    }

    #[test]
    fn level_xhigh() {
        let s = parse_model_suffix("model(xhigh)");
        assert_eq!(
            s.thinking,
            Some(ThinkingRequest::Level {
                level: ThinkingLevel::XHigh
            })
        );
    }

    #[test]
    fn med_not_accepted() {
        // upstream only accepts exact "medium", not "med"
        let s = parse_model_suffix("model(med)");
        assert!(s.thinking.is_none());
        assert_eq!(s.model, "model(med)");
    }

    #[test]
    fn disabled_none() {
        let s = parse_model_suffix("model(none)");
        assert_eq!(s.thinking, Some(ThinkingRequest::Disabled));
    }

    #[test]
    fn disabled_off_not_accepted() {
        // upstream only accepts "none", not "disabled" or "off"
        assert!(parse_model_suffix("m(disabled)").thinking.is_none());
        assert!(parse_model_suffix("m(off)").thinking.is_none());
    }

    #[test]
    fn auto_keyword() {
        let s = parse_model_suffix("model(auto)");
        assert_eq!(s.thinking, Some(ThinkingRequest::Auto));
    }

    #[test]
    fn auto_minus_one() {
        let s = parse_model_suffix("model(-1)");
        assert_eq!(s.thinking, Some(ThinkingRequest::Auto));
    }

    #[test]
    fn legacy_dash_thinking_n() {
        let s = parse_model_suffix("claude-opus-4-5-thinking-10000");
        assert_eq!(s.model, "claude-opus-4-5");
        assert_eq!(
            s.thinking,
            Some(ThinkingRequest::Budget {
                budget_tokens: 10_000
            })
        );
    }

    #[test]
    fn no_suffix() {
        let s = parse_model_suffix("claude-opus-4-5");
        assert_eq!(s.model, "claude-opus-4-5");
        assert!(s.thinking.is_none());
    }

    #[test]
    fn unknown_paren_value_left_intact() {
        let s = parse_model_suffix("model(invalid)");
        assert_eq!(s.model, "model(invalid)");
        assert!(s.thinking.is_none());
    }

    #[test]
    fn empty_parens_left_intact() {
        let s = parse_model_suffix("model()");
        assert_eq!(s.model, "model()");
        assert!(s.thinking.is_none());
    }
}
