use byokey_types::ProviderId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn default_true() -> bool {
    true
}

/// Configuration for a single provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Raw API key (takes precedence over OAuth tokens).
    #[serde(default)]
    pub api_key: Option<String>,
    /// Whether this provider is enabled (defaults to `true`).
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            enabled: true,
        }
    }
}

fn default_port() -> u16 {
    8018
}
fn default_host() -> String {
    "127.0.0.1".to_string()
}

/// `AmpCode` 管理代理配置。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AmpConfig {
    /// 设置后，byokey 进入"共享代理"模式：
    /// 客户端的 Authorization / X-Api-Key 头会被剥离，
    /// 改为注入此 key（同时设置 `Authorization: Bearer {key}` 和 `X-Api-Key: {key}`）。
    /// 不设置则保持 BYOK 透传行为（默认）。
    #[serde(default)]
    pub upstream_key: Option<String>,
}

/// Top-level application configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Listen port (defaults to 8018).
    #[serde(default = "default_port")]
    pub port: u16,
    /// Listen address (defaults to `127.0.0.1`).
    #[serde(default = "default_host")]
    pub host: String,
    /// Provider configuration map.
    #[serde(default)]
    pub providers: HashMap<ProviderId, ProviderConfig>,
    /// `AmpCode` 管理代理配置。
    #[serde(default)]
    pub amp: AmpConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: default_port(),
            host: default_host(),
            providers: HashMap::new(),
            amp: AmpConfig::default(),
        }
    }
}

impl Config {
    /// Parses configuration from a YAML string, merged with defaults.
    ///
    /// # Errors
    ///
    /// Returns a [`figment::Error`] if the YAML is invalid or extraction fails.
    #[allow(clippy::result_large_err)]
    pub fn from_yaml(yaml: &str) -> Result<Self, figment::Error> {
        use figment::{
            Figment,
            providers::{Format as _, Serialized, Yaml},
        };
        Figment::from(Serialized::defaults(Config::default()))
            .merge(Yaml::string(yaml))
            .extract()
    }

    /// Loads configuration from a file path, merged with defaults.
    ///
    /// # Errors
    ///
    /// Returns a [`figment::Error`] if the file cannot be read or parsed.
    #[allow(clippy::result_large_err)]
    pub fn from_file(path: &std::path::Path) -> Result<Self, figment::Error> {
        use figment::{
            Figment,
            providers::{Format as _, Serialized, Yaml},
        };
        Figment::from(Serialized::defaults(Config::default()))
            .merge(Yaml::file(path))
            .extract()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_YAML: &str = r#"
port: 9000
host: "0.0.0.0"
providers:
  claude:
    api_key: "sk-ant-test"
    enabled: true
  codex:
    enabled: false
"#;

    #[test]
    fn test_default_config() {
        let c = Config::default();
        assert_eq!(c.port, 8018);
        assert_eq!(c.host, "127.0.0.1");
        assert!(c.providers.is_empty());
    }

    #[test]
    fn test_from_yaml_port_and_host() {
        let c = Config::from_yaml(SAMPLE_YAML).unwrap();
        assert_eq!(c.port, 9000);
        assert_eq!(c.host, "0.0.0.0");
    }

    #[test]
    fn test_from_yaml_provider_api_key() {
        let c = Config::from_yaml(SAMPLE_YAML).unwrap();
        let claude = c.providers.get(&ProviderId::Claude).unwrap();
        assert_eq!(claude.api_key.as_deref(), Some("sk-ant-test"));
        assert!(claude.enabled);
    }

    #[test]
    fn test_from_yaml_provider_disabled() {
        let c = Config::from_yaml(SAMPLE_YAML).unwrap();
        let codex = c.providers.get(&ProviderId::Codex).unwrap();
        assert!(!codex.enabled);
        assert!(codex.api_key.is_none());
    }

    #[test]
    fn test_from_yaml_defaults_applied() {
        let c = Config::from_yaml("port: 1234").unwrap();
        assert_eq!(c.port, 1234);
        assert_eq!(c.host, "127.0.0.1"); // default preserved
    }

    #[test]
    fn test_provider_config_default_enabled() {
        let pc = ProviderConfig::default();
        assert!(pc.enabled);
        assert!(pc.api_key.is_none());
    }

    #[test]
    fn test_default_amp_upstream_key_is_none() {
        let c = Config::default();
        assert!(c.amp.upstream_key.is_none());
    }

    #[test]
    fn test_from_yaml_amp_upstream_key() {
        let yaml = r#"
amp:
  upstream_key: "amp-key-xxx"
"#;
        let c = Config::from_yaml(yaml).unwrap();
        assert_eq!(c.amp.upstream_key.as_deref(), Some("amp-key-xxx"));
    }

    #[test]
    fn test_from_yaml_amp_defaults_when_omitted() {
        let c = Config::from_yaml("port: 1234").unwrap();
        assert!(c.amp.upstream_key.is_none());
    }
}
