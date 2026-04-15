use serde::{Deserialize, Serialize};

fn default_keepalive_seconds() -> u64 {
    15
}
fn default_bootstrap_retries() -> u32 {
    1
}
fn default_nonstream_keepalive_interval() -> u64 {
    30
}

/// Streaming SSE configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamingConfig {
    /// SSE keepalive interval in seconds (sends empty comment lines).
    #[serde(default = "default_keepalive_seconds")]
    pub keepalive_seconds: u64,
    /// Number of retries before the first byte arrives.
    #[serde(default = "default_bootstrap_retries")]
    pub bootstrap_retries: u32,
    /// Non-streaming request keepalive interval in seconds.
    #[serde(default = "default_nonstream_keepalive_interval")]
    pub nonstream_keepalive_interval: u64,
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            keepalive_seconds: default_keepalive_seconds(),
            bootstrap_retries: default_bootstrap_retries(),
            nonstream_keepalive_interval: default_nonstream_keepalive_interval(),
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Config;

    #[test]
    fn test_default_streaming_config() {
        let c = Config::default();
        assert_eq!(c.streaming.keepalive_seconds, 15);
        assert_eq!(c.streaming.bootstrap_retries, 1);
        assert_eq!(c.streaming.nonstream_keepalive_interval, 30);
    }

    #[test]
    fn test_from_yaml_streaming_config() {
        let yaml = r"
streaming:
  keepalive_seconds: 30
  bootstrap_retries: 3
  nonstream_keepalive_interval: 60
";
        let c = Config::from_yaml(yaml).unwrap();
        assert_eq!(c.streaming.keepalive_seconds, 30);
        assert_eq!(c.streaming.bootstrap_retries, 3);
        assert_eq!(c.streaming.nonstream_keepalive_interval, 60);
    }

    #[test]
    fn test_from_yaml_streaming_partial() {
        let yaml = r"
streaming:
  keepalive_seconds: 20
";
        let c = Config::from_yaml(yaml).unwrap();
        assert_eq!(c.streaming.keepalive_seconds, 20);
        assert_eq!(c.streaming.bootstrap_retries, 1);
        assert_eq!(c.streaming.nonstream_keepalive_interval, 30);
    }

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
