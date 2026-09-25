//! Model registry: static model lists and provider resolution.

use byokey_types::{ProviderId, ThinkingCapability};

/// Per-model thinking configuration support metadata.
pub struct ThinkingSupport {
    /// Minimum allowed thinking budget (tokens).
    pub min: u32,
    /// Maximum allowed thinking budget (tokens).
    pub max: u32,
    /// Supported discrete effort levels (e.g. `["low", "medium", "high", "max"]`).
    pub levels: &'static [&'static str],
    /// Whether a budget of 0 (disabled) is valid for this model.
    pub zero_allowed: bool,
}

/// A single model entry in the registry, mapping a model ID to its providers.
pub struct ModelEntry {
    /// The model identifier string (e.g. `"gpt-6-sol"` or `"claude-opus-5-5"`).
    pub id: &'static str,
    /// Providers that can serve this model, in priority order.
    pub providers: &'static [ProviderId],
    /// Thinking support metadata, if the model supports extended thinking.
    pub thinking: Option<&'static ThinkingSupport>,
}

/// Adaptive thinking steered by effort (Claude 4.6 generation and later).
const ADAPTIVE_THINKING: ThinkingSupport = ThinkingSupport {
    min: 1024,
    max: 128_000,
    levels: &["low", "medium", "high", "xhigh", "max"],
    zero_allowed: false,
};

/// Manual `budget_tokens` thinking (Claude Haiku 4.5 and earlier).
const BUDGET_THINKING: ThinkingSupport = ThinkingSupport {
    min: 1024,
    max: 64_000,
    levels: &[],
    zero_allowed: true,
};

