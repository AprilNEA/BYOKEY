use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Claude Code integration, applied by `byokey claude-code inject`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClaudeCodeConfig {
    /// Extra Claude Code settings merged into its `settings.json`. `env` is
    /// merged variable by variable; every other key replaces the existing one.
    #[serde(default)]
    pub settings: Map<String, Value>,
}
