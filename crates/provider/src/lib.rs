//! Provider executor implementations and model registry.
//!
//! ## Module layout
//!
//! - [`executor`]  — Per-provider [`ProviderExecutor`] implementations.
//! - [`factory`]   — Executor creation from provider/model identifiers + config.
//! - [`registry`]  — Model-to-provider mapping and model listing.
//! - [`http_util`] — Shared HTTP send/stream helpers ([`ProviderHttp`]).
//! - [`routing`]   — Round-robin API key selection ([`CredentialRouter`]).
//! - [`retry`]     — Multi-key retry wrapper ([`RetryExecutor`]).

pub mod cloak;
pub mod device_profile;
pub mod executor;
pub mod factory;
pub mod http_util;
pub mod registry;
pub mod retry;
pub mod routing;
pub mod selector;
pub mod stream_bridge;
pub mod thinking;
pub mod versions;

pub use device_profile::DeviceProfileCache;
pub use executor::{
    AntigravityExecutor, ClaudeExecutor, CodexExecutor, CodexWsExecutor, CopilotExecutor,
    GeminiExecutor, IFlowExecutor, KimiExecutor, KiroExecutor, QwenExecutor,
};
pub use factory::{make_executor, make_executor_for_model, make_executor_with_cache};
pub use http_util::ProviderHttp;
pub use registry::{
    ModelEntry, ThinkingSupport, all_models, is_copilot_free_model, models_for_provider,
    parse_qualified_model, resolve_provider, resolve_provider_with, thinking_capability,
    thinking_support,
};
pub use routing::{CredentialRouter, RoutingStrategy};
pub use selector::{AccountNode, AccountSelector, RoutingPolicy, StrategyKind};
pub use thinking::{ModelSuffix, parse_model_suffix};
pub use versions::VersionStore;

/// Claude fingerprint constants shared with the proxy crate's `/v1/messages` handler.
pub mod claude_headers {
    pub use crate::executor::claude::{
        ANTHROPIC_BETA, ANTHROPIC_VERSION, RUNTIME_VERSION, SDK_PACKAGE_VERSION, USER_AGENT,
    };
}
