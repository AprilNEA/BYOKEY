//! Presentation settings for the Codex model catalog.

use byokey_types::{ByokError, Result};
use minijinja::{Environment, UndefinedBehavior, context};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::LazyLock,
};

static NAME_ENVIRONMENT: LazyLock<Environment<'static>> = LazyLock::new(|| {
    let mut environment = Environment::new();
    environment.set_undefined_behavior(UndefinedBehavior::Strict);
    environment
});

/// Model picker names and visibility, independent of inference routing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResponsesCatalog {
    /// Plain-text `MiniJinja` template with `model` and `provider` variables.
    pub name_format: String,
    /// Display names for the built-in providers.
    pub provider_names: ProviderNames,
    /// Display name overrides keyed by the actual upstream model ID, not the alias.
    pub model_names: BTreeMap<String, String>,
    /// Exact catalog slugs to hide without removing their metadata or routing.
    pub hidden_aliases: BTreeSet<String>,
}

impl Default for ResponsesCatalog {
    fn default() -> Self {
        Self {
            name_format: "{{ model }} ({{ provider }})".into(),
            provider_names: ProviderNames::default(),
            model_names: BTreeMap::new(),
            hidden_aliases: BTreeSet::new(),
        }
    }
}

/// Built-in provider labels. Custom upstreams use their `display_name` instead.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderNames {
    /// Label for client-owned `ChatGPT` subscriptions.
    pub chatgpt: String,
    /// Label for GitHub Copilot models.
    pub copilot: String,
}

impl Default for ProviderNames {
    fn default() -> Self {
        Self {
            chatgpt: "ChatGPT".into(),
            copilot: "Copilot".into(),
        }
    }
}

impl ResponsesCatalog {
    /// Compile the name template once for a catalog response.
    ///
    /// # Errors
    /// Rejects invalid syntax and unknown variables. The formatter rejects render errors and empty names.
    pub fn name_formatter(&self) -> Result<impl Fn(&str, &str) -> Result<String> + '_> {
        let template_error =
            |error| ByokError::Config(format!("responses.catalog.name_format: {error}"));
        let template = NAME_ENVIRONMENT
            .template_from_str(&self.name_format)
            .map_err(template_error)?;
        for variable in template.undeclared_variables(false) {
            if !matches!(variable.as_str(), "model" | "provider") {
                return Err(ByokError::Config(format!(
                    "responses.catalog.name_format: unknown variable {variable}; use model or provider"
                )));
            }
        }
        Ok(move |model: &str, provider: &str| {
            let name = template
                .render(context! { model, provider })
                .map_err(template_error)?;
            if name.trim().is_empty() {
                return Err(ByokError::Config(
                    "responses.catalog.name_format must render a nonempty name".into(),
                ));
            }
            Ok(name)
        })
    }

    pub(super) fn validate(&self) -> Result<()> {
        self.name_formatter()?("model", "provider")?;
        if self.provider_names.chatgpt.trim().is_empty()
            || self.provider_names.copilot.trim().is_empty()
            || self
                .model_names
                .iter()
                .any(|(id, name)| id.trim().is_empty() || name.trim().is_empty())
        {
            return Err(ByokError::Config(
                "responses.catalog provider names, model IDs and model names must be nonempty"
                    .into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::Config;

    #[test]
    fn templates_render_plain_text_and_do_not_reinterpret_model_names() {
        let config = Config::from_yaml(
            r"
responses:
  catalog:
    name_format: '{{ provider | upper }} — {{ model }}'
    provider_names:
      copilot: GitHub
",
        )
        .unwrap();
        let catalog = &config.responses.catalog;

        let name = catalog.name_formatter().unwrap()("GPT-6-Astra <{{ model }}>", "r&d").unwrap();

        assert_eq!(name, "R&D — GPT-6-Astra <{{ model }}>");
        assert_eq!(catalog.provider_names.chatgpt, "ChatGPT");
        assert_eq!(catalog.provider_names.copilot, "GitHub");
    }

    #[test]
    fn invalid_template_syntax_is_rejected_on_load() {
        let error =
            Config::from_yaml("responses:\n  catalog:\n    name_format: '{{ model'").unwrap_err();

        assert!(
            error.to_string().contains("responses.catalog.name_format"),
            "{error}"
        );
        assert!(error.to_string().contains("syntax error"), "{error}");
    }

    #[test]
    fn unknown_variables_are_rejected_even_in_an_unused_branch() {
        let error = Config::from_yaml(
            r"responses:
  catalog:
    name_format: '{% if false %}{{ provder }}{% endif %}{{ model }}'
",
        )
        .unwrap_err();

        assert!(
            error.to_string().contains("unknown variable provder"),
            "{error}"
        );
    }

    #[test]
    fn invalid_filters_are_rejected_on_load() {
        let error = Config::from_yaml(
            "responses:\n  catalog:\n    name_format: '{{ model | nonexistent }}'",
        )
        .unwrap_err();

        assert!(error.to_string().contains("unknown filter"), "{error}");
    }

    #[test]
    fn empty_rendered_names_are_rejected() {
        let error = Config::from_yaml(
            "responses:\n  catalog:\n    name_format: '{% if false %}{{ model }}{% endif %}'",
        )
        .unwrap_err();

        assert!(error.to_string().contains("nonempty name"), "{error}");
    }

    #[test]
    fn custom_provider_labels_belong_to_the_upstream() {
        let error = Config::from_yaml(
            "responses:\n  catalog:\n    provider_names:\n      company: Company",
        )
        .unwrap_err();

        assert!(error.to_string().contains("unknown field"), "{error}");
    }
}
