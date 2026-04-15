use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Proxy-side configuration for `AmpCode` integration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AmpConfig {
    /// Separate listen port for the Amp-compatible router.
    #[serde(default)]
    pub port: Option<u16>,

    /// Enables shared-proxy mode: when set, byokey strips the client's
    /// `Authorization` and `X-Api-Key` headers and injects this key upstream.
    #[serde(default)]
    pub upstream_key: Option<String>,

    /// AMP CLI settings merged into `~/.config/amp/settings.json` by
    /// `byokey amp inject`.
    #[serde(default)]
    pub settings: HashMap<String, serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use crate::schema::Config;

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
        assert!(c.amp.port.is_none());
        assert!(c.amp.upstream_key.is_none());
        assert!(c.amp.settings.is_empty());
    }

    #[test]
    fn test_from_yaml_amp_port() {
        let yaml = "amp:\n  port: 18018";
        let c = Config::from_yaml(yaml).unwrap();
        assert_eq!(c.amp.port, Some(18018));
    }

    #[test]
    fn test_from_yaml_amp_settings() {
        let yaml = r#"
amp:
  settings:
    "amp.url": "https://byokey.example.com/amp"
    "amp.anthropic.effort": "high"
    "amp.tools.disable":
      - "builtin:edit_file"
    "amp.network.timeout": 60
"#;
        let c = Config::from_yaml(yaml).unwrap();
        assert_eq!(
            c.amp.settings.get("amp.url"),
            Some(&serde_json::json!("https://byokey.example.com/amp")),
        );
        assert_eq!(
            c.amp.settings.get("amp.anthropic.effort"),
            Some(&serde_json::json!("high")),
        );
        assert_eq!(
            c.amp.settings.get("amp.tools.disable"),
            Some(&serde_json::json!(["builtin:edit_file"])),
        );
        assert_eq!(
            c.amp.settings.get("amp.network.timeout"),
            Some(&serde_json::json!(60)),
        );
    }
}
