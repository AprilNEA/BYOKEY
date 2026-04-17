//! Configuration loading and hot-reloading for the byokey proxy.
//!
//! Uses figment for YAML-based configuration with sensible defaults,
//! and notify + arc-swap for live file watching.

pub mod schema;
pub mod watcher;

pub use schema::{
    AmpConfig, ApiKeyEntry, ClaudeHeaderDefaults, CloakConfig, CodexHeaderDefaults, Config,
    KeyRoutingStrategy, LogConfig, LogFormat, ModelAlias, PayloadFilterRule, PayloadRule,
    PayloadRules, PolicyStrategyKind, ProviderConfig, RoutingPolicyEntry, StreamingConfig,
};
pub use watcher::ConfigWatcher;