/// Unified model registry. Provider order within each entry determines
/// resolution priority: the first provider wins in `resolve_provider()`.
/// Later providers serve the model under the same id, so a bare id still
/// routes when the first has no credentials (`resolve_provider_with()`).
const REGISTRY: &[ModelEntry] = &[
    // Codex (ChatGPT). Copilot serves these on `/responses` only, not on the
    // chat path.
    ModelEntry {
        id: "gpt-6-astra",
        providers: &[ProviderId::Codex],
        thinking: None,
    },
    ModelEntry {
        id: "gpt-6-sol",
        providers: &[ProviderId::Codex],
        thinking: None,
    },
    ModelEntry {
        id: "gpt-6-luna",
        providers: &[ProviderId::Codex],
        thinking: None,
    },
    // Claude (Anthropic). Legacy models the API still serves are reached
    // unprefixed on `/v1/messages` and as `claude/<id>` elsewhere.
    ModelEntry {
        id: "claude-fable-5-1",
        providers: &[ProviderId::Claude, ProviderId::Cursor],
        thinking: Some(&ADAPTIVE_THINKING),
    },
    ModelEntry {
        id: "claude-opus-5-5",
        providers: &[ProviderId::Claude, ProviderId::Cursor],
        thinking: Some(&ADAPTIVE_THINKING),
    },
    ModelEntry {
        id: "claude-sonnet-5",
        providers: &[ProviderId::Claude, ProviderId::Cursor, ProviderId::Copilot],
        thinking: Some(&ADAPTIVE_THINKING),
    },
    ModelEntry {
        id: "claude-haiku-4-5",
        providers: &[ProviderId::Claude, ProviderId::Cursor],
        thinking: Some(&BUDGET_THINKING),
    },
    // Copilot (GitHub), which spells Claude versions with dots.
    ModelEntry {
        id: "claude-fable-5.1",
        providers: &[ProviderId::Copilot],
        thinking: None,
    },
    ModelEntry {
        id: "claude-opus-5.5",
        providers: &[ProviderId::Copilot],
        thinking: None,
    },
    ModelEntry {
        id: "claude-haiku-4.5",
        providers: &[ProviderId::Copilot],
        thinking: None,
    },
    ModelEntry {
        id: "gpt-5.4",
        providers: &[ProviderId::Copilot, ProviderId::Cursor],
        thinking: None,
    },
    ModelEntry {
        id: "gpt-5-mini",
        providers: &[ProviderId::Copilot, ProviderId::Cursor],
        thinking: None,
    },
    // Gemini (Google AI)
    ModelEntry {
        id: "gemini-3.1-pro-preview",
        providers: &[ProviderId::Gemini],
        thinking: None,
    },
    ModelEntry {
        id: "gemini-3.8-flash",
        providers: &[ProviderId::Gemini, ProviderId::Copilot, ProviderId::Cursor],
        thinking: None,
    },
    ModelEntry {
        id: "gemini-3.5-flash-lite",
        providers: &[ProviderId::Gemini],
        thinking: None,
    },
    // Kiro
    ModelEntry {
        id: "kiro-default",
        providers: &[ProviderId::Kiro],
        thinking: None,
    },
    // Antigravity: Cloud Code model ids behind an `ag-` prefix.
    ModelEntry {
        id: "ag-gemini-pro-agent",
        providers: &[ProviderId::Antigravity],
        thinking: None,
    },
    ModelEntry {
        id: "ag-gemini-3.1-pro-low",
        providers: &[ProviderId::Antigravity],
        thinking: None,
    },
    ModelEntry {
        id: "ag-gemini-3.8-flash-high",
        providers: &[ProviderId::Antigravity],
        thinking: None,
    },
    ModelEntry {
        id: "ag-gemini-3.5-flash-lite",
        providers: &[ProviderId::Antigravity],
        thinking: None,
    },
    ModelEntry {
        id: "ag-claude-opus-4-6-thinking",
        providers: &[ProviderId::Antigravity],
        thinking: None,
    },
    ModelEntry {
        id: "ag-claude-sonnet-4-6",
        providers: &[ProviderId::Antigravity],
        thinking: None,
    },
    ModelEntry {
        id: "ag-gpt-oss-120b-medium",
        providers: &[ProviderId::Antigravity],
        thinking: None,
    },
    // Qwen: the one model Qwen OAuth serves.
    ModelEntry {
        id: "coder-model",
        providers: &[ProviderId::Qwen],
        thinking: None,
    },
    // Kimi (Kimi Code)
    ModelEntry {
        id: "kimi-for-coding",
        providers: &[ProviderId::Kimi],
        thinking: None,
    },
    ModelEntry {
        id: "kimi-for-coding-highspeed",
        providers: &[ProviderId::Kimi],
        thinking: None,
    },
    ModelEntry {
        id: "kimi-k3",
        providers: &[ProviderId::Kimi, ProviderId::Cursor],
        thinking: None,
    },
    // iFlow
    ModelEntry {
        id: "glm-4.5",
        providers: &[ProviderId::IFlow],
        thinking: None,
    },
    ModelEntry {
        id: "glm-4.5-air",
        providers: &[ProviderId::IFlow],
        thinking: None,
    },
    ModelEntry {
        id: "glm-z1-flash",
        providers: &[ProviderId::IFlow],
        thinking: None,
    },
    ModelEntry {
        id: "kimi-k2",
        providers: &[ProviderId::IFlow],
        thinking: None,
    },
    // Cursor: a sample of its live catalog (see `/v1/models` for the rest).
    // Any Cursor model is reachable as `cursor/<model>`.
    ModelEntry {
        id: "claude-opus-5-5-low-fast",
        providers: &[ProviderId::Cursor],
        thinking: None,
    },
    ModelEntry {
        id: "composer-2.5",
        providers: &[ProviderId::Cursor],
        thinking: None,
    },
    ModelEntry {
        id: "composer-2.5-fast",
        providers: &[ProviderId::Cursor],
        thinking: None,
    },
];

/// Returns the full model registry.
#[must_use]
pub fn all_models() -> &'static [ModelEntry] {
    REGISTRY
}

/// Parse a `"provider/model"` qualified string into `(Some(provider), model)`.
/// If there is no slash or the prefix is not a valid provider, returns
/// `(None, model)` unchanged.
#[must_use]
pub fn parse_qualified_model(model: &str) -> (Option<ProviderId>, &str) {
    if let Some((prefix, rest)) = model.split_once('/')
        && !rest.is_empty()
        && let Ok(provider) = prefix.parse::<ProviderId>()
    {
        return (Some(provider), rest);
    }
    (None, model)
}

/// Resolve a model string to its backing provider, considering only providers
/// for which `filter` returns `true`. Uses REGISTRY order (first match wins).
#[must_use]
pub fn resolve_provider_with<F>(model: &str, filter: F) -> Option<ProviderId>
where
    F: Fn(&ProviderId) -> bool,
{
    for entry in REGISTRY {
        if entry.id == model {
            for provider in entry.providers {
                if filter(provider) {
                    return Some(provider.clone());
                }
            }
        }
    }
    None
}

/// Map a model string to its backing provider.
/// Returns `None` if the model is not recognised.
#[must_use]
pub fn resolve_provider(model: &str) -> Option<ProviderId> {
    resolve_provider_with(model, |_| true)
}

