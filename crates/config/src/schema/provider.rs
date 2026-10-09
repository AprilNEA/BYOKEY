use byokey_types::{ByokError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

fn default_true() -> bool {
    true
}

/// Connection and model metadata shared by all routes to a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderConfig {
    /// Credential used instead of a stored login or for a custom Responses provider.
    /// `ChatGPT` credentials must come from the client, not this field.
    pub api_key: Option<ConfigValue>,
    /// Custom base URL for the provider API (overrides the default endpoint).
    /// Claude, Copilot and Cursor use an origin; `ChatGPT` and custom Responses
    /// providers accept a path prefix. Request paths are appended to this URL.
    pub base_url: Option<String>,
    /// Whether this provider is enabled (defaults to `true`).
    pub enabled: bool,
    /// Provider label used by both protocol catalogs.
    pub display_name: Option<String>,
    /// Sparse metadata overrides keyed by the provider's model ID.
    pub model_overrides: BTreeMap<String, ModelOverride>,
    /// Model discovery URL for a custom Responses provider.
    pub models_url: Option<String>,
    /// Custom Responses headers; Authorization overrides `api_key`.
    pub headers: BTreeMap<String, ConfigValue>,
    /// Custom Responses service tier; absent preserves the client's value.
    pub service_tier: Option<String>,
    /// Independent Messages connection for a custom provider.
    pub anthropic: Option<AnthropicProviderConfig>,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            base_url: None,
            enabled: default_true(),
            display_name: None,
            model_overrides: BTreeMap::new(),
            models_url: None,
            headers: BTreeMap::new(),
            service_tier: None,
            anthropic: None,
        }
    }
}

/// A custom Messages upstream. Credentials never inherit from Responses or stored logins.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnthropicProviderConfig {
    /// URL prefix to which `/v1/messages` and `/v1/messages/count_tokens` are appended.
    pub base_url: String,
    /// Discovery endpoint returning a `data` array of Claude model IDs.
    #[serde(default)]
    pub models_url: Option<String>,
    /// Exact catalog IDs: filter discovery, or define entries without `models_url`.
    /// An empty set hides all entries. Requests are not restricted.
    #[serde(default)]
    pub enabled_models: Option<BTreeSet<String>>,
    /// Optional credential sent as `x-api-key`.
    #[serde(default)]
    pub api_key: Option<ConfigValue>,
    /// Headers for discovery, generation and counting. Values resolve for each request.
    #[serde(default)]
    pub headers: BTreeMap<String, ConfigValue>,
}

/// Metadata patches applied after discovery, independent of route aliases.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModelOverride {
    /// Display name, without changing the requested model ID.
    pub name: Option<String>,
    /// Codex catalog model whose metadata matches this provider model.
    pub catalog_model: Option<String>,
    /// Complete Codex metadata instead of borrowing the `ChatGPT` catalog.
    pub catalog: Option<Value>,
}

/// An explicit literal, environment reference, or generated header value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum ConfigValue {
    /// A literal value; never interpreted as an environment variable or command.
    Literal(String),
    /// Read the named environment variable from the server process.
    Environment {
        /// Environment variable name.
        env: String,
    },
    /// Generate a UUID v4 each time the value is resolved.
    Uuid {
        /// Prefix before the lowercase, hyphenated UUID.
        uuid_prefix: String,
    },
}

impl ConfigValue {
    /// Resolve a value without including credentials in errors.
    ///
    /// # Errors
    /// Returns an error if the referenced environment variable is unavailable.
    pub fn resolve(&self) -> Result<String> {
        match self {
            Self::Literal(value) => Ok(value.clone()),
            Self::Environment { env } => std::env::var(env).map_err(|_| {
                ByokError::Config(format!("environment variable {env} is unavailable"))
            }),
            Self::Uuid { uuid_prefix } => Ok(format!("{uuid_prefix}{}", uuid::Uuid::new_v4())),
        }
    }
}

