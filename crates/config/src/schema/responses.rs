//! Routing for the Responses API, independent of Anthropic model names.

use byokey_types::{ByokError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

fn chatgpt() -> String {
    "chatgpt".into()
}

fn chatgpt_base_url() -> String {
    "https://chatgpt.com/backend-api/codex".into()
}

/// Responses routing and named upstreams. `ChatGPT` credentials come from the client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResponsesConfig {
    /// Upstream for unqualified models: `chatgpt`, `copilot`, or a configured name.
    pub default: String,
    /// Trusted `ChatGPT` backend root, including its API path.
    pub chatgpt_base_url: String,
    /// Arbitrary client-facing model names and their upstream targets.
    pub models: BTreeMap<String, ResponseModel>,
    /// Custom Responses-compatible upstreams. Built-in names are reserved.
    pub upstreams: BTreeMap<String, ResponsesUpstream>,
}

impl Default for ResponsesConfig {
    fn default() -> Self {
        Self {
            default: chatgpt(),
            chatgpt_base_url: chatgpt_base_url(),
            models: BTreeMap::new(),
            upstreams: BTreeMap::new(),
        }
    }
}

/// An alias and the metadata used for the ChatGPT.app model picker.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponseModel {
    /// Upstream name.
    pub upstream: String,
    /// Model identifier sent to the upstream.
    pub model: String,
    /// `ChatGPT` catalog model whose metadata matches this model; defaults to `model`.
    #[serde(default)]
    pub catalog_model: Option<String>,
    /// Full Codex model metadata, instead of borrowing `ChatGPT` catalog metadata.
    #[serde(default)]
    pub catalog: Option<Value>,
}

/// A Responses endpoint with credentials independent of the client's `ChatGPT` login.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResponsesUpstream {
    /// API root; `/responses` is appended. May include `/v1` or a gateway path.
    pub base_url: String,
    /// OpenAI-compatible model list URL. When set, discover models with matching Codex metadata.
    #[serde(default)]
    pub models_url: Option<String>,
    /// Provider label in discovered model names; defaults to the upstream name.
    #[serde(default)]
    pub display_name: Option<String>,
    /// Bearer token, either literal or read from an environment variable.
    #[serde(default)]
    pub api_key: Option<ConfigValue>,
    /// Additional headers; a configured Authorization header overrides `api_key`.
    #[serde(default)]
    pub headers: BTreeMap<String, ConfigValue>,
    /// Override the client's service tier for this upstream; absent preserves the request.
    #[serde(default)]
    pub service_tier: Option<String>,
}

/// A literal, an environment-variable reference, or a per-request generated value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ConfigValue {
    /// A literal value.
    Literal(String),
    /// Read the named environment variable from the server process.
    Environment {
        /// Name of the environment variable.
        env: String,
    },
    /// Generate a new UUID v4 each time the value is resolved.
    Uuid {
        /// Prefix placed before the lowercase, hyphenated UUID.
        uuid_prefix: String,
    },
}

impl ConfigValue {
    /// Resolve the value without exposing credentials in error messages.
    ///
    /// # Errors
    /// Returns a configuration error if the environment variable is unavailable.
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

impl ResponsesConfig {
    /// Resolve an exact alias before a qualified model, then apply the default route.
    ///
    /// # Errors
    /// Returns an error for an unknown upstream or an empty model identifier.
    pub fn route<'a>(&'a self, requested: &'a str) -> Result<(&'a str, &'a str)> {
        let (upstream, model) = if let Some(route) = self.models.get(requested) {
            (route.upstream.as_str(), route.model.as_str())
        } else if let Some((upstream, model)) = requested.split_once('/') {
            (upstream, model)
        } else {
            (self.default.as_str(), requested)
        };
        if model.is_empty() {
            return Err(ByokError::InvalidRequest("model must not be empty".into()));
        }
        if !matches!(upstream, "chatgpt" | "copilot") && !self.upstreams.contains_key(upstream) {
            return Err(ByokError::UnsupportedProvider(upstream.into()));
        }
        Ok((upstream, model))
    }

    pub(super) fn validate(&self) -> Result<()> {
        for name in self.upstreams.keys() {
            if name.is_empty()
                || name.contains('/')
                || matches!(name.as_str(), "chatgpt" | "copilot")
            {
                return Err(ByokError::Config(format!(
                    "invalid or reserved Responses upstream name: {name}"
                )));
            }
        }
        if !matches!(self.default.as_str(), "chatgpt" | "copilot")
            && !self.upstreams.contains_key(&self.default)
        {
            return Err(ByokError::UnsupportedProvider(self.default.clone()));
        }
        for (alias, route) in &self.models {
            if alias.is_empty() || route.catalog.as_ref().is_some_and(|v| !v.is_object()) {
                return Err(ByokError::Config(
                    "Responses aliases must be nonempty and catalog metadata must be an object"
                        .into(),
                ));
            }
            self.route(alias)?;
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
responses:
  default: copilot
  models:
    team/fast:
      upstream: company
      model: vendor/gpt-fast
  upstreams:
    company:
      base_url: https://example.com/v1
      api_key: { env: BYOKEY_TEST_MISSING_KEY_9FC65 }
      headers:
        X-Tenant: engineering
",
        )
        .unwrap();
        assert_eq!(
            config.responses.route("team/fast").unwrap(),
            ("company", "vendor/gpt-fast")
        );
        assert_eq!(
            config.responses.route("chatgpt/gpt-x").unwrap(),
            ("chatgpt", "gpt-x")
        );
        assert_eq!(
            config.responses.route("gpt-y").unwrap(),
            ("copilot", "gpt-y")
        );
        assert!(config.responses.route("copilot/").is_err());
        assert!(config.responses.route("typo/model").is_err());
        assert!(
            config.responses.upstreams["company"]
                .api_key
                .as_ref()
                .unwrap()
                .resolve()
                .is_err()
        );
        assert_eq!(
            config.responses.upstreams["company"].headers["X-Tenant"]
                .resolve()
                .unwrap(),
            "engineering"
        );
    }

    #[test]
    fn reserved_and_unknown_upstreams_are_rejected() {
        assert!(Config::from_yaml("responses:\n  default: typo").is_err());
        assert!(
            Config::from_yaml(
                "responses:\n  upstreams:\n    chatgpt:\n      base_url: https://example.com"
            )
            .is_err()
        );
    }
}
