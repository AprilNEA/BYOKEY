use byokey_types::ProviderId;
use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

/// Configuration for a single API key entry within a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyEntry {
    /// The API key value.
    pub api_key: String,
    /// Optional label for identification in logs.
    #[serde(default)]
    pub label: Option<String>,
    /// Optional custom base URL for this key (overrides the provider-level `base_url`).
    /// Enables using different endpoints per key, e.g. official API + third-party proxy.
    #[serde(default)]
    pub base_url: Option<String>,
}

/// Default header values injected into Claude API requests.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ClaudeHeaderDefaults {
    /// User-Agent header value.
    pub user_agent: Option<String>,
    /// `anthropic-package-version` header value.
    pub package_version: Option<String>,
    /// `anthropic-runtime-version` header value.
    pub runtime_version: Option<String>,
    /// `anthropic-os` header value.
    pub os: Option<String>,
    /// `anthropic-arch` header value.
    pub arch: Option<String>,
    /// `anthropic-timeout` header value (seconds).
    pub timeout: Option<u32>,
    /// Whether to stabilize the device profile across requests.
    pub stabilize_device_profile: Option<bool>,
}

/// Configuration for Claude request cloaking (system prompt injection + sensitive word obfuscation).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CloakConfig {
    /// Enable cloaking for Claude requests.
    pub enabled: bool,
    /// Strict mode: discard user-supplied system blocks, keep only billing + agent blocks.
    pub strict_mode: bool,
    /// Words to obfuscate by inserting a zero-width space after the first character.
    pub sensitive_words: Vec<String>,
}

/// Default header values injected into Codex API requests.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CodexHeaderDefaults {
    /// User-Agent header value.
    pub user_agent: Option<String>,
    /// `openai-beta` header value for feature flags.
    pub beta_features: Option<String>,
}

/// Strategy for selecting among multiple API keys.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyRoutingStrategy {
    /// Rotate through keys evenly (default).
    #[default]
    RoundRobin,
    /// Always prefer the first ready key; only use later keys on failure.
    Priority,
}

/// Configuration for a single provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Single API key (shorthand, takes precedence for simple configs).
    #[serde(default)]
    pub api_key: Option<String>,
    /// Multiple API keys with round-robin routing.
    #[serde(default)]
    pub api_keys: Vec<ApiKeyEntry>,
    /// Custom base URL for the provider API (overrides the default endpoint).
    /// Only the origin (scheme + host + optional port) should be specified;
    /// the executor appends its own path. Example: `https://my-proxy.example.com`
    #[serde(default)]
    pub base_url: Option<String>,
    /// Whether this provider is enabled (defaults to `true`).
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Strategy for selecting among multiple API keys.
    /// `round_robin` (default): rotate evenly.
    /// `priority`: always prefer the first key, only try later keys on failure.
    #[serde(default)]
    pub routing: KeyRoutingStrategy,
    /// Always route requests to this provider instead (e.g. `backend: copilot`
    /// lets Gemini requests go through GitHub Copilot).
    #[serde(default)]
    pub backend: Option<ProviderId>,
    /// Fallback provider to use when the primary provider fails.
    #[serde(default)]
    pub fallback: Option<ProviderId>,
    /// Maximum number of credentials to try before giving up.
    #[serde(default)]
    pub max_retry_credentials: Option<usize>,
    /// Default headers for Claude API requests.
    #[serde(default)]
    pub claude_headers: ClaudeHeaderDefaults,
    /// Default headers for Codex API requests.
    #[serde(default)]
    pub codex_headers: CodexHeaderDefaults,
    /// Claude request cloaking configuration.
    #[serde(default)]
    pub cloak: CloakConfig,
    /// Use WebSocket transport instead of HTTP (currently Codex only).
    #[serde(default)]
    pub websocket: bool,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            api_keys: Vec::new(),
            base_url: None,
            enabled: true,
            routing: KeyRoutingStrategy::default(),
            backend: None,
            fallback: None,
            max_retry_credentials: None,
            claude_headers: ClaudeHeaderDefaults::default(),
            codex_headers: CodexHeaderDefaults::default(),
            cloak: CloakConfig::default(),
            websocket: false,
        }
    }
}

