pub mod claude_code;
pub mod provider;
pub mod responses;
pub mod routes;
pub mod runtime;

pub use claude_code::ClaudeCodeConfig;
pub use provider::ProviderConfig;
pub use routes::{RouteSource, Routes};
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
    /// Which provider serves each Anthropic model.
    #[serde(default)]
    pub routes: Routes,
    /// Responses API routing for ChatGPT.app and Codex.
    #[serde(default)]
    pub responses: responses::ResponsesConfig,
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
            routes: Routes::default(),
            responses: responses::ResponsesConfig::default(),
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
        extract(&Figment::from(Serialized::defaults(Config::default())).merge(Yaml::string(yaml)))
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
        extract(&figment)
    }
}

/// Extract a [`Config`], refusing settings that no longer exist.
#[allow(clippy::result_large_err)]
fn extract(figment: &figment::Figment) -> Result<Config, figment::Error> {
    if let Ok(backend) = figment.find_value("providers.claude.backend") {
        let provider = backend.as_str().unwrap_or("<provider>");
        return Err(format!(
            "`providers.claude.backend` was replaced by `routes.default`; \
             run `byokey route set --default {provider}`"
        )
        .into());
    }
    if figment.find_value("providers.copilot.small_model").is_ok() {
        return Err(
            "`providers.copilot.small_model` was removed because requests without tools can be normal chat; \
             remove this setting and set `ANTHROPIC_DEFAULT_HAIKU_MODEL` in Claude Code to select its background model"
                .into(),
        );
    }
    let config: Config = figment.extract()?;
    config
        .responses
        .validate()
        .map_err(|e| figment::Error::from(e.to_string()))?;
    Ok(config)
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
    fn the_removed_claude_backend_names_its_replacement() {
        let err = Config::from_yaml("providers:\n  claude:\n    backend: copilot\n").unwrap_err();
        assert!(
            err.to_string()
                .contains("byokey route set --default copilot"),
            "{err}"
        );
    }

    #[test]
    fn small_model_requires_explicit_client_model_selection() {
        let err =
            Config::from_yaml("providers:\n  copilot:\n    small_model: gpt-5-mini\n").unwrap_err();
        assert!(
            err.to_string().contains("providers.copilot.small_model"),
            "{err}"
        );
        assert!(
            err.to_string().contains("ANTHROPIC_DEFAULT_HAIKU_MODEL"),
            "{err}"
        );
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
