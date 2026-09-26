//! Provider identifiers and model capability definitions.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Identifies a supported upstream AI provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    Claude,
    Copilot,
    Cursor,
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Claude => write!(f, "claude"),
            Self::Copilot => write!(f, "copilot"),
            Self::Cursor => write!(f, "cursor"),
        }
    }
}

impl std::str::FromStr for ProviderId {
    type Err = crate::ByokError;

    /// Parse a provider name or well-known alias into a [`ProviderId`].
    ///
    /// # Errors
    ///
    /// Returns [`ByokError::UnsupportedProvider`](crate::ByokError::UnsupportedProvider)
    /// if the string does not match any known provider name or alias.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "claude" | "anthropic" => Ok(Self::Claude),
            "copilot" | "github" => Ok(Self::Copilot),
            "cursor" => Ok(Self::Cursor),
            other => Err(crate::ByokError::UnsupportedProvider(other.to_string())),
        }
    }
}

impl ProviderId {
    /// Returns a human-readable display name for the provider.
    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Claude => "Claude (Anthropic)",
            Self::Copilot => "GitHub Copilot",
            Self::Cursor => "Cursor",
        }
    }

    /// Returns all known provider variants.
    #[must_use]
    pub const fn all() -> [Self; 3] {
        [Self::Claude, Self::Copilot, Self::Cursor]
    }
}

/// How a model takes an extended-thinking configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingCapability {
    /// A `budget_tokens` value only (Claude Haiku 4.5 and earlier).
    BudgetOnly,
    /// Adaptive thinking steered by effort, or a budget (Claude 4.6 and later).
    Hybrid,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_display() {
        assert_eq!(ProviderId::Claude.to_string(), "claude");
        assert_eq!(ProviderId::Copilot.to_string(), "copilot");
        assert_eq!(ProviderId::Cursor.to_string(), "cursor");
    }

    #[test]
    fn test_from_str_canonical() {
        assert_eq!(ProviderId::from_str("claude").unwrap(), ProviderId::Claude);
        assert_eq!(
            ProviderId::from_str("copilot").unwrap(),
            ProviderId::Copilot
        );
        assert_eq!(ProviderId::from_str("cursor").unwrap(), ProviderId::Cursor);
    }

    #[test]
    fn test_from_str_aliases() {
        assert_eq!(
            ProviderId::from_str("anthropic").unwrap(),
            ProviderId::Claude
        );
        assert_eq!(ProviderId::from_str("github").unwrap(), ProviderId::Copilot);
    }

    #[test]
    fn test_from_str_unknown() {
        for name in ["xyz", "codex", "gemini"] {
            let err = ProviderId::from_str(name).unwrap_err();
            assert!(err.to_string().contains(name));
            assert!(matches!(err, crate::ByokError::UnsupportedProvider(_)));
        }
    }

    #[test]
    fn test_serde_roundtrip() {
        for p in ProviderId::all() {
            let json = serde_json::to_string(&p).unwrap();
            let back: ProviderId = serde_json::from_str(&json).unwrap();
            assert_eq!(back, p);
        }
    }

    #[test]
    fn test_hash_in_map() {
        use std::collections::HashMap;
        let mut map = HashMap::new();
        map.insert(ProviderId::Claude, "val");
        assert_eq!(map[&ProviderId::Claude], "val");
    }
}
