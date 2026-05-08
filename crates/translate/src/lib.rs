//! Request and response translators between LLM API formats.
//!
//! This crate provides bidirectional translation between `OpenAI`, Claude, and Gemini
//! message formats. All translators are pure functions with no I/O.
//!
//! Anthropic translation (request/response/stream/cache_control) and the canonical
//! thinking projector are delegated to the `aigw-anthropic` and `aigw-core` crates;
//! the leftovers in this crate are the pieces that remain BYOKEY-specific —
//! Gemini-native bidirectional bridges for the AmpCode handler, message merging,
//! per-provider thinking JSON injection for executors that don't go through aigw,
//! and the `model(...)` suffix UX convention.

pub mod gemini_native_to_openai;
pub mod merge_messages;
pub mod openai_to_gemini_native;
pub mod thinking;

pub use gemini_native_to_openai::GeminiNativeRequest;
pub use merge_messages::merge_adjacent_messages;
pub use openai_to_gemini_native::{OpenAIResponseToGemini, OpenAISseChunk};
pub use thinking::ThinkingExtractor;
pub use thinking::{
    DEFAULT_AUTO_BUDGET, ModelSuffix, ThinkingConfig, ThinkingLevel, apply_thinking,
    has_valid_claude_signature, parse_model_suffix, strip_invalid_thinking_signatures,
};
