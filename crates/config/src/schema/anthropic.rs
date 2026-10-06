//! Anthropic routing and model catalog presentation.

use byokey_types::Result;
use serde::{Deserialize, Serialize};

/// Anthropic client settings, independent of Responses settings and routing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AnthropicConfig {
    /// Provider selection by model, family, then default.
    pub routes: super::Routes,
    /// Display names and context variants for `/v1/models` and Claude Desktop.
    pub catalog: AnthropicCatalog,
}

/// Model picker names and context variants without changing standard Anthropic model IDs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AnthropicCatalog {
    /// Plain-text `MiniJinja` template with `model` and `provider` variables.
    pub name_format: String,
    /// Hide the additional 1M picker entry for native 1M models. Defaults to true.
    pub merge_native_1m: bool,
}

impl Default for AnthropicCatalog {
    fn default() -> Self {
        Self {
            name_format: "{{ model }} · {{ provider }}".into(),
            merge_native_1m: true,
        }
    }
}

impl AnthropicCatalog {
    /// Compile the name template once for a catalog response.
    ///
    /// # Errors
    /// Rejects invalid syntax and unknown variables. The formatter rejects render errors and empty names.
    pub fn name_formatter(&self) -> Result<impl Fn(&str, &str) -> Result<String> + '_> {
        super::catalog::name_formatter(&self.name_format, "anthropic.catalog.name_format")
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
    fn anthropic_and_responses_templates_are_independent() {
        let config = Config::from_yaml(
            r"
anthropic:
  catalog:
    name_format: '{{ provider | upper }} / {{ model }}'
responses:
  catalog:
    name_format: '{{ model }} [{{ provider }}]'
",
        )
        .unwrap();

        let anthropic =
            config.anthropic.catalog.name_formatter().unwrap()("Opus <{{ model }}>", "r&d")
                .unwrap();
        let responses =
            config.responses.catalog.name_formatter().unwrap()("Astra", "Native").unwrap();

        assert_eq!(anthropic, "R&D / Opus <{{ model }}>");
        assert_eq!(responses, "Astra [Native]");
    }

    #[test]
    fn unknown_anthropic_template_variables_are_rejected_on_load() {
        let error = Config::from_yaml(
            "anthropic:\n  catalog:\n    name_format: '{% if false %}{{ provder }}{% endif %}{{ model }}'",
        )
        .unwrap_err();

        assert!(
            error.to_string().contains("anthropic.catalog.name_format"),
            "{error}"
        );
        assert!(
            error.to_string().contains("unknown variable provder"),
            "{error}"
        );
    }

    #[test]
    fn empty_anthropic_provider_names_are_rejected() {
        let error = Config::from_yaml("providers:\n  copilot:\n    display_name: ' '").unwrap_err();

        assert!(error.to_string().contains("providers.copilot"), "{error}");
        assert!(error.to_string().contains("nonempty"), "{error}");
    }

    #[test]
    fn empty_anthropic_model_names_are_rejected() {
        let error = Config::from_yaml(
            "providers:\n  copilot:\n    model_overrides:\n      claude-opus-5-5: { name: '' }",
        )
        .unwrap_err();

        assert!(error.to_string().contains("providers.copilot"), "{error}");
        assert!(error.to_string().contains("nonempty"), "{error}");
    }

    #[test]
    fn responses_providers_cannot_serve_anthropic_routes() {
        let error = Config::from_yaml("anthropic:\n  routes:\n    default: chatgpt").unwrap_err();

        assert!(error.to_string().contains("chatgpt"), "{error}");
        assert!(error.to_string().contains("unknown variant"), "{error}");
    }
}
