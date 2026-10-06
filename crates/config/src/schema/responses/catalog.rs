//! Presentation settings for the Codex model catalog.

use byokey_types::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Model picker names and visibility, independent of inference routing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResponsesCatalog {
    /// Plain-text `MiniJinja` template with `model` and `provider` variables.
    pub name_format: String,
    /// Exact catalog slugs to hide without removing their metadata or routing.
    pub hidden_aliases: BTreeSet<String>,
}

impl Default for ResponsesCatalog {
    fn default() -> Self {
        Self {
            name_format: "{{ model }} ({{ provider }})".into(),
            hidden_aliases: BTreeSet::new(),
        }
    }
}

impl ResponsesCatalog {
    /// Compile the name template once for a catalog response.
    ///
    /// # Errors
    /// Rejects invalid syntax and unknown variables. The formatter rejects render errors and empty names.
    pub fn name_formatter(&self) -> Result<impl Fn(&str, &str) -> Result<String> + '_> {
        crate::schema::catalog::name_formatter(&self.name_format, "responses.catalog.name_format")
    }

    pub(super) fn validate(&self) -> Result<()> {
        self.name_formatter()?("model", "provider")?;
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
providers:
  copilot:
    display_name: GitHub
",
        )
        .unwrap();
        let catalog = &config.responses.catalog;

        let name = catalog.name_formatter().unwrap()("GPT-6-Astra <{{ model }}>", "r&d").unwrap();

        assert_eq!(name, "R&D — GPT-6-Astra <{{ model }}>");
        assert_eq!(config.provider_name("chatgpt"), "ChatGPT");
        assert_eq!(config.provider_name("copilot"), "GitHub");
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
