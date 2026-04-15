pub mod amp;
pub mod model;
pub mod payload;
pub mod provider;
pub mod runtime;

pub use amp::AmpConfig;
pub use model::ModelAlias;
pub use payload::{PayloadFilterRule, PayloadRule, PayloadRules};
pub use provider::{
    ApiKeyEntry, ClaudeHeaderDefaults, CloakConfig, CodexHeaderDefaults, KeyRoutingStrategy,
    ProviderConfig,
};
pub use runtime::{LogConfig, LogFormat, StreamingConfig};

use byokey_types::ProviderId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn default_port() -> u16 {
    8018
}
fn default_host() -> String {
    "127.0.0.1".to_string()
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
    /// Global upstream proxy URL (e.g. "socks5://user:pass@host:port").
    /// All upstream requests will go through this proxy.
    #[serde(default)]
    pub proxy_url: Option<String>,
    /// Model alias mappings per provider.
    #[serde(default)]
    pub model_alias: HashMap<ProviderId, Vec<ModelAlias>>,
    /// Models to exclude from the /v1/models listing, per provider.
    /// Supports glob patterns (e.g. "claude-3-*", "*-thinking").
    #[serde(default)]
    pub excluded_models: HashMap<ProviderId, Vec<String>>,
    /// Streaming SSE configuration.
    #[serde(default)]
    pub streaming: StreamingConfig,
    /// Payload rules for modifying request bodies.
    #[serde(default)]
    pub payload: PayloadRules,
    /// Logging configuration.
    #[serde(default)]
    pub log: LogConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: default_port(),
            host: default_host(),
            providers: HashMap::new(),
            amp: AmpConfig::default(),
            proxy_url: None,
            model_alias: HashMap::new(),
            excluded_models: HashMap::new(),
            streaming: StreamingConfig::default(),
            payload: PayloadRules::default(),
            log: LogConfig::default(),
        }
    }
}

/// Simple glob matching with `*` wildcard support.
pub(crate) fn glob_match(pattern: &str, text: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(suffix) = pattern.strip_prefix('*') {
        text.ends_with(suffix)
    } else if let Some(prefix) = pattern.strip_suffix('*') {
        text.starts_with(prefix)
    } else {
        pattern == text
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

    /// Resolves a model alias back to the original model name.
    /// If the input is not an alias, returns it unchanged.
    #[must_use]
    pub fn resolve_alias(&self, model: &str) -> String {
        for aliases in self.model_alias.values() {
            for alias in aliases {
                if alias.alias == model {
                    return alias.name.clone();
                }
            }
        }
        model.to_string()
    }

    /// Returns true if the model matches any excluded pattern for its provider.
    #[must_use]
    pub fn is_model_excluded(&self, provider: &ProviderId, model: &str) -> bool {
        if let Some(patterns) = self.excluded_models.get(provider) {
            for pattern in patterns {
                if glob_match(pattern, model) {
                    return true;
                }
            }
        }
        false
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
    fn test_from_yaml_defaults_applied() {
        let c = Config::from_yaml("port: 1234").unwrap();
        assert_eq!(c.port, 1234);
        assert_eq!(c.host, "127.0.0.1");
    }

    #[test]
    fn test_default_config_alias_and_excluded_empty() {
        let c = Config::default();
        assert!(c.model_alias.is_empty());
        assert!(c.excluded_models.is_empty());
    }

    #[test]
    fn test_default_proxy_url_is_none() {
        let c = Config::default();
        assert!(c.proxy_url.is_none());
    }

    #[test]
    fn test_from_yaml_proxy_url() {
        let yaml = r#"
proxy_url: "socks5://user:pass@host:1080"
"#;
        let c = Config::from_yaml(yaml).unwrap();
        assert_eq!(c.proxy_url.as_deref(), Some("socks5://user:pass@host:1080"));
    }

    #[test]
    fn test_from_yaml_model_alias() {
        let yaml = r#"
model_alias:
  claude:
    - name: "claude-sonnet-4-5-20250929"
      alias: "cs4.5"
      fork: true
    - name: "claude-opus-4-5"
      alias: "co4.5"
"#;
        let c = Config::from_yaml(yaml).unwrap();
        let aliases = c.model_alias.get(&ProviderId::Claude).unwrap();
        assert_eq!(aliases.len(), 2);
        assert_eq!(aliases[0].alias, "cs4.5");
        assert!(aliases[0].fork);
        assert_eq!(aliases[1].alias, "co4.5");
        assert!(!aliases[1].fork);
    }

    #[test]
    fn test_from_yaml_excluded_models() {
        let yaml = r#"
excluded_models:
  claude:
    - "claude-3-*"
    - "*-thinking"
"#;
        let c = Config::from_yaml(yaml).unwrap();
        let excluded = c.excluded_models.get(&ProviderId::Claude).unwrap();
        assert_eq!(excluded.len(), 2);
    }

    #[test]
    fn test_resolve_alias() {
        let yaml = r#"
model_alias:
  claude:
    - name: "claude-sonnet-4-5-20250929"
      alias: "cs4.5"
"#;
        let c = Config::from_yaml(yaml).unwrap();
        assert_eq!(c.resolve_alias("cs4.5"), "claude-sonnet-4-5-20250929");
        assert_eq!(c.resolve_alias("unknown"), "unknown");
    }

    #[test]
    fn test_is_model_excluded() {
        let yaml = r#"
excluded_models:
  claude:
    - "claude-3-*"
    - "*-thinking"
"#;
        let c = Config::from_yaml(yaml).unwrap();
        assert!(c.is_model_excluded(&ProviderId::Claude, "claude-3-opus"));
        assert!(c.is_model_excluded(&ProviderId::Claude, "anything-thinking"));
        assert!(!c.is_model_excluded(&ProviderId::Claude, "claude-opus-4-5"));
        assert!(!c.is_model_excluded(&ProviderId::Gemini, "claude-3-opus"));
    }

    #[test]
    fn test_glob_match_exact() {
        assert!(glob_match("claude-3-opus", "claude-3-opus"));
        assert!(!glob_match("claude-3-opus", "claude-3-sonnet"));
    }

    #[test]
    fn test_glob_match_star_prefix() {
        assert!(glob_match("*-thinking", "claude-thinking"));
        assert!(glob_match("*-thinking", "model-thinking"));
        assert!(!glob_match("*-thinking", "thinking-model"));
    }

    #[test]
    fn test_glob_match_star_suffix() {
        assert!(glob_match("claude-3-*", "claude-3-opus"));
        assert!(glob_match("claude-3-*", "claude-3-"));
        assert!(!glob_match("claude-3-*", "claude-4-opus"));
    }

    #[test]
    fn test_glob_match_star_only() {
        assert!(glob_match("*", "anything"));
    }
}
