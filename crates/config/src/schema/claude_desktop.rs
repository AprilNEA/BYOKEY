use byokey_types::{ByokError, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Claude Desktop integration, applied by `byokey claude desktop`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaudeDesktopConfig {
    /// Native third-party settings that override Desktop behavior defaults.
    /// Gateway connection, credentials, discovery, and models remain BYOKEY-owned.
    #[serde(default)]
    pub settings: Map<String, Value>,
}

impl ClaudeDesktopConfig {
    pub(super) fn validate(&self) -> Result<()> {
        for key in [
            "inferenceProvider",
            "inferenceGatewayBaseUrl",
            "inferenceCredentialKind",
            "inferenceGatewayApiKey",
            "inferenceGatewayAuthScheme",
            "modelDiscoveryEnabled",
            "inferenceModels",
        ] {
            if self.settings.contains_key(key) {
                return Err(ByokError::Config(format!(
                    "claude_desktop.settings.{key} is managed by BYOKEY and cannot be overridden"
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::Config;

    #[test]
    fn desktop_gateway_fields_cannot_be_overridden_or_cleared() {
        for (key, value) in [
            ("inferenceProvider", "gateway"),
            ("inferenceGatewayBaseUrl", "https://other.example"),
            ("inferenceCredentialKind", "static"),
            ("inferenceGatewayApiKey", "null"),
            ("inferenceGatewayAuthScheme", "bearer"),
            ("modelDiscoveryEnabled", "true"),
            ("inferenceModels", "[]"),
        ] {
            let yaml = format!("claude_desktop:\n  settings:\n    {key}: {value}");

            let error = Config::from_yaml(&yaml).unwrap_err().to_string();

            assert!(
                error.contains(&format!(
                    "claude_desktop.settings.{key} is managed by BYOKEY"
                )),
                "{error}"
            );
        }
    }

    #[test]
    fn desktop_native_settings_require_the_settings_object() {
        let error = Config::from_yaml("claude_desktop: {coworkEgressAllowedHosts: ['*']}")
            .unwrap_err()
            .to_string();

        assert!(error.contains("unknown field"), "{error}");
        assert!(error.contains("coworkEgressAllowedHosts"), "{error}");
    }
}
