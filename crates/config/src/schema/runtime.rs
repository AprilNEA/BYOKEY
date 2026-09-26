use serde::{Deserialize, Serialize};

/// Output format for structured logs.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    #[default]
    Text,
    Json,
}

/// Logging configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    /// Output format: text (default) or json.
    #[serde(default)]
    pub format: LogFormat,
    /// Optional log file path. If set, logs are written to this file
    /// with daily rotation. Stdout logging continues alongside.
    #[serde(default)]
    pub file: Option<String>,
    /// Log level override (default: "info"). Overridden by `RUST_LOG` env var.
    #[serde(default = "default_log_level")]
    pub level: String,
}

fn default_log_level() -> String {
    "info".to_string()
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            format: LogFormat::default(),
            file: None,
            level: default_log_level(),
        }
    }
}

fn default_telemetry_sample_rate() -> f32 {
    1.0
}

/// Telemetry (Sentry error reporting) configuration.
///
/// DSN resolution order at runtime:
/// 1. `SENTRY_DSN` environment variable
/// 2. `telemetry.sentry_dsn` from the config file
/// 3. Compile-time `BYOKEY_SENTRY_DSN` (baked in by CI for release builds)
///
/// Users can opt out completely by setting `telemetry.disabled: true`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryConfig {
    /// Explicit opt-out. When `true`, Sentry is never initialized, even if
    /// a DSN is present in the env, config, or compile-time defaults.
    #[serde(default)]
    pub disabled: bool,
    /// Sentry DSN override. When `Some`, takes precedence over the
    /// compile-time default but is itself overridden by the `SENTRY_DSN`
    /// environment variable.
    #[serde(default)]
    pub sentry_dsn: Option<String>,
    /// Sample rate for error events (0.0–1.0). Defaults to 1.0.
    #[serde(default = "default_telemetry_sample_rate")]
    pub sample_rate: f32,
    /// Environment tag sent with events (e.g. "production", "staging").
    #[serde(default)]
    pub environment: Option<String>,
    /// Release identifier. Defaults to `byokey@<CARGO_PKG_VERSION>`.
    #[serde(default)]
    pub release: Option<String>,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            disabled: false,
            sentry_dsn: None,
            sample_rate: default_telemetry_sample_rate(),
            environment: None,
            release: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Config;

    #[test]
    fn test_default_log_config() {
        let c = Config::default();
        assert_eq!(c.log.format, LogFormat::Text);
        assert!(c.log.file.is_none());
        assert_eq!(c.log.level, "info");
    }

    #[test]
    fn test_from_yaml_log_config() {
        let yaml = r#"
log:
  format: "json"
  file: "/tmp/byokey.log"
  level: "debug"
"#;
        let c = Config::from_yaml(yaml).unwrap();
        assert_eq!(c.log.format, LogFormat::Json);
        assert_eq!(c.log.file.as_deref(), Some("/tmp/byokey.log"));
        assert_eq!(c.log.level, "debug");
    }
}
