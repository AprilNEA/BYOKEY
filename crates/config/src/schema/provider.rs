use byokey_types::ProviderId;
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
    /// Claude: serve every `/v1/messages` request from this provider instead
    /// of Anthropic (`copilot` or `cursor`). A `copilot/` or `cursor/` model
    /// prefix picks a provider per request without this.
    #[serde(default)]
    pub backend: Option<ProviderId>,
    /// Copilot: serve Anthropic Messages requests that carry no tools with
    /// this model instead of the requested one. Claude Code sends several
    /// such requests per turn (titles, suggestions, summaries), and on a
    /// per-request Copilot plan each one costs a premium request.
    #[serde(default)]
    pub small_model: Option<String>,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            base_url: None,
            enabled: true,
            backend: None,
            small_model: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Config;

    #[test]
    fn test_provider_config_default() {
        let pc = ProviderConfig::default();
        assert!(pc.enabled);
        assert!(pc.api_key.is_none());
        assert!(pc.backend.is_none());
        assert!(pc.small_model.is_none());
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
    fn test_from_yaml_copilot_small_model() {
        let yaml = r"
providers:
  copilot:
    small_model: gpt-5-mini
";
        let c = Config::from_yaml(yaml).unwrap();
        let copilot = c.providers.get(&ProviderId::Copilot).unwrap();
        assert_eq!(copilot.small_model.as_deref(), Some("gpt-5-mini"));
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
    fn test_from_yaml_backend_copilot() {
        let yaml = r"
providers:
  claude:
    backend: copilot
";
        let c = Config::from_yaml(yaml).unwrap();
        let claude = c.providers.get(&ProviderId::Claude).unwrap();
        assert_eq!(claude.backend, Some(ProviderId::Copilot));
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
