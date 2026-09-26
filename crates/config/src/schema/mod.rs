pub mod claude_code;
pub mod provider;
pub mod runtime;

pub use claude_code::ClaudeCodeConfig;
pub use provider::ProviderConfig;
pub use runtime::{LogConfig, LogFormat, TelemetryConfig};

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
    /// Claude Code CLI integration configuration.
    #[serde(default)]
    pub claude_code: ClaudeCodeConfig,
    /// Global upstream proxy URL (e.g. "socks5://user:pass@host:port").
    /// All upstream requests will go through this proxy.
    #[serde(default)]
    pub proxy_url: Option<String>,
    /// Logging configuration.
    #[serde(default)]
    pub log: LogConfig,
    /// Telemetry (Sentry) configuration.
    #[serde(default)]
    pub telemetry: TelemetryConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: default_port(),
            host: default_host(),
            providers: HashMap::new(),
            claude_code: ClaudeCodeConfig::default(),
            proxy_url: None,
            log: LogConfig::default(),
            telemetry: TelemetryConfig::default(),
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
  cursor:
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
}
