use byokey_types::{ByokError, ClaudeFamily, ClaudeModel, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Which provider serves each Anthropic model.
///
/// A model goes to the provider set for that model, else the one set for its
/// family, else `default`, and to Anthropic without any. A provider prefix
/// or `[provider]` suffix on the request's model takes precedence over these routes.
///
/// ```yaml
/// anthropic:
///   routes:
///     default: copilot
///     families:
///       opus: cursor
///     models:
///       claude-opus-5-5: copilot
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Routes {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub families: BTreeMap<ClaudeFamily, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<ClaudeModel, String>,
}

/// Which setting of [`Routes`] picked a model's provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteSource {
    Model,
    Family,
    Default,
    /// No setting applies; Anthropic serves the model.
    Unset,
}

impl Routes {
    /// The provider serving `model`.
    #[must_use]
    pub fn provider(&self, model: ClaudeModel) -> &str {
        self.resolve(model).0
    }

    /// The provider serving `model`, and which setting picked it.
    #[must_use]
    pub fn resolve(&self, model: ClaudeModel) -> (&str, RouteSource) {
        if let Some(p) = self.models.get(&model) {
            (p, RouteSource::Model)
        } else if let Some(p) = self.families.get(&model.family) {
            (p, RouteSource::Family)
        } else {
            self.fallback()
        }
    }

    /// Resolve an upstream model ID, including provider-specific variants.
    #[must_use]
    pub fn resolve_id(&self, model: &str) -> (&str, RouteSource) {
        ClaudeModel::from_id(model).map_or_else(|| self.fallback(), |model| self.resolve(model))
    }

    /// The provider for a model without a model or family route.
    #[must_use]
    pub fn fallback(&self) -> (&str, RouteSource) {
        match self.default.as_deref() {
            Some(p) => (p, RouteSource::Default),
            None => ("claude", RouteSource::Unset),
        }
    }
}

impl super::Config {
    /// Check that a provider can serve Messages, independently of its login state.
    ///
    /// # Errors
    /// Rejects names that are neither Messages built-ins nor configured Messages providers.
    pub fn validate_anthropic_provider(&self, name: &str) -> Result<()> {
        if !matches!(name, "claude" | "copilot" | "cursor")
            && self
                .providers
                .get(name)
                .is_none_or(|p| p.anthropic.is_none())
        {
            return Err(ByokError::UnsupportedProvider(name.into()));
        }
        Ok(())
    }

    pub(super) fn validate_anthropic(&self) -> Result<()> {
        self.anthropic.catalog.validate()?;
        let routes = &self.anthropic.routes;
        for name in routes
            .default
            .iter()
            .chain(routes.families.values())
            .chain(routes.models.values())
        {
            self.validate_anthropic_provider(name)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;

    fn model(id: &str) -> ClaudeModel {
        id.parse().unwrap()
    }

    #[test]
    fn a_model_beats_its_family_which_beats_the_default() {
        let c = Config::from_yaml(
            r"
anthropic:
  routes:
    default: copilot
    families:
      opus: cursor
    models:
      claude-opus-5.5: claude
",
        )
        .unwrap();
        let resolve = |id| c.anthropic.routes.resolve(model(id));
        assert_eq!(
            resolve("claude-opus-5-5"),
            ("claude", RouteSource::Model),
            "any spelling names the model"
        );
        assert_eq!(resolve("claude-opus-4-8"), ("cursor", RouteSource::Family));
        assert_eq!(
            resolve("claude-sonnet-5"),
            ("copilot", RouteSource::Default)
        );
        assert_eq!(
            Routes::default().resolve(model("claude-sonnet-5")),
            ("claude", RouteSource::Unset)
        );
    }

    #[test]
    fn routes_name_claude_models_families_and_known_providers() {
        for yaml in [
            "anthropic:\n  routes:\n    models:\n      gpt-5.4: copilot\n",
            "anthropic:\n  routes:\n    models:\n      claude-opus-5-5: codex\n",
            "anthropic:\n  routes:\n    families:\n      claude-opus: copilot\n",
        ] {
            assert!(Config::from_yaml(yaml).is_err(), "{yaml}");
        }
    }
}
