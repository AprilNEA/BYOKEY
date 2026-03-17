//! Model registry: static model lists and provider resolution.

use byokey_types::ProviderId;

/// A single model entry in the registry, mapping a model ID to its providers.
pub struct ModelEntry {
    /// The model identifier string (e.g. `"gpt-5.1"` or `"claude-opus-4-6"`).
    pub id: &'static str,
    /// Providers that can serve this model, in priority order.
    pub providers: &'static [ProviderId],
}

/// Unified model registry. Provider order within each entry determines
/// resolution priority: the first provider wins in `resolve_provider()`.
const REGISTRY: &[ModelEntry] = &[
    // Codex-only (reasoning + legacy)
    ModelEntry {
        id: "o3",
        providers: &[ProviderId::Codex],
    },
    ModelEntry {
        id: "o4-mini",
        providers: &[ProviderId::Codex],
    },
    ModelEntry {
        id: "gpt-4-turbo",
        providers: &[ProviderId::Codex],
    },
    ModelEntry {
        id: "gpt-4",
        providers: &[ProviderId::Codex],
    },
    // Codex-primary, also on Copilot
    ModelEntry {
        id: "gpt-5.4",
        providers: &[ProviderId::Codex, ProviderId::Copilot],
    },
    ModelEntry {
        id: "gpt-5.4-mini",
        providers: &[ProviderId::Codex, ProviderId::Copilot],
    },
    ModelEntry {
        id: "gpt-5.4-nano",
        providers: &[ProviderId::Codex],
    },
    ModelEntry {
        id: "gpt-5.3-codex",
        providers: &[ProviderId::Codex, ProviderId::Copilot],
    },
    ModelEntry {
        id: "gpt-5.3-codex-spark",
        providers: &[ProviderId::Codex],
    },
    ModelEntry {
        id: "gpt-5.2-codex",
        providers: &[ProviderId::Codex, ProviderId::Copilot],
    },
    ModelEntry {
        id: "gpt-5.2",
        providers: &[ProviderId::Codex, ProviderId::Copilot],
    },
    ModelEntry {
        id: "gpt-5.1-codex-max",
        providers: &[ProviderId::Codex, ProviderId::Copilot],
    },
    ModelEntry {
        id: "gpt-5.1-codex",
        providers: &[ProviderId::Codex, ProviderId::Copilot],
    },
    ModelEntry {
        id: "gpt-5.1-codex-mini",
        providers: &[ProviderId::Codex, ProviderId::Copilot],
    },
    ModelEntry {
        id: "gpt-5.1",
        providers: &[ProviderId::Codex, ProviderId::Copilot],
    },
    ModelEntry {
        id: "gpt-5-codex",
        providers: &[ProviderId::Codex],
    },
    ModelEntry {
        id: "gpt-5-codex-mini",
        providers: &[ProviderId::Codex],
    },
    ModelEntry {
        id: "gpt-5",
        providers: &[ProviderId::Codex, ProviderId::Copilot],
    },
    // Copilot-only
    ModelEntry {
        id: "gpt-4o",
        providers: &[ProviderId::Copilot],
    },
    ModelEntry {
        id: "gpt-4.1",
        providers: &[ProviderId::Copilot],
    },
    ModelEntry {
        id: "gpt-5-mini",
        providers: &[ProviderId::Copilot],
    },
    ModelEntry {
        id: "raptor-mini",
        providers: &[ProviderId::Copilot],
    },
    ModelEntry {
        id: "goldeneye",
        providers: &[ProviderId::Copilot],
    },
    ModelEntry {
        id: "grok-code-fast-1",
        providers: &[ProviderId::Copilot],
    },
    // Claude (Anthropic — dashes)
    ModelEntry {
        id: "claude-opus-4-6",
        providers: &[ProviderId::Claude],
    },
    ModelEntry {
        id: "claude-opus-4-5",
        providers: &[ProviderId::Claude],
    },
    ModelEntry {
        id: "claude-sonnet-4-5",
        providers: &[ProviderId::Claude, ProviderId::Antigravity],
    },
    ModelEntry {
        id: "claude-haiku-4-5-20251001",
        providers: &[ProviderId::Claude],
    },
    // Copilot Claude (GitHub — dots)
    ModelEntry {
        id: "claude-opus-4.6",
        providers: &[ProviderId::Copilot],
    },
    ModelEntry {
        id: "claude-opus-4.5",
        providers: &[ProviderId::Copilot],
    },
    ModelEntry {
        id: "claude-sonnet-4.6",
        providers: &[ProviderId::Copilot],
    },
    ModelEntry {
        id: "claude-sonnet-4.5",
        providers: &[ProviderId::Copilot],
    },
    ModelEntry {
        id: "claude-sonnet-4",
        providers: &[ProviderId::Copilot],
    },
    ModelEntry {
        id: "claude-haiku-4.5",
        providers: &[ProviderId::Copilot],
    },
    // Gemini (Google AI)
    ModelEntry {
        id: "gemini-2.0-flash",
        providers: &[ProviderId::Gemini],
    },
    ModelEntry {
        id: "gemini-2.0-flash-lite",
        providers: &[ProviderId::Gemini],
    },
    ModelEntry {
        id: "gemini-1.5-pro",
        providers: &[ProviderId::Gemini],
    },
    ModelEntry {
        id: "gemini-1.5-flash",
        providers: &[ProviderId::Gemini],
    },
    // Copilot Gemini (GitHub)
    ModelEntry {
        id: "gemini-2.5-pro",
        providers: &[ProviderId::Copilot],
    },
    ModelEntry {
        id: "gemini-3-flash",
        providers: &[ProviderId::Copilot],
    },
    ModelEntry {
        id: "gemini-3-pro",
        providers: &[ProviderId::Copilot],
    },
    ModelEntry {
        id: "gemini-3.1-pro",
        providers: &[ProviderId::Copilot],
    },
    // Kiro
    ModelEntry {
        id: "kiro-default",
        providers: &[ProviderId::Kiro],
    },
    // Antigravity
    ModelEntry {
        id: "ag-gemini-2.5-flash",
        providers: &[ProviderId::Antigravity],
    },
    ModelEntry {
        id: "ag-gemini-2.5-pro",
        providers: &[ProviderId::Antigravity],
    },
    ModelEntry {
        id: "ag-claude-sonnet-4-5",
        providers: &[ProviderId::Antigravity],
    },
    // Qwen
    ModelEntry {
        id: "qwen3-coder-plus",
        providers: &[ProviderId::Qwen],
    },
    ModelEntry {
        id: "qwen3-235b-a22b",
        providers: &[ProviderId::Qwen],
    },
    ModelEntry {
        id: "qwen3-32b",
        providers: &[ProviderId::Qwen],
    },
    ModelEntry {
        id: "qwen3-14b",
        providers: &[ProviderId::Qwen],
    },
    ModelEntry {
        id: "qwen3-8b",
        providers: &[ProviderId::Qwen],
    },
    ModelEntry {
        id: "qwen3-max",
        providers: &[ProviderId::Qwen],
    },
    ModelEntry {
        id: "qwen-plus",
        providers: &[ProviderId::Qwen],
    },
    ModelEntry {
        id: "qwen-turbo",
        providers: &[ProviderId::Qwen],
    },
    // Kimi
    ModelEntry {
        id: "kimi-k2-0711",
        providers: &[ProviderId::Kimi],
    },
    // iFlow
    ModelEntry {
        id: "glm-4.5",
        providers: &[ProviderId::IFlow],
    },
    ModelEntry {
        id: "glm-4.5-air",
        providers: &[ProviderId::IFlow],
    },
    ModelEntry {
        id: "glm-z1-flash",
        providers: &[ProviderId::IFlow],
    },
    ModelEntry {
        id: "kimi-k2",
        providers: &[ProviderId::IFlow],
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

/// Returns `true` if the model is available on the Copilot **Free** tier.
#[must_use]
pub fn is_copilot_free_model(model: &str) -> bool {
    matches!(
        model,
        "gpt-4o" | "gpt-4.1" | "gpt-5-mini" | "claude-haiku-4.5" | "raptor-mini" | "goldeneye"
    )
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
    }

    #[test]
    fn test_shared_models_resolve_to_codex_first() {
        // Codex is listed first in REGISTRY for shared models.
        assert_eq!(resolve_provider("gpt-5.1"), Some(ProviderId::Codex));
        assert_eq!(resolve_provider("gpt-5.1-codex"), Some(ProviderId::Codex));
        assert_eq!(resolve_provider("gpt-5.2"), Some(ProviderId::Codex));
        assert_eq!(resolve_provider("gpt-5.3-codex"), Some(ProviderId::Codex));
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
    fn test_resolve_kimi() {
        assert_eq!(resolve_provider("kimi-k2-0711"), Some(ProviderId::Kimi));
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
        // Antigravity-only models (ag- prefix) resolve to Antigravity.
        // Shared models like claude-sonnet-4-5 resolve to their first provider (Claude).
        for m in models_for_provider(&ProviderId::Antigravity) {
            let resolved = resolve_provider(&m);
            assert!(
                resolved == Some(ProviderId::Antigravity) || {
                    // Shared model: Antigravity must be a listed provider
                    resolve_provider_with(&m, |p| *p == ProviderId::Antigravity)
                        == Some(ProviderId::Antigravity)
                },
                "model {m} should be servable by Antigravity"
            );
        }
    }

    #[test]
    fn test_legacy_gpt4_resolves_to_codex() {
        assert_eq!(resolve_provider("gpt-4-turbo"), Some(ProviderId::Codex));
        assert_eq!(resolve_provider("gpt-4"), Some(ProviderId::Codex));
    }

    #[test]
    fn test_resolve_provider_with_filter() {
        // gpt-5.1 has [Codex, Copilot]; filtering out Codex should yield Copilot.
        assert_eq!(
            resolve_provider_with("gpt-5.1", |p| *p != ProviderId::Codex),
            Some(ProviderId::Copilot)
        );
        // Filtering out both should yield None.
        assert_eq!(
            resolve_provider_with("gpt-5.1", |p| {
                *p != ProviderId::Codex && *p != ProviderId::Copilot
            }),
            None
        );
        // Single-provider model unaffected by permissive filter.
        assert_eq!(
            resolve_provider_with("o3", |_| true),
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
            resolve_provider("claude-opus-4-6"),
            Some(ProviderId::Claude)
        );
        // Dots → Copilot (GitHub)
        assert_eq!(
            resolve_provider("claude-opus-4.6"),
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
