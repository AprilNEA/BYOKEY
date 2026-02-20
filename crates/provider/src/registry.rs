//! Model registry: static model lists and provider resolution.

use byok_types::ProviderId;

/// Returns the list of supported Claude model identifiers.
#[must_use]
pub fn claude_models() -> Vec<String> {
    vec![
        "claude-opus-4-6".into(),
        "claude-opus-4-5".into(),
        "claude-sonnet-4-5".into(),
        "claude-haiku-4-5-20251001".into(),
    ]
}

/// Returns the list of supported `OpenAI` (Codex) model identifiers.
#[must_use]
pub fn codex_models() -> Vec<String> {
    vec![
        "o4-mini".into(),
        "o3".into(),
        "gpt-4o".into(),
        "gpt-4o-mini".into(),
    ]
}

/// Returns the list of supported Gemini model identifiers.
#[must_use]
pub fn gemini_models() -> Vec<String> {
    vec![
        "gemini-2.0-flash".into(),
        "gemini-2.0-flash-lite".into(),
        "gemini-1.5-pro".into(),
        "gemini-1.5-flash".into(),
    ]
}

/// Returns the list of supported Kiro model identifiers.
#[must_use]
pub fn kiro_models() -> Vec<String> {
    // Kiro wraps Anthropic-compatible models
    vec!["kiro-default".into()]
}

/// Returns the list of supported GitHub Copilot model identifiers.
#[must_use]
pub fn copilot_models() -> Vec<String> {
    // GitHub Copilot OpenAI-compatible endpoint
    vec![
        "gpt-4o".into(),
        "gpt-4o-mini".into(),
        "claude-3.5-sonnet".into(),
        "o3-mini".into(),
    ]
}

/// Map a model string to its backing provider.
/// Returns `None` if the model is not recognised.
#[must_use]
pub fn resolve_provider(model: &str) -> Option<ProviderId> {
    if model.starts_with("claude-") {
        Some(ProviderId::Claude)
    } else if model.starts_with("gemini-") {
        Some(ProviderId::Gemini)
    } else if model.starts_with("kiro-") {
        Some(ProviderId::Kiro)
    } else if matches!(
        model,
        "o4-mini" | "o3" | "gpt-4o" | "gpt-4o-mini" | "gpt-4-turbo" | "gpt-4"
    ) {
        Some(ProviderId::Codex)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_claude() {
        assert_eq!(
            resolve_provider("claude-opus-4-6"),
            Some(ProviderId::Claude)
        );
        assert_eq!(
            resolve_provider("claude-haiku-4-5-20251001"),
            Some(ProviderId::Claude)
        );
    }

    #[test]
    fn test_resolve_gemini() {
        assert_eq!(
            resolve_provider("gemini-2.0-flash"),
            Some(ProviderId::Gemini)
        );
        assert_eq!(resolve_provider("gemini-1.5-pro"), Some(ProviderId::Gemini));
    }

    #[test]
    fn test_resolve_kiro() {
        assert_eq!(resolve_provider("kiro-default"), Some(ProviderId::Kiro));
    }

    #[test]
    fn test_resolve_codex() {
        assert_eq!(resolve_provider("gpt-4o"), Some(ProviderId::Codex));
        assert_eq!(resolve_provider("o4-mini"), Some(ProviderId::Codex));
        assert_eq!(resolve_provider("o3"), Some(ProviderId::Codex));
    }

    #[test]
    fn test_resolve_unknown() {
        assert_eq!(resolve_provider("unknown-model"), None);
        assert_eq!(resolve_provider(""), None);
    }

    #[test]
    fn test_model_lists_non_empty() {
        assert!(!claude_models().is_empty());
        assert!(!codex_models().is_empty());
        assert!(!gemini_models().is_empty());
        assert!(!kiro_models().is_empty());
        assert!(!copilot_models().is_empty());
    }

    #[test]
    fn test_claude_models_resolve_to_claude() {
        for m in claude_models() {
            assert_eq!(
                resolve_provider(&m),
                Some(ProviderId::Claude),
                "model {m} should resolve to Claude"
            );
        }
    }

    #[test]
    fn test_codex_models_resolve_to_codex() {
        for m in codex_models() {
            assert_eq!(
                resolve_provider(&m),
                Some(ProviderId::Codex),
                "model {m} should resolve to Codex"
            );
        }
    }

    #[test]
    fn test_gemini_models_resolve_to_gemini() {
        for m in gemini_models() {
            assert_eq!(
                resolve_provider(&m),
                Some(ProviderId::Gemini),
                "model {m} should resolve to Gemini"
            );
        }
    }
}
