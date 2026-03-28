//! Per-provider executor implementations.
//!
//! Each sub-module implements [`ProviderExecutor`](byokey_types::traits::ProviderExecutor)
//! for a specific AI backend. The [`factory`](crate::factory) module creates
//! boxed executors based on provider or model identifiers.

pub mod antigravity;
pub mod claude;
pub mod codex;
pub mod codex_ws;
pub mod copilot;
pub mod gemini;
pub mod iflow;
pub mod kimi;
pub mod kiro;
pub mod qwen;

pub use antigravity::AntigravityExecutor;
pub use claude::ClaudeExecutor;
pub use codex::CodexExecutor;
pub use codex_ws::CodexWsExecutor;
pub use copilot::CopilotExecutor;
pub use gemini::GeminiExecutor;
pub use iflow::IFlowExecutor;
pub use kimi::KimiExecutor;
pub use kiro::KiroExecutor;
pub use qwen::QwenExecutor;
