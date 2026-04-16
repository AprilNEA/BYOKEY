//! Request and response translators between LLM API formats.
//!
//! This crate provides bidirectional translation between `OpenAI`, Claude, and Gemini
//! message formats. All translators are pure functions with no I/O.
//!
//! Codex (`OpenAI` Responses API) translation is delegated to the `aigw-openai` crate
//! via the BYOKEY provider executor.

pub mod cache_control;
pub mod claude_to_openai;
pub mod gemini_native_to_openai;
pub mod gemini_to_openai;
pub mod merge_messages;
pub mod openai_to_claude;
pub mod openai_to_gemini;
pub mod openai_to_gemini_native;
pub mod thinking;

pub use cache_control::inject_cache_control;
pub use claude_to_openai::ClaudeToOpenAI;
pub use gemini_native_to_openai::GeminiNativeRequest;
pub use gemini_to_openai::GeminiToOpenAI;
pub use merge_messages::merge_adjacent_messages;
pub use openai_to_claude::OpenAIToClaude;
pub use openai_to_gemini::OpenAIToGemini;
pub use openai_to_gemini_native::{OpenAIResponseToGemini, OpenAISseChunk};
pub use thinking::ThinkingExtractor;
pub use thinking::{
    DEFAULT_AUTO_BUDGET, ModelSuffix, ThinkingConfig, ThinkingLevel, apply_thinking,
    has_valid_claude_signature, parse_model_suffix, strip_invalid_thinking_signatures,
};
