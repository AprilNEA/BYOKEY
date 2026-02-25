use byokey_types::ProviderId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    /// Whether this provider is enabled (defaults to `true`).
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Always route requests to this provider instead (e.g. `backend: copilot`
    /// lets Gemini requests go through GitHub Copilot).
    #[serde(default)]
    pub backend: Option<ProviderId>,
    /// Fallback provider to use when the primary provider fails.
    #[serde(default)]
    pub fallback: Option<ProviderId>,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            api_keys: Vec::new(),
            enabled: true,
            backend: None,
            fallback: None,
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
    /// 拦截 `getUserFreeTierStatus` 响应，将 `canUseAmpFree` 和
    /// `isDailyGrantEnabled` 改为 `false`，隐藏免费层提示（默认关闭）。
    #[serde(default)]
    pub hide_free_tier: bool,
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
    /// The file format is determined by the file extension:
    /// `.json` uses JSON, everything else uses YAML.
    ///
    /// # Errors
    ///
    /// Returns a [`figment::Error`] if the file cannot be read or parsed.
    #[allow(clippy::result_large_err)]
    pub fn from_file(path: &std::path::Path) -> Result<Self, figment::Error> {
        use figment::{
            Figment,
            providers::{Format as _, Json, Serialized, Yaml},
        };
        let base = Figment::from(Serialized::defaults(Config::default()));
        let figment = if path.extension().is_some_and(|e| e == "json") {
            base.merge(Json::file(path))
        } else {
            base.merge(Yaml::file(path))
        };
        figment.extract()
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

    #[test]
    fn test_provider_config_backend_fallback_default_none() {
        let pc = ProviderConfig::default();
        assert!(pc.backend.is_none());
        assert!(pc.fallback.is_none());
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
                },
                ApiKeyEntry {
                    api_key: "multi-2".into(),
                    label: None,
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
