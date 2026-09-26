//! Anthropic model registry.
//!
//! The ids Anthropic's API serves unprefixed on `/v1/messages`, with what
//! each one supports for extended thinking. Copilot and Cursor publish live
//! catalogs instead, so their models are not listed here.

use byokey_types::ThinkingCapability;

/// A model Anthropic serves.
pub struct ModelEntry {
    /// The model identifier (e.g. `"claude-opus-5-5"`).
    pub id: &'static str,
    /// How the model takes a thinking configuration, if it thinks at all.
    pub thinking: Option<ThinkingCapability>,
}

/// Current Anthropic models. Older ids the API still serves are not listed;
/// requests for them pass through unchanged.
const REGISTRY: &[ModelEntry] = &[
    ModelEntry {
        id: "claude-fable-5-1",
        thinking: Some(ThinkingCapability::Hybrid),
    },
    ModelEntry {
        id: "claude-opus-5-5",
        thinking: Some(ThinkingCapability::Hybrid),
    },
    ModelEntry {
        id: "claude-sonnet-5",
        thinking: Some(ThinkingCapability::Hybrid),
    },
    ModelEntry {
        id: "claude-haiku-4-5",
        thinking: Some(ThinkingCapability::BudgetOnly),
    },
];

/// Every registered model, newest first.
#[must_use]
pub fn all_models() -> &'static [ModelEntry] {
    REGISTRY
}

/// Split a `"provider/model"` qualified id into `(Some(provider), model)`.
/// Without a slash, or with a prefix that is not a provider, the whole
/// string is the model.
#[must_use]
pub fn parse_qualified_model(model: &str) -> (Option<byokey_types::ProviderId>, &str) {
    if let Some((prefix, rest)) = model.split_once('/')
        && !rest.is_empty()
        && let Ok(provider) = prefix.parse()
    {
        return (Some(provider), rest);
    }
    (None, model)
}

/// How a registered model takes a thinking configuration.
#[must_use]
pub fn thinking_capability(model: &str) -> Option<ThinkingCapability> {
    REGISTRY
        .iter()
        .find(|e| e.id == model)
        .and_then(|e| e.thinking)
}

#[cfg(test)]
mod tests {
    use super::*;
    use byokey_types::ProviderId;

    #[test]
    fn current_models_think_adaptively_except_haiku() {
        for id in ["claude-fable-5-1", "claude-opus-5-5", "claude-sonnet-5"] {
            assert_eq!(
                thinking_capability(id),
                Some(ThinkingCapability::Hybrid),
                "{id}"
            );
        }
        assert_eq!(
            thinking_capability("claude-haiku-4-5"),
            Some(ThinkingCapability::BudgetOnly)
        );
        assert_eq!(thinking_capability("claude-opus-4-6"), None);
        assert_eq!(
            thinking_capability("claude-opus-5.5"),
            None,
            "Copilot spelling"
        );
    }

    #[test]
    fn qualified_ids_name_a_provider() {
        assert_eq!(
            parse_qualified_model("copilot/claude-opus-5.5"),
            (Some(ProviderId::Copilot), "claude-opus-5.5")
        );
        assert_eq!(
            parse_qualified_model("cursor/composer-2.5"),
            (Some(ProviderId::Cursor), "composer-2.5")
        );
        assert_eq!(
            parse_qualified_model("claude-opus-5-5"),
            (None, "claude-opus-5-5")
        );
        assert_eq!(
            parse_qualified_model("codex/gpt-5.4"),
            (None, "codex/gpt-5.4"),
            "not a provider"
        );
        assert_eq!(parse_qualified_model("copilot/"), (None, "copilot/"));
    }
}
