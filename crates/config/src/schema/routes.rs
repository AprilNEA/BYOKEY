use byokey_types::{ClaudeFamily, ClaudeModel, ProviderId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Which provider serves each Anthropic model.
///
/// A model goes to the provider set for that model, else the one set for its
/// family, else `default`, and to Anthropic without any. A `copilot/` or
/// `cursor/` prefix on the request's model picks the provider for that
/// request regardless.
///
/// ```yaml
/// routes:
///   default: copilot
///   families:
///     opus: cursor
///   models:
///     claude-opus-5-5: copilot
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Routes {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<ProviderId>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub families: BTreeMap<ClaudeFamily, ProviderId>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<ClaudeModel, ProviderId>,
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
    pub fn provider(&self, model: ClaudeModel) -> ProviderId {
        self.resolve(model).0
    }

    /// The provider serving `model`, and which setting picked it.
    #[must_use]
    pub fn resolve(&self, model: ClaudeModel) -> (ProviderId, RouteSource) {
        if let Some(&p) = self.models.get(&model) {
            (p, RouteSource::Model)
        } else if let Some(&p) = self.families.get(&model.family) {
            (p, RouteSource::Family)
        } else {
            self.fallback()
        }
    }

    /// The provider for a model that names no Claude model, such as
    /// `gpt-5.4`: `default`, since only Copilot and Cursor serve those.
    #[must_use]
    pub fn fallback(&self) -> (ProviderId, RouteSource) {
        match self.default {
            Some(p) => (p, RouteSource::Default),
            None => (ProviderId::Claude, RouteSource::Unset),
        }
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
routes:
  default: copilot
  families:
    opus: cursor
  models:
    claude-opus-5.5: claude
",
        )
        .unwrap();
        let resolve = |id| c.routes.resolve(model(id));
        assert_eq!(
            resolve("claude-opus-5-5"),
            (ProviderId::Claude, RouteSource::Model),
            "any spelling names the model"
        );
        assert_eq!(
            resolve("claude-opus-4-8"),
            (ProviderId::Cursor, RouteSource::Family)
        );
        assert_eq!(
            resolve("claude-sonnet-5"),
            (ProviderId::Copilot, RouteSource::Default)
        );
        assert_eq!(
            Routes::default().resolve(model("claude-sonnet-5")),
            (ProviderId::Claude, RouteSource::Unset)
        );
    }

    #[test]
    fn routes_name_claude_models_families_and_known_providers() {
        for yaml in [
            "routes:\n  models:\n    gpt-5.4: copilot\n",
            "routes:\n  models:\n    claude-opus-5-5: codex\n",
            "routes:\n  families:\n    claude-opus: copilot\n",
        ] {
            assert!(Config::from_yaml(yaml).is_err(), "{yaml}");
        }
    }
}
