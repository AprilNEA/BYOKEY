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

pub mod executor;
pub mod factory;
pub mod http_util;
pub mod registry;
pub mod retry;
pub mod routing;

pub use executor::{
    AntigravityExecutor, ClaudeExecutor, CodexExecutor, CopilotExecutor, GeminiExecutor,
    IFlowExecutor, KimiExecutor, KiroExecutor, QwenExecutor,
};
pub use factory::{make_executor, make_executor_for_model};
pub use http_util::ProviderHttp;
pub use registry::{
    ModelEntry, all_models, is_copilot_free_model, models_for_provider, parse_qualified_model,
    resolve_provider, resolve_provider_with,
};
pub use routing::CredentialRouter;

/// Claude fingerprint constants shared with the proxy crate's `/v1/messages` handler.
pub mod claude_headers {
    pub use crate::executor::claude::{
        ANTHROPIC_BETA, ANTHROPIC_VERSION, RUNTIME_VERSION, SDK_PACKAGE_VERSION, USER_AGENT,
    };
}