impl ProviderConfig {
    pub(super) fn validate(&self, name: &str) -> Result<()> {
        let path = format!("providers.{name}");
        if name.trim().is_empty() || name.contains('/') {
            return Err(ByokError::Config(format!("invalid provider name: {name}")));
        }
        let builtin = matches!(name, "claude" | "copilot" | "cursor" | "chatgpt");
        if !builtin
            && (self.anthropic.is_none()
                || self.base_url.is_some()
                || self.models_url.is_some()
                || self.api_key.is_some()
                || !self.headers.is_empty()
                || self.service_tier.is_some())
            && self
                .base_url
                .as_deref()
                .is_none_or(|url| url.trim().is_empty())
        {
            return Err(ByokError::Config(format!("{path}.base_url is required")));
        }
        if name == "chatgpt" && self.api_key.is_some() {
            return Err(ByokError::Config(format!(
                "{path} uses client-owned credentials"
            )));
        }
        if builtin
            && (self.models_url.is_some()
                || !self.headers.is_empty()
                || self.service_tier.is_some())
        {
            return Err(ByokError::Config(format!(
                "{path}: models_url, headers and service_tier require a custom Responses provider"
            )));
        }
        if let Some(anthropic) = &self.anthropic {
            if builtin {
                return Err(ByokError::Config(format!(
                    "{path}.anthropic requires a custom provider"
                )));
            }
            if name == "1m" || name.contains(['[', ']']) {
                return Err(ByokError::Config(format!(
                    "{path}: Messages provider names must not be 1m or contain brackets"
                )));
            }
            if anthropic.base_url.trim().is_empty()
                || anthropic
                    .models_url
                    .as_ref()
                    .is_some_and(|url| url.trim().is_empty())
            {
                return Err(ByokError::Config(format!(
                    "{path}.anthropic URLs must be nonempty"
                )));
            }
        }
        if self
            .display_name
            .as_ref()
            .is_some_and(|label| label.trim().is_empty())
        {
            return Err(ByokError::Config(format!(
                "{path}.display_name must be nonempty"
            )));
        }
        for (id, model) in &self.model_overrides {
            if id.trim().is_empty()
                || model
                    .name
                    .as_ref()
                    .is_some_and(|label| label.trim().is_empty())
            {
                return Err(ByokError::Config(format!(
                    "{path}.model_overrides IDs and names must be nonempty"
                )));
            }
            if model
                .catalog
                .as_ref()
                .is_some_and(|value| !value.is_object())
            {
                return Err(ByokError::Config(format!(
                    "{path}.model_overrides.{id}.catalog must be an object"
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Config;

    #[test]
    fn test_provider_config_default() {
        let pc = ProviderConfig::default();
        assert!(pc.enabled);
        assert!(pc.api_key.is_none());
    }

    #[test]
    fn test_from_yaml_provider_api_key() {
        let yaml = r#"
providers:
  claude:
    api_key: "sk-ant-test"
    enabled: true
"#;
        let c = Config::from_yaml(yaml).unwrap();
        let claude = c.providers.get("claude").unwrap();
        assert_eq!(
            claude.api_key.as_ref().unwrap().resolve().unwrap(),
            "sk-ant-test"
        );
        assert!(claude.enabled);
    }

    #[test]
    fn test_from_yaml_provider_disabled() {
        let yaml = r"
providers:
  cursor:
    enabled: false
";
        let c = Config::from_yaml(yaml).unwrap();
        let cursor = c.providers.get("cursor").unwrap();
        assert!(!cursor.enabled);
        assert!(cursor.api_key.is_none());
    }

    #[test]
    fn custom_providers_require_a_base_url() {
        let yaml = r"
providers:
  codex:
    enabled: false
";
        assert!(Config::from_yaml(yaml).is_err());
    }

    #[test]
    fn a_messages_only_provider_can_be_routed_without_becoming_a_responses_provider() {
        let config = Config::from_yaml(
            r"
providers:
  llm-router:
    anthropic:
      base_url: https://router.example/api
      headers:
        x-request-resource-group: '5'
anthropic:
  routes:
    default: copilot
    families: { sonnet: llm-router }
",
        )
        .unwrap();

        assert_eq!(
            config.anthropic.routes.resolve_id("claude-sonnet-4-6").0,
            "llm-router"
        );
        assert_eq!(
            config.anthropic.routes.resolve_id("claude-opus-5-5").0,
            "copilot"
        );
        assert!(
            config
                .response_route("llm-router/claude-sonnet-4-6")
                .is_err()
        );
        assert!(
            config.providers["llm-router"]
                .anthropic
                .as_ref()
                .unwrap()
                .api_key
                .is_none()
        );
    }

    #[test]
    fn messages_connections_do_not_make_invalid_responses_settings_valid() {
        let error = Config::from_yaml(
            r"
providers:
  llm-router:
    models_url: https://router.example/models
    anthropic:
      base_url: https://router.example/api
",
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("providers.llm-router.base_url is required"),
            "{error}"
        );
    }

    #[test]
    fn builtins_cannot_replace_their_authentication_with_a_custom_messages_connection() {
        let error = Config::from_yaml(
            "providers: { claude: { anthropic: { base_url: https://router.example } } }",
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("requires a custom provider"),
            "{error}"
        );
    }

    #[test]
    fn messages_provider_names_cannot_conflict_with_model_suffixes() {
        for name in ["1m", "router[team", "router]team"] {
            let mut config = Config::default();
            config.providers.insert(
                name.into(),
                ProviderConfig {
                    anthropic: Some(AnthropicProviderConfig {
                        base_url: "https://router.example/api".into(),
                        models_url: None,
                        enabled_models: None,
                        api_key: None,
                        headers: BTreeMap::new(),
                    }),
                    ..ProviderConfig::default()
                },
            );
            assert!(
                config
                    .validate()
                    .unwrap_err()
                    .to_string()
                    .contains("Messages provider names")
            );
        }
    }

    #[test]
    fn credentials_use_explicit_references_without_falling_back_to_literals() {
        let config = Config::from_yaml(
            r"
providers:
  claude:
    api_key: { env: PATH }
  cursor:
    api_key: { env: BYOKEY_TEST_MISSING_KEY_9FC65 }
  copilot:
    api_key: PATH
",
        )
        .unwrap();
        assert_eq!(
            config.providers["claude"]
                .api_key
                .as_ref()
                .unwrap()
                .resolve()
                .unwrap(),
            std::env::var("PATH").unwrap()
        );
        assert!(
            config.providers["cursor"]
                .api_key
                .as_ref()
                .unwrap()
                .resolve()
                .unwrap_err()
                .to_string()
                .contains("BYOKEY_TEST_MISSING_KEY_9FC65")
        );
        assert_eq!(
            config.providers["copilot"]
                .api_key
                .as_ref()
                .unwrap()
                .resolve()
                .unwrap(),
            "PATH"
        );
    }

    #[test]
    fn chatgpt_auth_cannot_come_from_server_configuration() {
        for settings in [
            "api_key: server-key",
            "headers: { Authorization: server-key }",
        ] {
            assert!(Config::from_yaml(&format!("providers:\n  chatgpt:\n    {settings}")).is_err());
        }
    }
}
