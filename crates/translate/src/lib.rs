//! Request and response translators between LLM API formats.
//!
//! This crate provides bidirectional translation between `OpenAI`, Claude, Gemini,
//! and Codex (`OpenAI` Responses API) message formats. All translators are pure functions
//! with no I/O.

pub mod cache_control;
pub mod claude_to_openai;
pub mod codex_to_openai;
pub mod gemini_to_openai;
pub mod merge_messages;
pub mod openai_to_claude;
pub mod openai_to_codex;
pub mod openai_to_gemini;
pub mod thinking;

pub use cache_control::inject_cache_control;
pub use claude_to_openai::ClaudeToOpenAI;
pub use codex_to_openai::CodexToOpenAI;
pub use gemini_to_openai::GeminiToOpenAI;
pub use merge_messages::merge_adjacent_messages;
pub use openai_to_claude::OpenAIToClaude;
pub use openai_to_codex::OpenAIToCodex;
pub use openai_to_gemini::OpenAIToGemini;
pub use thinking::ThinkingExtractor;
