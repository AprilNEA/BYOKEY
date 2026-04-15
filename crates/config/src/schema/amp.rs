use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn default_amp_port() -> u16 {
    18018
}

/// Proxy-side configuration for `AmpCode` integration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AmpConfig {
    /// Separate listen port for the Amp-compatible router (default 18018).
    #[serde(default = "default_amp_port")]
    pub port: u16,

    /// Enables shared-proxy mode: when set, byokey strips the client's
    /// `Authorization` and `X-Api-Key` headers and injects this key upstream.
    #[serde(default)]
    pub upstream_key: Option<String>,

    /// AMP CLI settings merged into `~/.config/amp/settings.json` by
    /// `byokey amp inject`.
    #[serde(default)]
    pub settings: HashMap<String, serde_json::Value>,
}

impl Default for AmpConfig {
    fn default() -> Self {
        Self {
            port: default_amp_port(),
            upstream_key: None,
            settings: HashMap::new(),
        }
    }
}

impl AmpConfig {
    /// Resolve the `amp.url` value.
    ///
    /// Priority: CLI `--url` > `amp.settings["amp.url"]` > `http://{host}:{amp.port}`.
    #[must_use]
    pub fn resolve_url(&self, explicit: Option<&str>, host: &str) -> String {
        explicit
            .map(String::from)
            .or_else(|| {
                self.settings
                    .get("amp.url")
                    .and_then(|v| v.as_str().map(String::from))
            })
            .unwrap_or_else(|| format!("http://{host}:{}", self.port))
    }

    /// Default path for Amp CLI settings: `~/.config/amp/settings.json`.
    #[must_use]
    pub fn default_settings_path() -> Option<PathBuf> {
        std::env::var("HOME").ok().map(|h| {
            Path::new(&h)
                .join(".config")
                .join("amp")
                .join("settings.json")
        })
    }

    /// Merge this config's `settings` into the Amp CLI settings file,
    /// overwriting `amp.url` with `resolved_url`. Returns the number of
    /// extra settings merged (excluding `amp.url` itself).
    ///
    /// # Errors
    ///
    /// Returns an error if the settings file cannot be read or written.
    pub fn inject(&self, resolved_url: &str, settings_path: &Path) -> anyhow::Result<usize> {
        if let Some(parent) = settings_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut map: serde_json::Map<String, serde_json::Value> = if settings_path.exists() {
            let content = std::fs::read_to_string(settings_path)?;
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            serde_json::Map::new()
        };

        for (k, v) in &self.settings {
            map.insert(k.clone(), v.clone());
        }
        map.insert(
            "amp.url".to_string(),
            serde_json::Value::String(resolved_url.to_string()),
        );

        let json = serde_json::to_string_pretty(&serde_json::Value::Object(map))?;
        std::fs::write(settings_path, format!("{json}\n"))?;

        let extras = self
            .settings
            .len()
            .saturating_sub(usize::from(self.settings.contains_key("amp.url")));
        Ok(extras)
    }
}

#[cfg(test)]
mod tests {
    use super::AmpConfig;
    use crate::schema::Config;

    const HOST: &str = "127.0.0.1";

    #[test]
    fn test_resolve_url_explicit_wins() {
        let cfg = AmpConfig::default();
        assert_eq!(
            cfg.resolve_url(Some("http://custom:9999"), HOST),
            "http://custom:9999",
        );
    }

    #[test]
    fn test_resolve_url_from_settings() {
        let mut cfg = AmpConfig::default();
        cfg.settings.insert(
            "amp.url".to_string(),
            serde_json::json!("http://from-settings:1234"),
        );
        assert_eq!(cfg.resolve_url(None, HOST), "http://from-settings:1234",);
    }

    #[test]
    fn test_resolve_url_default_uses_amp_port() {
        let cfg = AmpConfig::default();
        assert_eq!(cfg.resolve_url(None, HOST), "http://127.0.0.1:18018");
    }

    #[test]
    fn test_resolve_url_custom_port() {
        let cfg = AmpConfig {
            port: 9999,
            ..AmpConfig::default()
        };
        assert_eq!(cfg.resolve_url(None, HOST), "http://127.0.0.1:9999");
    }

    #[test]
    fn test_resolve_url_custom_host() {
        let cfg = AmpConfig::default();
        assert_eq!(cfg.resolve_url(None, "0.0.0.0"), "http://0.0.0.0:18018");
    }

    #[test]
    fn test_inject_creates_and_merges() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");

        let mut cfg = AmpConfig::default();
        cfg.settings.insert(
            "amp.anthropic.effort".to_string(),
            serde_json::json!("high"),
        );

        let extras = cfg.inject("http://localhost:8018", &path).unwrap();
        assert_eq!(extras, 1);

        let content: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(content["amp.url"], "http://localhost:8018");
        assert_eq!(content["amp.anthropic.effort"], "high");
    }

    #[test]
    fn test_inject_preserves_existing_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, r#"{"amp.theme":"dark"}"#).unwrap();

        let cfg = AmpConfig::default();
        cfg.inject("http://localhost:8018", &path).unwrap();

        let content: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(content["amp.theme"], "dark");
        assert_eq!(content["amp.url"], "http://localhost:8018");
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
        assert_eq!(c.amp.port, 18018);
        assert!(c.amp.upstream_key.is_none());
        assert!(c.amp.settings.is_empty());
    }

    #[test]
    fn test_from_yaml_amp_port() {
        let yaml = "amp:\n  port: 19000";
        let c = Config::from_yaml(yaml).unwrap();
        assert_eq!(c.amp.port, 19000);
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