impl ProviderConfig {
    /// Returns all configured API keys (merging `api_key` and `api_keys`).
    /// If `api_key` is set, it's treated as the first entry.
    #[must_use]
    pub fn all_api_keys(&self) -> Vec<&str> {
        let mut keys: Vec<&str> = Vec::new();
        if let Some(ref key) = self.api_key {
            keys.push(key.as_str());
        }
        for entry in &self.api_keys {
            keys.push(entry.api_key.as_str());
        }
        keys
    }

    /// Returns all configured API keys with their resolved base URLs.
    ///
    /// Each entry is `(api_key, base_url)`. Per-key `base_url` takes precedence
    /// over the provider-level `base_url`.
    #[must_use]
    pub fn all_api_keys_with_base_url(&self) -> Vec<(&str, Option<&str>)> {
        let default_base = self.base_url.as_deref();
        let mut result = Vec::new();
        if let Some(ref key) = self.api_key {
            result.push((key.as_str(), default_base));
        }
        for entry in &self.api_keys {
            let base = entry.base_url.as_deref().or(default_base);
            result.push((entry.api_key.as_str(), base));
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Config;

    #[test]
    fn test_provider_config_default_enabled() {
        let pc = ProviderConfig::default();
        assert!(pc.enabled);
        assert!(pc.api_key.is_none());
    }

    #[test]
    fn test_provider_config_backend_fallback_default_none() {
        let pc = ProviderConfig::default();
        assert!(pc.backend.is_none());
        assert!(pc.fallback.is_none());
    }

    #[test]
    fn test_from_yaml_provider_api_key() {
        let yaml = r#"
providers:
  claude:
    api_key: "sk-ant-test"
    enabled: true
"#;
        let c = Config::from_yaml(yaml).unwrap();
        let claude = c.providers.get(&ProviderId::Claude).unwrap();
        assert_eq!(claude.api_key.as_deref(), Some("sk-ant-test"));
        assert!(claude.enabled);
    }

    #[test]
    fn test_from_yaml_provider_disabled() {
        let yaml = r"
providers:
  codex:
    enabled: false
";
        let c = Config::from_yaml(yaml).unwrap();
        let codex = c.providers.get(&ProviderId::Codex).unwrap();
        assert!(!codex.enabled);
        assert!(codex.api_key.is_none());
    }

    #[test]
    fn test_from_yaml_backend_copilot() {
        let yaml = r"
providers:
  gemini:
    backend: copilot
";
        let c = Config::from_yaml(yaml).unwrap();
        let gemini = c.providers.get(&ProviderId::Gemini).unwrap();
        assert_eq!(gemini.backend, Some(ProviderId::Copilot));
        assert!(gemini.fallback.is_none());
    }

    #[test]
    fn test_from_yaml_fallback_copilot() {
        let yaml = r"
providers:
  gemini:
    fallback: copilot
";
        let c = Config::from_yaml(yaml).unwrap();
        let gemini = c.providers.get(&ProviderId::Gemini).unwrap();
        assert!(gemini.backend.is_none());
        assert_eq!(gemini.fallback, Some(ProviderId::Copilot));
    }

    #[test]
    fn test_from_yaml_api_keys() {
        let yaml = r#"
providers:
  claude:
    api_keys:
      - api_key: "sk-key1"
        label: "team-a"
      - api_key: "sk-key2"
"#;
        let c = Config::from_yaml(yaml).unwrap();
        let claude = c.providers.get(&ProviderId::Claude).unwrap();
        assert_eq!(claude.api_keys.len(), 2);
        assert_eq!(claude.api_keys[0].api_key, "sk-key1");
        assert_eq!(claude.api_keys[0].label.as_deref(), Some("team-a"));
        assert_eq!(claude.api_keys[1].api_key, "sk-key2");
        assert!(claude.api_keys[1].label.is_none());
    }

    #[test]
    fn test_all_api_keys_merges() {
        let pc = ProviderConfig {
            api_key: Some("single-key".into()),
            api_keys: vec![
                ApiKeyEntry {
                    api_key: "multi-1".into(),
                    label: None,
                    base_url: None,
                },
                ApiKeyEntry {
                    api_key: "multi-2".into(),
                    label: None,
                    base_url: None,
                },
            ],
            ..Default::default()
        };
        let keys = pc.all_api_keys();
        assert_eq!(keys, vec!["single-key", "multi-1", "multi-2"]);
    }

    #[test]
    fn test_all_api_keys_empty() {
        let pc = ProviderConfig::default();
        assert!(pc.all_api_keys().is_empty());
    }
}
