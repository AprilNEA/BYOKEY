//! Configuration loading and hot-reloading for the byokey proxy.
//!
//! Uses figment for YAML/JSON configuration with sensible defaults,
//! and notify + arc-swap for live file watching.

pub mod schema;
pub mod watcher;

pub use schema::{
    AnthropicProviderConfig, ClaudeCodeConfig, ClaudeDesktopConfig, Config, ConfigValue, LogConfig,
    LogFormat, ModelOverride, ProviderConfig, RouteSource, Routes, TelemetryConfig,
};
pub use watcher::ConfigWatcher;
