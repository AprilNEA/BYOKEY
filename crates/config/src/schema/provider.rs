use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

/// Configuration for a single provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// A static credential used instead of the stored login: an Anthropic
    /// API key, a Copilot API bearer token, or a Cursor `crsr_…` key.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Custom base URL for the provider API (overrides the default endpoint).
    /// Only the origin (scheme + host + optional port) should be specified;
    /// paths are appended per request. Example: `https://my-proxy.example.com`
    #[serde(default)]
    pub base_url: Option<String>,
    /// Whether this provider is enabled (defaults to `true`).
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            base_url: None,
            enabled: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Config;
    use byokey_types::ProviderId;

    #[test]
    fn test_provider_config_default() {
        let pc = ProviderConfig::default();
        assert!(pc.enabled);
        assert!(pc.api_key.is_none());
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
  cursor:
    enabled: false
";
        let c = Config::from_yaml(yaml).unwrap();
        let cursor = c.providers.get(&ProviderId::Cursor).unwrap();
        assert!(!cursor.enabled);
        assert!(cursor.api_key.is_none());
    }

    #[test]
    fn unknown_providers_are_rejected() {
        let yaml = r"
providers:
  codex:
    enabled: false
";
        assert!(Config::from_yaml(yaml).is_err());
    }
}
