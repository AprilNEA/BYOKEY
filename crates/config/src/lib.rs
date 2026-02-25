//! Configuration loading and hot-reloading for the byokey proxy.
//!
//! Uses figment for YAML-based configuration with sensible defaults,
//! and notify + arc-swap for live file watching.

pub mod schema;
pub mod watcher;

pub use schema::{
    AmpConfig, ApiKeyEntry, Config, LogConfig, ModelAlias, PayloadFilterRule, PayloadRule,
    PayloadRules, ProviderConfig, StreamingConfig, TlsConfig,
};
pub use watcher::ConfigWatcher;