/// Returns the thinking support metadata for a model, if it supports extended thinking.
#[must_use]
pub fn thinking_support(model: &str) -> Option<&'static ThinkingSupport> {
    REGISTRY
        .iter()
        .find(|e| e.id == model)
        .and_then(|e| e.thinking)
}

/// Returns the thinking capability classification for a model.
#[must_use]
pub fn thinking_capability(model: &str) -> Option<ThinkingCapability> {
    let support = thinking_support(model)?;
    let has_budget = support.min > 0 || support.max > 0;
    let has_levels = !support.levels.is_empty();
    Some(match (has_budget, has_levels) {
        (true, true) => ThinkingCapability::Hybrid,
        (false, true) => ThinkingCapability::LevelOnly,
        (true | false, false) => ThinkingCapability::BudgetOnly,
    })
}

/// Returns the model list for a given provider.
///
/// Models served by multiple providers will appear in each provider's list.
#[must_use]
pub fn models_for_provider(provider: &ProviderId) -> Vec<String> {
    REGISTRY
        .iter()
        .filter(|entry| entry.providers.contains(provider))
        .map(|entry| entry.id.to_string())
        .collect()
}

/// Returns model entries that are served by more than one provider.
#[must_use]
pub fn multi_provider_models() -> Vec<&'static ModelEntry> {
    REGISTRY
        .iter()
        .filter(|entry| entry.providers.len() > 1)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_claude() {
        for id in [
            "claude-fable-5-1",
            "claude-opus-5-5",
            "claude-sonnet-5",
            "claude-haiku-4-5",
        ] {
            assert_eq!(resolve_provider(id), Some(ProviderId::Claude), "{id}");
        }
        // Legacy models are left to explicit `claude/<id>` routing.
        assert_eq!(resolve_provider("claude-opus-4-6"), None);
    }

    #[test]
    fn test_resolve_gemini() {
        assert_eq!(
            resolve_provider("gemini-3.8-flash"),
            Some(ProviderId::Gemini)
        );
        assert_eq!(
            resolve_provider("gemini-3.1-pro-preview"),
            Some(ProviderId::Gemini)
        );
    }

    #[test]
    fn test_resolve_kiro() {
        assert_eq!(resolve_provider("kiro-default"), Some(ProviderId::Kiro));
    }

    #[test]
    fn test_resolve_codex() {
        assert_eq!(resolve_provider("gpt-6-astra"), Some(ProviderId::Codex));
        assert_eq!(resolve_provider("gpt-6-sol"), Some(ProviderId::Codex));
    }

    #[test]
    fn test_resolve_to_copilot() {
        assert_eq!(resolve_provider("gpt-5.4"), Some(ProviderId::Copilot));
        assert_eq!(resolve_provider("gpt-5-mini"), Some(ProviderId::Copilot));
        assert_eq!(
            resolve_provider("claude-haiku-4.5"),
            Some(ProviderId::Copilot)
        );
    }

    #[test]
    fn test_shared_models_resolve_to_their_vendor_first() {
        assert_eq!(
            resolve_provider("claude-sonnet-5"),
            Some(ProviderId::Claude)
        );
        assert_eq!(
            resolve_provider("gemini-3.8-flash"),
            Some(ProviderId::Gemini)
        );
        assert_eq!(resolve_provider("kimi-k3"), Some(ProviderId::Kimi));
    }

    #[test]
    fn test_retired_models_no_longer_resolve() {
        for id in [
            "o3",
            "o4-mini",
            "gpt-4o",
            "gpt-4.1",
            "gpt-5.1-codex",
            "gemini-2.0-flash",
            "gemini-1.5-pro",
            "ag-gemini-2.5-pro",
            "kimi-k2-0711",
            "qwen3-coder-plus",
        ] {
            assert_eq!(resolve_provider(id), None, "{id}");
        }
    }

    #[test]
    fn test_resolve_antigravity() {
        assert_eq!(
            resolve_provider("ag-gemini-pro-agent"),
            Some(ProviderId::Antigravity)
        );
        assert_eq!(
            resolve_provider("ag-claude-sonnet-4-6"),
            Some(ProviderId::Antigravity)
        );
    }

    #[test]
    fn test_resolve_kimi() {
        assert_eq!(resolve_provider("kimi-for-coding"), Some(ProviderId::Kimi));
    }

    #[test]
    fn test_kimi_k2_stays_iflow() {
        assert_eq!(resolve_provider("kimi-k2"), Some(ProviderId::IFlow));
    }

    #[test]
    fn test_kimi_models_resolve_to_kimi() {
        for m in models_for_provider(&ProviderId::Kimi) {
            assert_eq!(
                resolve_provider(&m),
                Some(ProviderId::Kimi),
                "model {m} should resolve to Kimi"
            );
        }
    }

    #[test]
    fn test_resolve_unknown() {
        assert_eq!(resolve_provider("unknown-model"), None);
        assert_eq!(resolve_provider(""), None);
    }

    #[test]
    fn test_model_lists_non_empty() {
        for provider in ProviderId::all() {
            let models = models_for_provider(provider);
            assert!(
                !models.is_empty(),
                "models_for_provider({provider:?}) returned empty — add at least one model to REGISTRY for this provider"
            );
        }
    }

    #[test]
    fn test_claude_models_resolve_to_claude() {
        for m in models_for_provider(&ProviderId::Claude) {
            assert_eq!(
                resolve_provider(&m),
                Some(ProviderId::Claude),
                "model {m} should resolve to Claude"
            );
        }
    }

    #[test]
    fn test_codex_models_resolve_to_codex() {
        for m in models_for_provider(&ProviderId::Codex) {
            assert_eq!(
                resolve_provider(&m),
                Some(ProviderId::Codex),
                "model {m} should resolve to Codex"
            );
        }
    }

    #[test]
    fn test_gemini_models_resolve_to_gemini() {
        for m in models_for_provider(&ProviderId::Gemini) {
            assert_eq!(
                resolve_provider(&m),
                Some(ProviderId::Gemini),
                "model {m} should resolve to Gemini"
            );
        }
    }

    #[test]
    fn test_antigravity_models_resolve_to_antigravity() {
        for m in models_for_provider(&ProviderId::Antigravity) {
            assert_eq!(
                resolve_provider(&m),
                Some(ProviderId::Antigravity),
                "model {m} should resolve to Antigravity"
            );
        }
    }

    #[test]
    fn test_resolve_provider_with_filter() {
        // gpt-5.4 has [Copilot, Cursor]; filtering out Copilot should yield Cursor.
        assert_eq!(
            resolve_provider_with("gpt-5.4", |p| *p != ProviderId::Copilot),
            Some(ProviderId::Cursor)
        );
        // Filtering out both should yield None.
        assert_eq!(
            resolve_provider_with("gpt-5.4", |p| {
                *p != ProviderId::Copilot && *p != ProviderId::Cursor
            }),
            None
        );
        // Single-provider model unaffected by permissive filter.
        assert_eq!(
            resolve_provider_with("gpt-6-sol", |_| true),
            Some(ProviderId::Codex)
        );
    }

    #[test]
    fn test_parse_qualified_model() {
        let (p, m) = parse_qualified_model("copilot/gpt-5.1");
        assert_eq!(p, Some(ProviderId::Copilot));
        assert_eq!(m, "gpt-5.1");

        let (p, m) = parse_qualified_model("gpt-5.1");
        assert_eq!(p, None);
        assert_eq!(m, "gpt-5.1");

        let (p, m) = parse_qualified_model("unknown/gpt-5.1");
        assert_eq!(p, None);
        assert_eq!(m, "unknown/gpt-5.1");

        // Empty tail should not be treated as qualified.
        let (p, m) = parse_qualified_model("copilot/");
        assert_eq!(p, None);
        assert_eq!(m, "copilot/");
    }

    #[test]
    fn test_all_models_non_empty() {
        assert!(!all_models().is_empty());
    }

    #[test]
    fn test_multi_provider_models() {
        let multi = multi_provider_models();
        assert!(!multi.is_empty());
        for entry in &multi {
            assert!(
                entry.providers.len() > 1,
                "model {} should have >1 providers",
                entry.id
            );
        }
    }

    #[test]
    fn test_claude_dashes_vs_dots() {
        // Dashes → Claude (Anthropic)
        assert_eq!(
            resolve_provider("claude-opus-5-5"),
            Some(ProviderId::Claude)
        );
        // Dots → Copilot (GitHub)
        assert_eq!(
            resolve_provider("claude-opus-5.5"),
            Some(ProviderId::Copilot)
        );
    }

    #[test]
    fn test_every_registry_model_resolves() {
        for entry in REGISTRY {
            assert!(
                resolve_provider(entry.id).is_some(),
                "model {} should resolve to some provider",
                entry.id
            );
        }
    }
}
