//! Model registry: static model lists and provider resolution.

use byokey_types::ProviderId;

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
///
/// Only reasoning models are listed here: the Codex Responses API (used for
/// OAuth / ChatGPT-account tokens) does not support GPT-4o variants.
/// GPT-4o models are served via GitHub Copilot instead.
#[must_use]
pub fn codex_models() -> Vec<String> {
    vec!["o4-mini".into(), "o3".into()]
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

/// Returns the list of supported Qwen (Alibaba) model identifiers.
#[must_use]
pub fn qwen_models() -> Vec<String> {
    vec![
        "qwen3-coder-plus".into(),
        "qwen3-235b-a22b".into(),
        "qwen3-32b".into(),
        "qwen3-14b".into(),
        "qwen3-8b".into(),
        "qwen3-max".into(),
        "qwen-plus".into(),
        "qwen-turbo".into(),
    ]
}

/// Returns the list of supported iFlow (Z.ai / GLM) model identifiers.
#[must_use]
pub fn iflow_models() -> Vec<String> {
    vec![
        "glm-4.5".into(),
        "glm-4.5-air".into(),
        "glm-z1-flash".into(),
        "kimi-k2".into(),
    ]
}

/// Returns the list of supported Antigravity model identifiers.
///
/// Prefixed with `ag-` to avoid conflicts with Claude/Gemini provider models.
#[must_use]
pub fn antigravity_models() -> Vec<String> {
    vec![
        "ag-gemini-2.5-flash".into(),
        "ag-gemini-2.5-pro".into(),
        "ag-claude-sonnet-4-5".into(),
    ]
}

/// Returns the list of supported GitHub Copilot model identifiers.
///
/// Includes Free-tier and Pro/Business/Enterprise models.
/// Some models (e.g. `claude-*`, `gemini-*`) share names with other providers
/// and are only routable via Copilot when `backend: copilot` is configured.
#[must_use]
pub fn copilot_models() -> Vec<String> {
    // GitHub Copilot OpenAI-compatible endpoint
    vec![
        // Free tier
        "gpt-4o".into(),
        "gpt-4.1".into(),
        "gpt-5-mini".into(),
        "claude-haiku-4.5".into(),
        "raptor-mini".into(),
        "goldeneye".into(),
        // Pro / Business / Enterprise
        "claude-sonnet-4".into(),
        "claude-sonnet-4.5".into(),
        "claude-sonnet-4.6".into(),
        "claude-opus-4.5".into(),
        "claude-opus-4.6".into(),
        "gemini-2.5-pro".into(),
        "gemini-3-flash".into(),
        "gemini-3-pro".into(),
        "gemini-3.1-pro".into(),
        "gpt-5.1".into(),
        "gpt-5.1-codex".into(),
        "gpt-5.1-codex-mini".into(),
        "gpt-5.1-codex-max".into(),
        "gpt-5.2".into(),
        "gpt-5.2-codex".into(),
        "gpt-5.3-codex".into(),
        "grok-code-fast-1".into(),
    ]
}

/// Returns `true` if the model is available on the Copilot **Free** tier.
#[must_use]
pub fn is_copilot_free_model(model: &str) -> bool {
    matches!(
        model,
        "gpt-4o" | "gpt-4.1" | "gpt-5-mini" | "claude-haiku-4.5" | "raptor-mini" | "goldeneye"
    )
}

/// Map a model string to its backing provider.
/// Returns `None` if the model is not recognised.
#[must_use]
pub fn resolve_provider(model: &str) -> Option<ProviderId> {
    if model.starts_with("ag-") {
        Some(ProviderId::Antigravity)
    } else if model.starts_with("claude-") {
        Some(ProviderId::Claude)
    } else if model.starts_with("gemini-") {
        Some(ProviderId::Gemini)
    } else if model.starts_with("kiro-") {
        Some(ProviderId::Kiro)
    } else if model.starts_with("qwen") {
        Some(ProviderId::Qwen)
    } else if model.starts_with("glm-") || model.starts_with("kimi-") {
        Some(ProviderId::IFlow)
    } else if matches!(
        model,
        "gpt-4o" | "gpt-4.1" | "gpt-5-mini" | "raptor-mini" | "goldeneye" | "grok-code-fast-1"
    ) || model.starts_with("gpt-5.")
    {
        Some(ProviderId::Copilot)
    } else if matches!(model, "o4-mini" | "o3" | "gpt-4-turbo" | "gpt-4") {
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
        assert_eq!(resolve_provider("o4-mini"), Some(ProviderId::Codex));
        assert_eq!(resolve_provider("o3"), Some(ProviderId::Codex));
    }

    #[test]
    fn test_resolve_to_copilot() {
        assert_eq!(resolve_provider("gpt-4o"), Some(ProviderId::Copilot));
        assert_eq!(resolve_provider("gpt-4.1"), Some(ProviderId::Copilot));
        assert_eq!(resolve_provider("gpt-5-mini"), Some(ProviderId::Copilot));
        assert_eq!(resolve_provider("raptor-mini"), Some(ProviderId::Copilot));
        assert_eq!(resolve_provider("goldeneye"), Some(ProviderId::Copilot));
        assert_eq!(
            resolve_provider("grok-code-fast-1"),
            Some(ProviderId::Copilot)
        );
        assert_eq!(resolve_provider("gpt-5.1"), Some(ProviderId::Copilot));
        assert_eq!(resolve_provider("gpt-5.1-codex"), Some(ProviderId::Copilot));
        assert_eq!(resolve_provider("gpt-5.2"), Some(ProviderId::Copilot));
        assert_eq!(resolve_provider("gpt-5.3-codex"), Some(ProviderId::Copilot));
    }

    #[test]
    fn test_retired_models_no_longer_copilot() {
        // These were retired on 2025-10-23 and should no longer resolve to Copilot.
        assert_ne!(resolve_provider("gpt-4o-mini"), Some(ProviderId::Copilot));
        assert_ne!(resolve_provider("o3-mini"), Some(ProviderId::Copilot));
        assert_ne!(
            resolve_provider("claude-3.5-sonnet"),
            Some(ProviderId::Copilot)
        );
    }

    #[test]
    fn test_is_copilot_free_model() {
        assert!(is_copilot_free_model("gpt-4o"));
        assert!(is_copilot_free_model("gpt-4.1"));
        assert!(is_copilot_free_model("gpt-5-mini"));
        assert!(is_copilot_free_model("claude-haiku-4.5"));
        assert!(is_copilot_free_model("raptor-mini"));
        assert!(is_copilot_free_model("goldeneye"));
        assert!(!is_copilot_free_model("gpt-5.1"));
        assert!(!is_copilot_free_model("claude-sonnet-4.5"));
        assert!(!is_copilot_free_model("grok-code-fast-1"));
    }

    #[test]
    fn test_resolve_antigravity() {
        assert_eq!(
            resolve_provider("ag-gemini-2.5-pro"),
            Some(ProviderId::Antigravity)
        );
        assert_eq!(
            resolve_provider("ag-claude-sonnet-4-5"),
            Some(ProviderId::Antigravity)
        );
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
        assert!(!antigravity_models().is_empty());
        assert!(!qwen_models().is_empty());
        assert!(!iflow_models().is_empty());
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

    #[test]
    fn test_antigravity_models_resolve_to_antigravity() {
        for m in antigravity_models() {
            assert_eq!(
                resolve_provider(&m),
                Some(ProviderId::Antigravity),
                "model {m} should resolve to Antigravity"
            );
        }
    }
}
