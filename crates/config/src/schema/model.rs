use serde::{Deserialize, Serialize};

/// A single model alias mapping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelAlias {
    /// Original model name to map from.
    pub name: String,
    /// Alias name to expose.
    pub alias: String,
    /// If true, expose both the original name and the alias.
    #[serde(default)]
    pub fork: bool,
}
