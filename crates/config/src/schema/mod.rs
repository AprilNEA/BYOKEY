pub mod anthropic;
mod catalog;
pub mod claude_code;
pub mod claude_desktop;
pub mod provider;
pub mod responses;
pub mod routes;
pub mod runtime;

pub use claude_code::ClaudeCodeConfig;
pub use claude_desktop::ClaudeDesktopConfig;
pub use provider::{AnthropicProviderConfig, ConfigValue, ModelOverride, ProviderConfig};
pub use routes::{RouteSource, Routes};
pub use runtime::{LogConfig, LogFormat, TelemetryConfig};

use byokey_types::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn default_port() -> u16 {
    8018
}
fn default_host() -> String {
    "127.0.0.1".to_string()
}

/// Top-level application configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Listen port (defaults to 8018).
    #[serde(default = "default_port")]
    pub port: u16,
    /// Listen address (defaults to `127.0.0.1`).
    #[serde(default = "default_host")]
    pub host: String,
    /// Provider configuration map.
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderConfig>,
    /// Anthropic model catalog presentation for Claude clients.
    #[serde(default)]
    pub anthropic: anthropic::AnthropicConfig,
    /// Responses API routing for ChatGPT.app and Codex.
    #[serde(default)]
    pub responses: responses::ResponsesConfig,
    /// Claude Code CLI integration configuration.
    #[serde(default)]
    pub claude_code: ClaudeCodeConfig,
    /// Claude Desktop third-party integration configuration.
    #[serde(default)]
    pub claude_desktop: ClaudeDesktopConfig,
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
            providers: BTreeMap::new(),
            anthropic: anthropic::AnthropicConfig::default(),
            responses: responses::ResponsesConfig::default(),
            claude_code: ClaudeCodeConfig::default(),
            claude_desktop: ClaudeDesktopConfig::default(),
            proxy_url: None,
            log: LogConfig::default(),
            telemetry: TelemetryConfig::default(),
        }
    }
}

impl Config {
    /// Provider label shared by the Anthropic and Responses catalogs.
    #[must_use]
    pub fn provider_name<'a>(&'a self, name: &'a str) -> &'a str {
        self.providers
            .get(name)
            .and_then(|p| p.display_name.as_deref())
            .unwrap_or(match name {
                "chatgpt" => "ChatGPT",
                "copilot" => "Copilot",
                "claude" => "Claude (Anthropic)",
                "cursor" => "Cursor",
                _ => name,
            })
    }

    /// Sparse metadata for one provider model, not a client alias.
    #[must_use]
    pub fn model_override(&self, provider: &str, model: &str) -> Option<&ModelOverride> {
        self.providers.get(provider)?.model_overrides.get(model)
    }

    /// Trusted `ChatGPT` backend root for Responses and native Codex requests.
    #[must_use]
    pub fn chatgpt_base_url(&self) -> &str {
        self.providers
            .get("chatgpt")
            .and_then(|p| p.base_url.as_deref())
            .unwrap_or("https://chatgpt.com/backend-api/codex")
    }

    /// Validate provider, protocol, and client settings without resolving credentials.
    ///
    /// # Errors
    /// Returns an error for invalid provider metadata, templates, route targets, or client settings.
    pub fn validate(&self) -> Result<()> {
        for (name, provider) in &self.providers {
            provider.validate(name)?;
        }
        self.validate_anthropic()?;
        self.validate_responses()?;
        self.claude_desktop.validate()
    }

    /// Parses configuration from a YAML string, merged with defaults.
    ///
    /// # Errors
    ///
    /// Returns a [`figment::Error`] if the YAML is invalid or extraction fails.
    #[allow(clippy::result_large_err)]
    pub fn from_yaml(yaml: &str) -> std::result::Result<Self, figment::Error> {
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
    pub fn from_file(path: &std::path::Path) -> std::result::Result<Self, figment::Error> {
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

/// Extract and validate configuration before making the snapshot available.
#[allow(clippy::result_large_err)]
fn extract(figment: &figment::Figment) -> std::result::Result<Config, figment::Error> {
    let config: Config = figment.extract()?;
    config
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
    fn obsolete_settings_are_rejected_without_conversion() {
        for yaml in [
            "routes: { default: copilot }",
            "providers: { claude: { backend: copilot } }",
            "providers: { copilot: { small_model: gpt-5-mini } }",
            "responses: { default: copilot }",
            "responses: { models: {} }",
            "responses: { upstreams: {} }",
            "responses: { chatgpt_base_url: 'https://example.com' }",
            "responses: { catalog: { model_names: {} } }",
            "anthropic: { catalog: { provider_names: {} } }",
        ] {
            let error = Config::from_yaml(yaml).unwrap_err();
            assert!(
                error.to_string().contains("unknown field"),
                "{yaml}: {error}"
            );
        }
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
