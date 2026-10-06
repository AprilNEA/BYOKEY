//! Plain-text name templates shared by model catalogs.

use byokey_types::{ByokError, Result};
use minijinja::{Environment, UndefinedBehavior, context};
use std::sync::LazyLock;

static NAME_ENVIRONMENT: LazyLock<Environment<'static>> = LazyLock::new(|| {
    let mut environment = Environment::new();
    environment.set_undefined_behavior(UndefinedBehavior::Strict);
    environment
});

/// Compile once per catalog response and identify errors by configuration path.
pub(super) fn name_formatter<'a>(
    format: &'a str,
    path: &'static str,
) -> Result<impl Fn(&str, &str) -> Result<String> + 'a> {
    let template_error = move |error| ByokError::Config(format!("{path}: {error}"));
    let template = NAME_ENVIRONMENT
        .template_from_str(format)
        .map_err(template_error)?;
    for variable in template.undeclared_variables(false) {
        if !matches!(variable.as_str(), "model" | "provider") {
            return Err(ByokError::Config(format!(
                "{path}: unknown variable {variable}; use model or provider"
            )));
        }
    }
    Ok(move |model: &str, provider: &str| {
        let name = template
            .render(context! { model, provider })
            .map_err(template_error)?;
        if name.trim().is_empty() {
            return Err(ByokError::Config(format!(
                "{path} must render a nonempty name"
            )));
        }
        Ok(name)
    })
}
