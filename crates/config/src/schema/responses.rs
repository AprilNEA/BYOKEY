//! Responses routing, independent of provider connections and model metadata.

pub mod catalog;

use super::Config;
use byokey_types::{ByokError, Result};
use catalog::ResponsesCatalog;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Responses routing and presentation for ChatGPT.app and Codex.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResponsesConfig {
    /// Provider selection for Responses requests.
    pub routes: ResponsesRoutes,
    /// Model picker presentation, independent of routing and capabilities.
    pub catalog: ResponsesCatalog,
}

/// Exact aliases take precedence over provider prefixes, then the default.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResponsesRoutes {
    /// Provider for unqualified models.
    pub default: String,
    /// Client-facing model aliases and their provider targets.
    pub models: BTreeMap<String, ResponseModel>,
}

impl Default for ResponsesRoutes {
    fn default() -> Self {
        Self {
            default: "chatgpt".into(),
            models: BTreeMap::new(),
        }
    }
}

/// A route identifies a provider and the model ID sent to that provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponseModel {
    /// Built-in or configured provider name.
    pub provider: String,
    /// Model identifier sent to the provider.
    pub model: String,
}

impl Config {
    /// Resolve a Responses alias, qualified model, or default provider.
    ///
    /// # Errors
    /// Rejects empty models and unknown, disabled, or incompatible providers.
    pub fn response_route<'a>(&'a self, requested: &'a str) -> Result<(&'a str, &'a str)> {
        let routes = &self.responses.routes;
        let (provider, model) = if let Some(route) = routes.models.get(requested) {
            (route.provider.as_str(), route.model.as_str())
        } else if let Some((provider, model)) = requested.split_once('/') {
            (provider, model)
        } else {
            (routes.default.as_str(), requested)
        };
        if model.is_empty() {
            return Err(ByokError::InvalidRequest("model must not be empty".into()));
        }
        self.validate_responses_provider(provider)?;
        if self.providers.get(provider).is_some_and(|p| !p.enabled) {
            return Err(ByokError::UnsupportedProvider(format!(
                "{provider} is disabled"
            )));
        }
        Ok((provider, model))
    }

    fn validate_responses_provider(&self, name: &str) -> Result<()> {
        if matches!(name, "claude" | "cursor")
            || (!matches!(name, "chatgpt" | "copilot")
                && self.providers.get(name).is_none_or(|p| {
                    p.base_url
                        .as_deref()
                        .is_none_or(|url| url.trim().is_empty())
                }))
        {
            return Err(ByokError::UnsupportedProvider(name.into()));
        }
        Ok(())
    }

    pub(super) fn validate_responses(&self) -> Result<()> {
        self.responses.catalog.validate()?;
        self.validate_responses_provider(&self.responses.routes.default)?;
        for (alias, route) in &self.responses.routes.models {
            if alias.is_empty() || route.model.is_empty() {
                return Err(ByokError::Config(
                    "Responses aliases and model IDs must be nonempty".into(),
                ));
            }
            self.validate_responses_provider(&route.provider)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::Config;

    #[test]
    fn arbitrary_aliases_precede_prefixes_and_defaults() {
        let config = Config::from_yaml(
            r"
providers:
  company:
    base_url: https://example.com/v1
    api_key: { env: BYOKEY_TEST_MISSING_KEY_9FC65 }
    headers:
      X-Tenant: engineering
responses:
  routes:
    default: copilot
    models:
      team/fast:
        provider: company
        model: vendor/gpt-fast
",
        )
        .unwrap();
        assert_eq!(
            config.response_route("team/fast").unwrap(),
            ("company", "vendor/gpt-fast")
        );
        assert_eq!(
            config.response_route("chatgpt/gpt-x").unwrap(),
            ("chatgpt", "gpt-x")
        );
        assert_eq!(
            config.response_route("gpt-y").unwrap(),
            ("copilot", "gpt-y")
        );
        assert!(config.response_route("copilot/").is_err());
        assert!(config.response_route("typo/model").is_err());
        assert!(
            config.providers["company"]
                .api_key
                .as_ref()
                .unwrap()
                .resolve()
                .is_err()
        );
        assert_eq!(
            config.providers["company"].headers["X-Tenant"]
                .resolve()
                .unwrap(),
            "engineering"
        );
    }

    #[test]
    fn unknown_or_incompatible_providers_are_rejected() {
        assert!(Config::from_yaml("responses:\n  routes:\n    default: typo").is_err());
        assert!(Config::from_yaml("responses:\n  routes:\n    default: claude").is_err());
        assert!(Config::default().response_route("cursor/model").is_err());
    }

    #[test]
    fn disabling_a_provider_blocks_aliases_and_prefixes_without_invalidating_config() {
        let config = Config::from_yaml(
            r"
providers:
  company:
    base_url: https://example.com/v1
    enabled: false
responses:
  routes:
    default: company
    models:
      fast: { provider: company, model: actual }
",
        )
        .unwrap();
        assert!(
            config
                .response_route("fast")
                .unwrap_err()
                .to_string()
                .contains("company is disabled")
        );
        assert!(config.response_route("company/actual").is_err());
        assert!(config.response_route("actual").is_err());
    }
}
